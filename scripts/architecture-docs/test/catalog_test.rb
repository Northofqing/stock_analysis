#!/usr/bin/env ruby
# frozen_string_literal: true

require 'digest'
require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require_relative '../catalog'

CHECK_CATALOG = File.expand_path('../check-catalog.rb', __dir__)
RENDER_CATALOG = File.expand_path('../render-catalog.rb', __dir__)

class CatalogTest < Minitest::Test
  def test_draft_accepts_a_valid_isolated_git_fixture
    with_fixture do |root|
      out, err, result = cli(root)
      assert result.success?, out + err
      assert_includes out, 'push_catalog_valid'
    end
  end

  def test_draft_rejects_enum_addition_removal_and_duplicates
    ["    One,\n    Two,\n", '', "    One,\n    One,\n"].each do |variants|
      with_fixture do |root|
        File.binwrite(File.join(root, 'src/notify.rs'), "pub enum PushKind {\n#{variants}}\n")
        out, err, result = cli(root)
        refute result.success?
        assert_includes out + err, 'enum_coverage_mismatch'
      end
    end
  end

  def test_draft_rejects_comment_bytes_and_preserves_independent_diagnostics
    with_fixture do |root|
      File.binwrite(File.join(root, 'src/notify.rs'), "pub enum PushKind { // 注释也属于证据\n    One,\n}\n")
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'file_sha_mismatch'
      assert_includes out + err, 'symbol_sha_mismatch'
      refute_includes out + err, 'enum_coverage_mismatch'
    end
  end

  def test_schema_and_reference_errors_are_actionable
    cases = [
      ['push_schema_invalid', proc { |c| c['schema_version'] = 2 }],
      ['push_field_invalid', proc { |c| c['scope'] = [] }],
      ['push_field_invalid', proc { |c| c['status'] = 'READY' }],
      ['push_field_invalid', proc { |c| c['kinds'][0]['status'] = 'MAYBE' }],
      ['push_field_invalid', proc { |c| c['kinds'][0]['primary_phase'] = 'noon' }],
      ['duplicate_kind', proc { |c| c['kinds'] << c['kinds'][0].dup }],
      ['producer_missing', proc { |c| c['kinds'][0]['status'] = 'ACTIVE' }],
      ['producer_reference_missing', proc { |c| c['kinds'][0]['producer_ids'] = ['unknown'] }],
      ['evidence_reference_missing', proc { |c| c['kinds'][0]['evidence_ids'] = ['unknown'] }]
    ]
    cases.each do |reason, mutation|
      with_fixture do |root|
        mutate(root, 'push-capability-catalog.v1.json', &mutation)
        out, err, result = cli(root)
        refute result.success?, reason
        assert_includes out + err, reason
        refute_includes out + err, 'Traceback'
      end
    end
  end

  def test_git_baseline_file_set_and_symbol_expectations_are_frozen
    cases = [
      ['baseline_commit_invalid', proc { |m| m['baseline_commit'] = '0' * 40 }],
      ['baseline_commit_invalid', proc { |m| m['baseline_commit'] = 'HEAD' }],
      ['baseline_mismatch', proc { |m| m['baseline_commit'] = 'a' * 40 }],
      ['file_sha_mismatch', proc { |m| m['files'][0]['sha256'] = '0' * 64 }],
      ['file_set_mismatch', proc { |m| m['files'] = [] }],
      ['symbol_sha_mismatch', proc { |m| m['evidence'][0]['symbol_sha256'] = '0' * 64 }],
      ['symbol_lines_mismatch', proc { |m| m['evidence'][0]['start_line'] = 2 }],
      ['symbol_missing', proc { |m| m['evidence'][0]['symbol'] = 'Renamed' }],
      ['duplicate_evidence', proc { |m| m['evidence'] << m['evidence'][0].dup }],
      ['push_field_invalid', proc { |m| m['files'][0]['path'] = 'src/../src/notify.rs' }]
    ]
    cases.each do |reason, mutation|
      with_fixture do |root|
        mutate(root, 'push-evidence-manifest.v1.json', &mutation)
        out, err, result = cli(root)
        refute result.success?, reason
        assert_includes out + err, reason
      end
    end
    with_fixture do |root|
      File.binwrite(File.join(root, 'src/new.rs'), 'fn another_producer() {}')
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'file_set_mismatch'
    end
  end

  def test_lexical_evidence_ignores_fake_declarations_and_hashes_original_crlf_bytes
    with_fixture do |root|
      source = "// fn send() { fake }\r\n" +
               "/* nested /* fn send() {} */ enum PushKind { Fake } */\r\n" +
               "const TEXT: &str = r###\"fn send() { }\"###;\r\n" +
               "const OTHER: &str = \"fn send() { \\\" }\";\r\n" +
               "pub fn send<'a>(s: &'a str) {\r\n    let 中文 = '中'; let brace = '}'; let escaped = '\\'';\r\n}\r\n"
      add_code_evidence(root, source, 'send', 'rust_fn', 5, 7)
      out, err, result = cli(root)
      assert result.success?, out + err
      File.open(File.join(root, 'src/send.rs'), 'ab') { |file| file.write("fn send() {}\r\n") }
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'symbol_ambiguous'
    end
  end

  def test_item_with_const_generic_braces_locates_its_body
    with_fixture do |root|
      source = "fn send(value: [u8; { 2 }]) {\n    let done = true;\n}\n"
      add_code_evidence(root, source, 'send', 'rust_fn', 1, 3)
      out, err, result = cli(root)
      assert result.success?, out + err
    end
  end

  def test_producer_unit_and_shared_owner_relationships_are_bidirectional
    cases = [
      ['producer_kind_mismatch', proc { |c| c['producers'][0]['kinds'] = [] }],
      ['producer_kind_mismatch', proc { |c| c['kinds'][0]['producer_ids'] = [] }],
      ['unit_reference_missing', proc { |c| c['migration_units'] = [] }],
      ['unit_producer_mismatch', proc { |c| c['migration_units'][0]['producer_ids'] = ['missing'] }],
      ['completion_owner_mismatch', proc { |c| c['migration_units'][0]['completion_owner'] = 'other/day' }],
      ['unit_occurrence_mismatch', proc { |c| c['migration_units'][0]['occurrence_families'] = ['other'] }],
      ['unit_phase_mismatch', proc { |c| c['migration_units'][0]['phase_epics'] = ['盘后'] }],
      ['shared_owner_split', proc do |c|
        second = Marshal.load(Marshal.dump(c['producers'][0]))
        second['id'] = 'second'
        second['migration_unit_id'] = 'second-unit'
        c['producers'] << second
        c['kinds'][0]['producer_ids'] << 'second'
        unit = Marshal.load(Marshal.dump(c['migration_units'][0]))
        unit['id'] = 'second-unit'
        unit['producer_ids'] = ['second']
        c['migration_units'] << unit
      end]
    ]
    cases.each do |reason, mutation|
      with_fixture do |root|
        add_producer(root)
        out, err, result = cli(root)
        assert result.success?, out + err
        mutate(root, 'push-capability-catalog.v1.json', &mutation)
        out, err, result = cli(root)
        refute result.success?, reason
        assert_includes out + err, reason
      end
    end
  end

  def test_generic_impl_header_is_exact_normalized_and_ambiguity_fails
    with_fixture do |root|
      source = "impl<'a, T> Trait for Type<'a, T>\nwhere T: Send {\n    fn method(&self) {}\n}\n"
      add_code_evidence(root, source, "<'a, T> Trait for Type<'a, T> where T: Send", 'rust_impl', 1, 4)
      out, err, result = cli(root)
      assert result.success?, out + err
      File.open(File.join(root, 'src/send.rs'), 'ab') { |file| file.write(source) }
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'symbol_ambiguous'
    end
  end

  def test_inline_impl_duplicates_are_ambiguous_even_when_another_impl_starts_a_line
    with_fixture do |root|
      source = "mod a { struct Foo; impl Foo {} }\nstruct Foo;\nimpl Foo {}\n"
      add_code_evidence(root, source, 'Foo', 'rust_impl', 3, 3)
      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out + err, 'symbol_ambiguous symbol=Foo'
    end
  end

  def test_inline_generic_impl_ignores_masked_declarations_and_preserves_complete_header
    with_fixture do |root|
      source = "// impl<'a, T> Trait for Foo<'a, T> where T: Send {}\n" +
               "const TEXT: &str = r#\"impl<'a, T> Trait for Foo<'a, T> where T: Send {}\"#;\n" +
               "/* impl<'a, T> Trait for Foo<'a, T> where T: Send {} */\n" +
               "mod a { unsafe impl<'a, T> Trait for Foo<'a, T> where T: Send {} }\n"
      add_code_evidence(root, source, "<'a, T> Trait for Foo<'a, T> where T: Send", 'rust_impl', 4, 4)
      out, err, result = cli(root)
      assert_equal 0, result.exitstatus, out + err
    end
  end

  def test_strict_dirty_and_provisional_never_hide_actual_drift
    with_fixture do |root|
      out, err, result = Open3.capture3(RbConfig.ruby, CHECK_CATALOG, '--root', root)
      refute result.success?
      assert_includes out + err, 'provisional'
      assert_includes out + err, 'worktree_dirty'
      File.binwrite(File.join(root, 'src/notify.rs'), "pub enum PushKind { // changed\n One,\n}\n")
      out, err, result = Open3.capture3(RbConfig.ruby, CHECK_CATALOG, '--root', root)
      refute result.success?
      assert_includes out + err, 'provisional'
      assert_includes out + err, 'file_sha_mismatch'
      assert_includes out + err, 'symbol_sha_mismatch'
    end
  end

  def test_rejects_invalid_json_cli_paths_and_nonancestor_commit
    with_fixture do |root|
      out, err, result = cli(root, '--unknown')
      refute result.success?
      assert_equal 2, result.exitstatus
      assert_includes out + err, 'invalid option'
      mutate(root, 'push-evidence-manifest.v1.json') { |m| m['evidence'][0]['path'] = '/tmp/outside.rs' }
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'push_field_invalid'
      File.binwrite(File.join(root, 'docs/push-system/push-capability-catalog.v1.json'), '{')
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'push_json_invalid'
    end
    with_fixture do |root|
      original = git(root, 'rev-parse', 'HEAD')
      git(root, 'switch', '--orphan', 'unrelated')
      File.binwrite(File.join(root, 'unrelated.txt'), "unrelated\n")
      git(root, 'add', 'unrelated.txt')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'unrelated baseline')
      other = git(root, 'rev-parse', 'HEAD')
      git(root, 'switch', '--detach', original)
      %w[push-capability-catalog.v1.json push-evidence-manifest.v1.json].each do |path|
        mutate(root, path) { |document| document['baseline_commit'] = other }
      end
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'baseline_not_ancestor'
    end
  end

  def test_renderer_is_read_only_until_explicit_write_and_is_idempotent
    with_fixture do |root|
      render = proc { |mode| Open3.capture3(RbConfig.ruby, RENDER_CATALOG, '--root', root, mode) }
      output = File.join(root, 'docs/push-system/push-capability-catalog.md')
      manifest = File.binread(File.join(root, 'docs/push-system/push-evidence-manifest.v1.json'))
      out, err, result = render.call('--check')
      refute result.success?
      assert_includes out + err, 'markdown_missing'
      refute File.exist?(output)
      out, err, result = render.call('--write')
      assert result.success?, out + err
      first = File.binread(output)
      %w[盘前 集合竞价 盘中 盘后 One INACTIVE PushKind PaperBuy Watchdog PROVISIONAL].each do |word|
        assert_includes first.dup.force_encoding('UTF-8'), word
      end
      out, err, result = render.call('--write')
      assert result.success?, out + err
      assert_equal first, File.binread(output)
      assert_equal manifest, File.binread(File.join(root, 'docs/push-system/push-evidence-manifest.v1.json'))
      out, err, result = render.call('--check')
      assert result.success?, out + err
      File.binwrite(output, 'stale')
      out, err, result = render.call('--check')
      refute result.success?
      assert_includes out + err, 'markdown_stale'
      assert_equal 'stale', File.binread(output)
    end
  end

  def test_return_const_generic_braces_and_inline_module_have_complete_items
    with_fixture do |root|
      source = "fn send() -> Array<{ 2 }> {\n    Array::new()\n}\n"
      add_code_evidence(root, source, 'send', 'rust_fn', 1, 3)
      out, err, result = cli(root)
      assert result.success?, out + err
    end
    with_fixture do |root|
      source = "mod nested {\n    fn item() {}\n}\n"
      add_code_evidence(root, source, 'nested', 'rust_mod', 1, 3)
      out, err, result = cli(root)
      assert result.success?, out + err
    end
  end

  def test_duplicate_locator_and_symlink_escape_fail_without_backtrace
    with_fixture do |root|
      mutate(root, 'push-evidence-manifest.v1.json') do |manifest|
        duplicate = manifest['evidence'][0].dup
        duplicate['id'] = 'duplicate'
        manifest['evidence'] << duplicate
      end
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['kinds'][0]['evidence_ids'] << 'duplicate' }
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'duplicate_locator'
    end
    with_fixture do |root|
      Dir.mktmpdir('push-catalog-outside-') do |outside|
        path = File.join(outside, 'notify.rs')
        File.binwrite(path, File.binread(File.join(root, 'src/notify.rs')))
        FileUtils.rm(File.join(root, 'src/notify.rs'))
        File.symlink(path, File.join(root, 'src/notify.rs'))
        out, err, result = cli(root)
        refute result.success?
        assert_includes out + err, 'file_path_invalid'
        assert_includes out + err, 'evidence_path_invalid'
        refute_includes out + err, 'Traceback'
      end
    end
  end

  def test_hidden_rust_additions_and_cargo_bytes_are_in_the_frozen_set
    with_fixture do |root|
      FileUtils.mkdir_p(File.join(root, 'src/.hidden'))
      File.binwrite(File.join(root, 'src/.hidden/producer.rs'), 'fn send() {}')
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'file_set_mismatch'
    end
    with_fixture do |root|
      File.binwrite(File.join(root, 'Cargo.toml'), "[package]\nname = 'fixture'\n")
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'file_set_mismatch'
    end
  end

  def test_unicode_function_names_preserve_byte_offsets_without_backtrace
    with_fixture do |root|
      source = "pub fn 发送() {\n    let 名称 = \"消息\";\n}\n"
      add_code_evidence(root, source, '发送', 'rust_fn', 1, 3)
      out, err, result = cli(root)
      assert result.success?, out + err
    end
  end

  def test_bodyless_declarations_fail_closed
    [['fn send();\n', 'send', 'rust_fn'], ['mod nested;\n', 'nested', 'rust_mod']].each do |literal, symbol, kind|
      with_fixture do |root|
        source = literal.gsub('\\n', "\n")
        add_code_evidence(root, source, symbol, kind, 1, 1)
        out, err, result = cli(root)
        refute result.success?
        assert_includes out + err, 'symbol_body_missing'
        refute_includes out + err, 'Traceback'
      end
    end
  end

  def test_ignored_files_are_not_dirty_and_source_errors_propagate
    with_fixture do |root|
      File.binwrite(File.join(root, '.gitignore'), "ignored.txt\n")
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'fixture catalog')
      File.binwrite(File.join(root, 'ignored.txt'), "local only\n")
      out, err, result = Open3.capture3(RbConfig.ruby, CHECK_CATALOG, '--root', root)
      refute result.success?
      assert_includes out + err, 'provisional'
      refute_includes out + err, 'worktree_dirty'
      File.binwrite(File.join(root, 'docs/decisions.md'), 'changed')
      out, err, result = cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_sha_mismatch'
    end
  end

  def test_invalid_top_level_shapes_fail_without_backtrace
    [nil, [], 42, 'invalid'].each do |value|
      with_fixture do |root|
        json(root, 'docs/push-system/push-capability-catalog.v1.json', value)
        out, err, result = cli(root)
        refute result.success?
        assert_includes out + err, 'push_structure_invalid'
        refute_includes out + err, 'Traceback'
      end
    end
  end

  def test_renderer_cannot_overwrite_manifest_through_an_internal_link
    [:symlink, :link].each do |link_type|
      with_fixture do |root|
        manifest = File.join(root, 'docs/push-system/push-evidence-manifest.v1.json')
        original = File.binread(manifest)
        File.public_send(link_type, manifest, File.join(root, 'docs/push-system/push-capability-catalog.md'))
        out, err, result = Open3.capture3(RbConfig.ruby, RENDER_CATALOG, '--root', root, '--write')
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, 'markdown_path_invalid'
        assert_equal original, File.binread(manifest)
      end
    end
  end

  def test_renderer_rejects_dangling_symlink_without_creating_its_target
    Dir.mktmpdir('catalog-owned-parent-') do |parent|
      with_fixture(parent) do |root|
        target = File.join(parent, 'must-not-be-created.md')
        output = File.join(root, 'docs/push-system/push-capability-catalog.md')
        File.symlink(target, output)
        assert File.symlink?(output)
        refute File.exist?(target)
        out, err, result = Open3.capture3(RbConfig.ruby, RENDER_CATALOG, '--root', root, '--write')
        refute File.exist?(target), 'renderer followed the dangling symlink outside the fixture root'
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, 'markdown_path_invalid'
        assert File.symlink?(output)
      end
    end
  end

  def test_signature_fragments_cannot_bypass_complete_identifier_ambiguity
    [
      ["mod a { fn send() {} }\nmod b { fn send(x: u8) {} }\n", 'send()', 'send', 'rust_fn'],
      ["mod a { enum Event { One } }\nmod b { enum Event { Two } }\n", 'Event { One', 'Event', 'rust_enum'],
      ["mod a { mod inner { fn first() {} } }\nmod b { mod inner { fn second() {} } }\n", 'inner { fn first()', 'inner', 'rust_mod']
    ].each do |source, fragment, identifier, kind|
      with_fixture do |root|
        add_code_evidence(root, source, fragment, kind, 1, 1)
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, 'symbol_identifier_invalid'
        mutate(root, 'push-evidence-manifest.v1.json') { |manifest| manifest['evidence'].last['symbol'] = identifier }
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, 'symbol_ambiguous'
      end
    end
  end

  def test_complete_raw_identifiers_are_supported_for_functions_enums_and_modules
    [
      ['fn r#type() {}', 'r#type', 'rust_fn'],
      ['enum r#match { One }', 'r#match', 'rust_enum'],
      ['mod r#type {}', 'r#type', 'rust_mod']
    ].each do |source, identifier, kind|
      with_fixture do |root|
        add_code_evidence(root, source, identifier, kind, 1, 1)
        out, err, result = cli(root)
        assert_equal 0, result.exitstatus, out + err
      end
    end
  end

  def test_missing_owner_and_noninteger_schema_fail_with_reason_codes
    with_fixture do |root|
      add_producer(root)
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['producers'][0].delete('completion_owner') }
      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out + err, 'field=completion_owner'
    end
    with_fixture do |root|
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['schema_version'] = 1.0 }
      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out + err, 'field=schema_version'
    end
  end

  def test_missing_documents_are_both_reported_and_public_api_has_no_global_state_changes
    with_fixture do |root|
      cwd = Dir.pwd
      environment = ENV.to_h
      errors = ArchitectureDocs::Catalog.validate(root, strict: true)
      assert_kind_of Array, errors
      assert errors.all? { |error| error.is_a?(String) }
      assert errors.any? { |error| error.start_with?('provisional') }
      assert_equal cwd, Dir.pwd
      assert_equal environment, ENV.to_h
      %w[push-capability-catalog.v1.json push-evidence-manifest.v1.json].each do |path|
        FileUtils.rm(File.join(root, 'docs/push-system', path))
      end
      out, err, result = cli(root)
      assert_equal 1, result.exitstatus
      assert_includes out + err, 'push_document_missing'
      assert_includes out + err, 'push-capability-catalog.v1.json'
      assert_includes out + err, 'push-evidence-manifest.v1.json'
    end
  end

  private

  def add_producer(root)
    mutate(root, 'push-capability-catalog.v1.json') do |catalog|
      catalog['kinds'][0]['status'] = 'ACTIVE'
      catalog['kinds'][0]['producer_ids'] = ['one']
      producer = { 'id' => 'one', 'kinds' => ['One'], 'phase_epics' => ['盘中'], 'occurrence_family' => 'one/day',
                   'completion_owner' => 'last_one/day', 'migration_unit_id' => 'unit-one',
                   'evidence_ids' => ['enum'], 'known_gaps' => [] }
      %w[trigger source authority policy].each do |field|
        producer[field] = { 'description' => '仅临时 fixture 测试关系。', 'evidence_ids' => ['enum'] }
      end
      catalog['producers'] = [producer]
      catalog['migration_units'] = [{ 'id' => 'unit-one', 'producer_ids' => ['one'], 'completion_owner' => 'last_one/day',
                                     'occurrence_families' => ['one/day'], 'phase_epics' => ['盘中'], 'note' => '同状态同范围' }]
    end
  end

  def add_code_evidence(root, source, symbol, kind, first, last)
    File.binwrite(File.join(root, 'src/send.rs'), source)
    git(root, 'add', 'src/send.rs')
    git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'fixture evidence')
    baseline = git(root, 'rev-parse', 'HEAD')
    mutate(root, 'push-evidence-manifest.v1.json') do |manifest|
      manifest['baseline_commit'] = baseline
      manifest['files'] << { 'path' => 'src/send.rs', 'sha256' => Digest::SHA256.hexdigest(source) }
      manifest['evidence'] << { 'id' => 'send', 'path' => 'src/send.rs', 'symbol' => symbol, 'kind' => kind,
                                'symbol_sha256' => Digest::SHA256.hexdigest(source.lines[(first - 1)..(last - 1)].join),
                                'start_line' => first, 'end_line' => last }
    end
    mutate(root, 'push-capability-catalog.v1.json') do |catalog|
      catalog['baseline_commit'] = baseline
      catalog['kinds'][0]['evidence_ids'] << 'send'
    end
  end

  def mutate(root, filename)
    path = 'docs/push-system/' + filename
    data = JSON.parse(File.binread(File.join(root, path)))
    yield data
    json(root, path, data)
  end

  def cli(root, *options)
    Open3.capture3(RbConfig.ruby, CHECK_CATALOG, '--root', root, *(options.empty? ? ['--draft'] : options))
  end

  def git(root, *args)
    out, err, result = Open3.capture3('git', '-C', root, *args)
    raise out + err unless result.success?
    out.strip
  end

  def json(root, path, object)
    File.binwrite(File.join(root, path), JSON.pretty_generate(object) + "\n")
  end

  def with_fixture(parent = nil)
    Dir.mktmpdir('push-catalog-', parent) do |root|
      FileUtils.mkdir_p(File.join(root, 'docs/push-system'))
      FileUtils.mkdir_p(File.join(root, 'src'))
      source = "pub enum PushKind {\n    One,\n}\n"
      File.binwrite(File.join(root, 'src/notify.rs'), source)
      decisions = "## Q1--Q55\n" + (1..55).map { |n| "| #{n} | yes |\n" }.join +
                  "## Q56--Q108\n" + (56..108).map { |n| "| #{n} | yes |\n" }.join
      File.binwrite(File.join(root, 'docs/decisions.md'), decisions)
      json(root, 'design-source-catalog.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'provenance' => 'user_workspace_snapshot',
        'approved_decisions' => { 'path' => 'docs/decisions.md', 'sha256' => Digest::SHA256.hexdigest(decisions), 'question_count' => 108 },
        'sources' => []
      })
      git(root, 'init', '-q')
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'fixture baseline')
      baseline = git(root, 'rev-parse', 'HEAD')
      json(root, 'docs/push-system/push-evidence-manifest.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'baseline_commit' => baseline,
        'files' => [{ 'path' => 'src/notify.rs', 'sha256' => Digest::SHA256.hexdigest(source) }],
        'evidence' => [{ 'id' => 'enum', 'path' => 'src/notify.rs', 'symbol' => 'PushKind', 'kind' => 'rust_enum',
                         'symbol_sha256' => Digest::SHA256.hexdigest(source), 'start_line' => 1, 'end_line' => 3 }]
      })
      json(root, 'docs/push-system/push-capability-catalog.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'baseline_commit' => baseline,
        'scope' => '隔离代码基线；源码审计≠部署证明；NOT CHECKED：完整RFC/WBS/离线HTML/CI/运行时Foundation/部署/真实接收/工期。',
        'enum_evidence_id' => 'enum',
        'kinds' => [{ 'kind' => 'One', 'primary_phase' => '盘中', 'status' => 'INACTIVE',
                     'producer_ids' => [], 'evidence_ids' => ['enum'], 'note' => 'fixture 无生产 caller。' }],
        'producers' => [], 'migration_units' => [],
        'excluded_worktree_additions' => [
          { 'kind' => 'PaperBuy', 'reason' => '仅原混合工作树存在，本分支未移入' },
          { 'kind' => 'Watchdog', 'reason' => '仅原混合工作树存在，本分支未移入' }
        ]
      })
      yield root
    end
  end
end
