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
  def test_renderer_reports_missing_root_and_parent_without_backtrace
    Dir.mktmpdir('renderer-missing-') do |root|
      [root, File.join(root, 'missing')].each do |target|
        out, err, status = Open3.capture3(RbConfig.ruby, RENDER_CATALOG, '--root', target, '--current', '--check')
        assert_equal 1, status.exitstatus, out + err
        assert_includes out, 'push_document_missing'
        assert_empty err
      end
    end
  end

  def test_current_nonancestor_and_missing_source_are_rejected
    with_fixture do |root|
      original = git(root, 'rev-parse', 'HEAD')
      git(root, 'switch', '--orphan', 'unrelated-current')
      File.binwrite(File.join(root, 'unrelated.txt'), "unrelated\n")
      git(root, 'add', 'unrelated.txt')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'unrelated current')
      other = git(root, 'rev-parse', 'HEAD')
      git(root, 'switch', '--detach', original)
      %w[push-current-capability-catalog.v1.json push-current-evidence-manifest.v1.json].each do |name|
        mutate(root, name) { |document| document['baseline_commit'] = other }
      end
      bind_current(root)
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, "baseline_not_ancestor commit=#{other} pair=current"
      assert_empty err
    end
    with_fixture do |root|
      File.delete(File.join(root, 'src/notify.rs'))
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'file_path_invalid path=src/notify.rs pair=current'
      assert_includes out, 'evidence_path_invalid id=enum pair=current'
      assert_empty err
    end
  end

  def test_architecture_only_evidence_is_not_a_business_producer
    with_fixture do |root|
      File.binwrite(File.join(root, 'src/notify.rs'), "pub enum PushKind {\n    One,\n}\nfn library() {}\n")
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'library')
      install_current(root)
      mutate(root, 'push-current-capability-catalog.v1.json') do |catalog|
        catalog['architecture'] = [{ 'id' => 'library', 'evidence_ids' => ['library'],
          'supporting_files' => ['src/notify.rs'], 'boundary' => 'Declaration only; no production caller or migration promotion.' }]
      end
      mutate(root, 'push-current-evidence-manifest.v1.json') do |manifest|
        manifest['files'][0]['architecture_ids'] = ['library']
        manifest['evidence'] << { 'id' => 'library', 'path' => 'src/notify.rs', 'symbol' => 'library', 'kind' => 'rust_fn',
          'symbol_sha256' => Digest::SHA256.hexdigest("fn library() {}\n"), 'start_line' => 4, 'end_line' => 4,
          'audit_domains' => ['architecture'], 'dependencies' => [] }
      end
      bind_current(root)
      out, err, status = cli(root)
      assert_equal 0, status.exitstatus, out + err
      catalog = JSON.parse(File.binread(File.join(root, 'docs/push-system/push-current-capability-catalog.v1.json')))
      assert_empty catalog['producers']
      assert_empty catalog['migration_units']
      assert_equal 'INACTIVE', catalog['kinds'][0]['status']
    end
  end

  def test_standalone_strict_check_preserves_index_after_source_stat_change
    with_fixture do |root|
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'tracked audit fixture')
      source = File.join(root, 'src/notify.rs')
      later = Time.now + 10
      File.utime(later, later, source)
      index = File.join(root, '.git/index')
      before = [Digest::SHA256.file(index).hexdigest, File.stat(index).mtime.to_r]
      out, err, status = cli(root, '--check')
      assert_equal 1, status.exitstatus, out + err
      refute_includes out, 'worktree_dirty'
      assert_equal before, [Digest::SHA256.file(index).hexdigest, File.stat(index).mtime.to_r]
    end
  end

  def test_current_paths_and_json_shapes_fail_without_backtraces
    [nil, [], 42, {}, { 'schema_version' => 1, 'status' => 'PROVISIONAL', 'role' => 'current-source-audit', 'bindings' => [] }].each do |value|
      with_fixture do |root|
        json(root, 'docs/push-system/push-current-evidence-manifest.v1.json', value)
        out, err, status = cli(root)
        assert_equal 1, status.exitstatus, out + err
        assert_includes out, 'pair=current'
        assert_empty err
      end
    end
    %w[push-current-capability-catalog.v1.json push-current-evidence-manifest.v1.json].each do |name|
      with_fixture do |root|
        path = File.join(root, 'docs/push-system', name)
        File.binwrite(path, '{')
        out, err, status = cli(root)
        assert_equal 1, status.exitstatus, out + err
        assert_includes out, "push_json_invalid path=docs/push-system/#{name}"
      end
    end
    [:symlink, :link].each do |link_type|
      %w[docs/push-system/push-current-capability-catalog.v1.json docs/push-system/push-current-evidence-manifest.v1.json src/notify.rs].each do |relative|
        with_fixture do |root|
          path = File.join(root, relative)
          saved = path + '.saved'
          File.rename(path, saved)
          File.public_send(link_type, saved, path)
          out, err, status = cli(root)
          assert_equal 1, status.exitstatus, out + err
          assert_includes out, relative.start_with?('src/') ? 'file_path_invalid' : 'push_path_invalid'
          assert_empty err
        end
      end
    end
  end

  def test_current_integrity_failures_and_pair_origins_are_independent
    cases = {
      'file_sha_mismatch' => proc { |m| m['files'][0]['sha256'] = '0' * 64 },
      'symbol_sha_mismatch' => proc { |m| m['evidence'][0]['symbol_sha256'] = '0' * 64 },
      'symbol_lines_mismatch' => proc { |m| m['evidence'][0]['start_line'] = 2 },
      'symbol_missing' => proc { |m| m['evidence'][0]['symbol'] = 'missing' },
      'file_set_mismatch' => proc { |m| m['files'] = [] },
      'baseline_commit_invalid' => proc { |m| m['baseline_commit'] = '0' * 40 },
      'baseline_mismatch' => proc { |m| m['baseline_commit'] = 'a' * 40 },
      'current_bindings_mismatch' => proc { |m| m['bindings'][0]['path'] = 'docs/other.json' },
      'duplicate_locator' => proc { |m| m['evidence'] << m['evidence'][0].merge('id' => 'duplicate') }
    }
    cases.each do |code, mutation|
      with_fixture do |root|
        mutate(root, 'push-current-evidence-manifest.v1.json', &mutation)
        out, err, status = cli(root)
        assert_equal 1, status.exitstatus, out + err
        assert out.lines.any? { |line| line.start_with?(code) && line.include?('pair=current') }, out
        assert_empty err
      end
    end
    with_fixture do |root|
      %w[push-evidence-manifest.v1.json push-current-evidence-manifest.v1.json].each do |name|
        mutate(root, name) { |m| m['evidence'][0]['symbol_sha256'] = '0' * 64 }
      end
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'symbol_sha_mismatch origin=baseline id=enum pair=historical'
      assert_includes out, 'symbol_sha_mismatch origin=baseline id=enum pair=current'
    end
  end

  def test_supporting_file_requires_its_own_architecture_reference
    with_fixture do |root|
      mutate(root, 'push-current-evidence-manifest.v1.json') { |m| m['files'][0]['architecture_ids'] = ['removed-group'] }
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'architecture_file_ownership_mismatch path=src/notify.rs'
    end
  end

  def test_current_original_byte_bindings_reject_whitespace_and_schema_damage_does_not_hide_other_pair
    %w[push-capability-catalog.v1.json push-evidence-manifest.v1.json push-current-capability-catalog.v1.json].each do |name|
      with_fixture do |root|
        path = File.join(root, 'docs/push-system', name)
        File.open(path, 'ab') { |file| file.write("\n ") }
        out, err, status = cli(root)
        assert_equal 1, status.exitstatus, out + err
        assert_includes out, "binding_sha_mismatch path=docs/push-system/#{name} pair=current"
      end
    end
    with_fixture do |root|
      File.binwrite(File.join(root, 'docs/push-system/push-capability-catalog.v1.json'), '{')
      mutate(root, 'push-current-evidence-manifest.v1.json') { |m| m['evidence'][0]['symbol_sha256'] = '0' * 64 }
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'push_json_invalid path=docs/push-system/push-capability-catalog.v1.json'
      assert_includes out, 'symbol_sha_mismatch id=enum pair=current'
      assert_empty err
    end
  end

  def test_current_pin_survives_docs_commits_and_cannot_promote_identities
    with_fixture do |root|
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'docs only')
      out, err, status = cli(root)
      assert_equal 0, status.exitstatus, out + err
      mutate(root, 'push-current-capability-catalog.v1.json') { |c| c['kinds'][0]['status'] = 'STARVED' }
      bind_current(root)
      out, err, status = cli(root)
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'current_identity_mismatch collection=kinds'
    end
  end

  def test_current_renderer_uses_fixed_separate_output
    with_fixture do |root|
      out, err, result = Open3.capture3(RbConfig.ruby, RENDER_CATALOG, '--root', root, '--current', '--write')
      assert_equal 0, result.exitstatus, out + err
      path = File.join(root, 'docs/push-system/push-current-capability-catalog.md')
      assert_includes File.read(path), '当前源码审计'
      assert_includes File.read(path), '历史实施规范'
      refute File.exist?(File.join(root, 'docs/push-system/push-capability-catalog.md'))
    end
  end

  def test_current_architecture_and_business_dependency_closures_are_independent
    with_fixture do |root|
      add_producer(root)
      File.binwrite(File.join(root, 'src/notify.rs'), "pub enum PushKind {\n    One,\n}\nfn helper() {}\n")
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'helper')
      install_current(root)
      mutate(root, 'push-current-evidence-manifest.v1.json') do |manifest|
        manifest['files'][0]['architecture_ids'] = ['library']
        manifest['evidence'][0]['dependencies'] = [{ 'evidence_id' => 'helper', 'description' => 'fixture enum depends on helper' }]
        manifest['evidence'] << { 'id' => 'helper', 'path' => 'src/notify.rs', 'symbol' => 'helper', 'kind' => 'rust_fn',
          'symbol_sha256' => Digest::SHA256.hexdigest("fn helper() {}\n"), 'start_line' => 4, 'end_line' => 4,
          'audit_domains' => %w[business architecture], 'dependencies' => [] }
      end
      mutate(root, 'push-current-capability-catalog.v1.json') do |catalog|
        catalog['architecture'] = [{ 'id' => 'library', 'evidence_ids' => ['helper'],
          'supporting_files' => ['src/notify.rs'], 'boundary' => 'helper declaration only; no production caller' }]
      end
      bind_current(root)
      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out, 'evidence_dependency_missing'
      assert_includes out, 'evidence_unreferenced id=helper'
      mutate(root, 'push-current-capability-catalog.v1.json') do |catalog|
        catalog['kinds'][0]['evidence_ids'] << 'helper'
        producer = catalog['producers'][0]
        producer['evidence_ids'] << 'helper'
        %w[trigger source authority policy].each { |field| producer[field]['evidence_ids'] << 'helper' }
      end
      bind_current(root)
      out, err, result = cli(root)
      assert_equal 0, result.exitstatus, out + err
      original = File.binread(File.join(root, 'docs/push-system/push-current-capability-catalog.v1.json'))
      {
        'architecture_reference_missing' => proc { |c| c['architecture'][0]['evidence_ids'] << 'missing' },
        'architecture_evidence_unreferenced' => proc { |c| c['architecture'] = [] },
        'architecture_file_incomplete' => proc { |c| c['architecture'][0]['supporting_files'] = ['src/other.rs'] },
        'architecture_file_missing' => proc { |c| c['architecture'][0]['supporting_files'] << 'src/other.rs' },
        'duplicate_architecture' => proc { |c| c['architecture'] << c['architecture'][0].dup },
        'evidence_dependency_missing' => proc { |c| c['producers'][0]['source']['evidence_ids'].delete('helper') },
        'producer_evidence_incomplete' => proc { |c| c['producers'][0]['evidence_ids'].delete('helper') }
      }.each do |code, mutation|
        File.binwrite(File.join(root, 'docs/push-system/push-current-capability-catalog.v1.json'), original)
        mutate(root, 'push-current-capability-catalog.v1.json', &mutation)
        bind_current(root)
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out, code
      end
    end
  end

  def test_historical_b_and_mandatory_current_c_validate_together
    with_fixture do |root|
      source = "// current C\npub enum PushKind {\n    One,\n}\n"
      File.binwrite(File.join(root, 'src/notify.rs'), source)
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'current C')
      install_current(root)
      out, err, result = cli(root)
      assert_equal 0, result.exitstatus, out + err
      %w[push-current-capability-catalog.v1.json push-current-evidence-manifest.v1.json].each do |name|
        path = File.join(root, 'docs/push-system', name)
        bytes = File.binread(path)
        FileUtils.rm(path)
        %w[--draft --check].each do |mode|
          out, err, result = cli(root, mode)
          assert_equal 1, result.exitstatus, out + err
          assert_includes out, "push_document_missing path=docs/push-system/#{name}"
        end
        File.binwrite(path, bytes)
      end
    end
  end

  def test_source_bound_view_locates_multiple_symbols_with_original_crlf_bytes
    source = [
      "// fn send() { ignored }\r\n",
      "pub enum Café { One, Two }\r\n",
      "fn send() { let value = \"}\"; }\r\n",
      "mod nested { fn inside() {} }\r\n",
      "impl Thing { fn method(&self) {} }\r\n"
    ].join.b
    view = ArchitectureDocs::RustEvidence.view(source)

    [['Café', 'rust_enum', 2], ['send', 'rust_fn', 3], ['nested', 'rust_mod', 4],
     ['Thing', 'rust_impl', 5]].each do |symbol, kind, line|
      located = view.locate(symbol, kind)
      assert_equal line, located['start_line']
      assert_equal line, located['end_line']
      assert_equal Digest::SHA256.hexdigest(source.lines[line - 1]), located['symbol_sha256']
    end
    source.replace("fn replaced() {}\n")
    assert_equal 3, view.locate('send', 'rust_fn')['start_line']
  end

  def test_git_batch_parser_validates_every_blob_frame
    object = 'a' * 40
    valid = object + " blob 3\nabc\n"
    assert_equal({ object => 'abc'.b }, ArchitectureDocs::Catalog.parse_git_batch([object], valid.b))
    missing = assert_raises(ArchitectureDocs::Catalog::GitBatchInvalid) do
      ArchitectureDocs::Catalog.parse_git_batch([object], ''.b)
    end
    assert_equal 'baseline_batch_truncated', missing.message

    failures = {
      'baseline_batch_truncated' => object + " blob 4\nabc",
      'baseline_batch_type_invalid' => object + " tree 3\nabc\n",
      'baseline_batch_object_mismatch' => ('b' * 40) + " blob 3\nabc\n",
      'baseline_batch_terminator_invalid' => object + " blob 3\nabcX",
      'baseline_batch_extra' => valid + 'extra'
    }
    failures.each do |message, bytes|
      error = assert_raises(ArchitectureDocs::Catalog::GitBatchInvalid) do
        ArchitectureDocs::Catalog.parse_git_batch([object], bytes.b)
      end
      assert_equal message, error.message
    end
  end

  def test_catalog_cli_reuses_one_source_view_for_multiple_symbols
    with_fixture do |root|
      source = "pub enum PushKind {\n    One,\n}\nfn send() {}\n"
      File.binwrite(File.join(root, 'src/notify.rs'), source)
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'add second symbol')
      baseline = git(root, 'rev-parse', 'HEAD')
      mutate(root, 'push-evidence-manifest.v1.json') do |manifest|
        manifest['baseline_commit'] = baseline
        manifest['files'][0]['sha256'] = Digest::SHA256.hexdigest(source)
        manifest['evidence'] << {
          'id' => 'send', 'path' => 'src/notify.rs', 'symbol' => 'send', 'kind' => 'rust_fn',
          'symbol_sha256' => Digest::SHA256.hexdigest("fn send() {}\n"), 'start_line' => 4, 'end_line' => 4
        }
      end
      mutate(root, 'push-capability-catalog.v1.json') do |catalog|
        catalog['baseline_commit'] = baseline
        catalog['kinds'][0]['evidence_ids'] << 'send'
      end
      install_current(root)

      out, err, result = cli(root)
      assert_equal 0, result.exitstatus, out + err
      assert_includes out, 'push_catalog_valid'
      assert_empty err
    end
  end

  def test_git_batch_reads_real_blob_paths_with_spaces_and_newlines
    with_fixture do |root|
      path = "src/odd name\nfile.rs"
      bytes = "fn odd_path() {}\n"
      File.binwrite(File.join(root, path), bytes)
      git(root, 'add', path)
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'add unusual path')
      baseline = git(root, 'rev-parse', 'HEAD')
      mutate(root, 'push-evidence-manifest.v1.json') do |manifest|
        manifest['baseline_commit'] = baseline
        manifest['files'] << { 'path' => path, 'sha256' => Digest::SHA256.hexdigest(bytes) }
      end
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['baseline_commit'] = baseline }
      install_current(root)
      out, err, result = cli(root)
      assert_equal 0, result.exitstatus, out + err
      assert_includes out, 'push_catalog_valid'
      assert_empty err
    end
  end

  def test_non_blob_code_path_cannot_disappear_from_baseline_file_set
    with_fixture do |root|
      commit = git(root, 'rev-parse', 'HEAD')
      git(root, 'update-index', '--add', '--cacheinfo', "160000,#{commit},src/linked.rs")
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'add code-shaped gitlink')
      baseline = git(root, 'rev-parse', 'HEAD')
      mutate(root, 'push-evidence-manifest.v1.json') { |manifest| manifest['baseline_commit'] = baseline }
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['baseline_commit'] = baseline }

      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out, 'file_set_mismatch'
      assert_empty err
    end
  end

  def test_baseline_and_current_source_damage_remain_distinguishable
    with_fixture do |root|
      source_path = File.join(root, 'src/notify.rs')
      correct = File.binread(source_path)
      File.binwrite(source_path, correct.sub('One', 'Broken'))
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'damaged baseline')
      damaged_baseline = git(root, 'rev-parse', 'HEAD')
      File.binwrite(source_path, correct)
      git(root, 'add', 'src/notify.rs')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid',
          'commit', '-qm', 'restore current source')
      mutate(root, 'push-evidence-manifest.v1.json') { |manifest| manifest['baseline_commit'] = damaged_baseline }
      mutate(root, 'push-capability-catalog.v1.json') { |catalog| catalog['baseline_commit'] = damaged_baseline }

      out, err, result = cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out, 'file_sha_mismatch origin=baseline path=src/notify.rs'
      assert_includes out, 'symbol_sha_mismatch origin=baseline id=enum'
      refute_includes out, "file_sha_mismatch path=src/notify.rs\n"
      refute_includes out, "symbol_sha_mismatch id=enum\n"
      assert_empty err
    end
  end

  def test_explicit_check_preserves_strict_default_and_modes_are_exclusive
    with_fixture do |root|
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'freeze documents')
      expected = ['provisional path=docs/push-system/push-capability-catalog.v1.json pair=historical',
                  'provisional path=docs/push-system/push-evidence-manifest.v1.json pair=historical',
                  'provisional path=docs/push-system/push-current-capability-catalog.v1.json pair=current',
                  'provisional path=docs/push-system/push-current-evidence-manifest.v1.json pair=current']
      out, err, result = cli(root, '--check')
      assert_equal 1, result.exitstatus, out + err
      assert_equal expected, out.lines.map(&:strip).reject { |line| line.start_with?('NOT CHECKED') }
      assert_empty err
      legacy, legacy_err, legacy_result = Open3.capture3(RbConfig.ruby, CHECK_CATALOG, '--root', root)
      assert_equal out, legacy
      assert_equal result.exitstatus, legacy_result.exitstatus
      assert_empty legacy_err
      [%w[--draft --check], %w[--check --draft], %w[--draft --draft],
       %w[--check --check], %w[--unknown], %w[--check extra]].each do |options|
        out, err, result = cli(root, *options)
        assert_equal 2, result.exitstatus, out + err
        assert_empty out
        assert_includes err, 'Usage: check-catalog.rb'
      end
    end
  end

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

  def test_impl_item_locator_rejects_return_position_opaque_types
    [
      ["fn factory() -> impl std::fmt::Debug { 42 }\n", 'std::fmt::Debug', 1],
      ["trait Foo {}\nimpl Foo for () {}\nfn make() -> impl Foo { () }\n", 'Foo', 3]
    ].each do |source, symbol, line|
      with_fixture do |root|
        add_code_evidence(root, source, symbol, 'rust_impl', line, line)
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, "symbol_missing symbol=#{symbol}"
      end
    end
  end

  def test_impl_item_locator_rejects_raw_identifiers_and_type_or_expression_positions
    [
      ["fn build() { let _ = r#impl::Foo {}; }\n", '::Foo'],
      ["type Alias = impl std::fmt::Debug;\n", 'std::fmt::Debug'],
      ["fn consume(value: impl std::fmt::Debug) {}\n", 'std::fmt::Debug'],
      ["fn build() { invoke!(impl Foo {}); }\n", 'Foo'],
      ["fn build() { values[0] impl Foo {} }\n", 'Foo'],
      ["fn build() { values! [0] impl Foo {} }\n", 'Foo']
    ].each do |source, symbol|
      with_fixture do |root|
        add_code_evidence(root, source, symbol, 'rust_impl', 1, 1)
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, "symbol_missing symbol=#{symbol}"
      end
    end
  end

  def test_impl_item_locator_accepts_item_boundaries_and_balanced_attributes
    [
      ["struct Foo; impl Foo {}\n", 'Foo'],
      ["struct Foo {} impl Foo {}\n", 'Foo'],
      ["mod nested { #[cfg(all())] #[doc = r#\" ] impl Fake {} \"#] unsafe impl<T> Trait for Foo<T> where T: Send {} }\n", '<T> Trait for Foo<T> where T: Send'],
      ["#[cfg_attr(feature = \"x\", custom([one, two]))] impl Foo {}\n", 'Foo']
    ].each do |source, symbol|
      with_fixture do |root|
        add_code_evidence(root, source, symbol, 'rust_impl', 1, 1)
        out, err, result = cli(root)
        assert_equal 0, result.exitstatus, out + err
      end
    end
  end

  def test_spaced_attribute_markers_do_not_hide_duplicate_impl_items
    ['# [allow(dead_code)]', '# /* gap */ [allow(dead_code)]',
     '# ! [allow(dead_code)]', '# /* gap */ ! /* gap */ [allow(dead_code)]'].each do |attribute|
      with_fixture do |root|
        source = "mod one { #{attribute} impl Foo {} }\nimpl Foo {}\n"
        add_code_evidence(root, source, 'Foo', 'rust_impl', 2, 2)
        out, err, result = cli(root)
        assert_equal 1, result.exitstatus, out + err
        assert_includes out + err, 'symbol_ambiguous symbol=Foo'
      end
    end
  end

  def test_spaced_attribute_markers_allow_unique_impl_items
    ['# [allow(dead_code)]', '# /* gap */ [allow(dead_code)]',
     '# ! [allow(dead_code)]', '# /* gap */ ! /* gap */ [allow(dead_code)]'].each do |attribute|
      with_fixture do |root|
        source = "mod one { #{attribute} impl Foo {} }\n"
        add_code_evidence(root, source, 'Foo', 'rust_impl', 1, 1)
        out, err, result = cli(root)
        assert_equal 0, result.exitstatus, out + err
      end
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

  def install_current(root, locate: true)
    catalog = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::CATALOG_PATH)))
    manifest = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::MANIFEST_PATH)))
    [catalog, manifest].each do |document|
      document['baseline_commit'] = git(root, 'rev-parse', 'HEAD')
      document['role'] = 'current-source-audit'
    end
    catalog['architecture'] = []
    manifest['files'].each do |file|
      file['sha256'] = Digest::SHA256.file(File.join(root, file['path'])).hexdigest
      file['architecture_ids'] = []
    end
    manifest['evidence'].each do |entry|
      if locate
        located = ArchitectureDocs::RustEvidence.locate(File.binread(File.join(root, entry['path'])), entry['symbol'], entry['kind'])
        %w[symbol_sha256 start_line end_line].each { |field| entry[field] = located[field] }
      end
      entry['audit_domains'] = ['business']
      entry['dependencies'] = []
    end
    json(root, 'docs/push-system/push-current-capability-catalog.v1.json', catalog)
    manifest['bindings'] = [ArchitectureDocs::Catalog::CATALOG_PATH, ArchitectureDocs::Catalog::MANIFEST_PATH,
                            'docs/push-system/push-current-capability-catalog.v1.json'].map do |path|
      { 'path' => path, 'sha256' => Digest::SHA256.file(File.join(root, path)).hexdigest }
    end
    json(root, 'docs/push-system/push-current-evidence-manifest.v1.json', manifest)
  end

  def bind_current(root)
    mutate(root, 'push-current-evidence-manifest.v1.json') do |manifest|
      manifest['bindings'].each { |entry| entry['sha256'] = Digest::SHA256.file(File.join(root, entry['path'])).hexdigest }
    end
  end

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
    install_current(root)
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
    install_current(root, locate: false)
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
      install_current(root)
      yield root
    end
  end
end
