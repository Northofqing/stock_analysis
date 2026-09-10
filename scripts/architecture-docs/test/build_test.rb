#!/usr/bin/env ruby
# frozen_string_literal: true

require 'base64'
require 'digest'
require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require_relative '../html_builder'
require_relative 'support/catalog_fixture'
require_relative 'support/document_check_fixture'

BUILD = File.expand_path('../build.rb', __dir__)

class ArchitectureDocsBuildTest < Minitest::Test
  def test_blueprint_is_a_supported_closed_target
    with_fixture do |root, _markdown|
      error = assert_raises(ArchitectureDocs::HtmlBuilder::Invalid) do
        ArchitectureDocs::HtmlBuilder.build(root, 'blueprint')
      end

      refute_equal 'target_unknown', error.message
    end
  end

  def test_blueprint_build_embeds_its_exact_source_and_target_metadata
    DocumentCheckFixture.with_fixture do |root|
      source = File.binread(File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.md'))
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      File.delete(output)
      refute File.exist?(output)

      stdout, stderr, result = run_build(root, 'blueprint', '--draft')

      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal "html_written target=blueprint status=PROVISIONAL\n", stdout
      assert_empty stderr
      html = File.binread(output)
      metadata = JSON.parse(html.match(%r{<script id="build-metadata" type="application/json">(.*?)</script>}m)[1])
      assert_equal 'blueprint', metadata.fetch('target')
      assert_equal 'docs/architecture/current/Project_Architecture_Blueprint.md', metadata.dig('source', 'path')
      assert_equal Digest::SHA256.hexdigest(source), metadata.dig('source', 'sha256')
      assert_includes html, Base64.strict_encode64(source)
      assert_includes html, '<a href="../../Project_Architecture_Blueprint.md#L1833">A.1 61 个 src/lib.rs 顶层模块</a>'.b
    end
  end

  def test_draft_build_creates_offline_html_with_source_provenance
    with_fixture do |root, markdown|
      output = File.join(root, 'docs/push-system/push-system-implementation-rfc.html')
      refute File.exist?(output)

      stdout, stderr, result = run_build(root, 'rfc', '--draft')

      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal "html_written target=rfc status=PROVISIONAL\n", stdout
      assert_empty stderr
      html = File.binread(output).force_encoding(Encoding::UTF_8)
      assert_includes html, '<h1 id="中文标题">中文标题</h1>'
      assert_includes html, Digest::SHA256.hexdigest(markdown)
      assert_includes html, Base64.strict_encode64(markdown)
    end
  end

  def test_cli_has_a_closed_target_and_option_surface
    with_fixture do |root, _markdown|
      invalid = [
        [], ['unknown'], ['rfc', 'extra'], ['rfc', 'rfc'],
        ['rfc', '--root', root, '--root', root],
        ['rfc', '--draft', '--draft'], ['rfc', '--check', '--check'],
        ['rfc', '--check', '--draft', '--unknown'], ['--all', 'rfc'],
        ['--all', '--all']
      ]
      invalid.each do |args|
        stdout, stderr, result = Open3.capture3(RbConfig.ruby, BUILD, *args)
        assert_equal 2, result.exitstatus, "args=#{args.inspect}\n#{stdout}#{stderr}"
        assert_empty stdout
        assert_includes stderr, 'Usage: build.rb rfc|blueprint'
        refute_includes stderr, 'backtrace'
      end

      stdout, stderr, result = Open3.capture3(RbConfig.ruby, BUILD, '--help')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_includes stdout, 'Usage: build.rb'
      assert_empty stderr

      stdout, stderr, result = run_build(root, 'rfc')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal "html_status_provisional target=rfc\n", stdout
      assert_empty stderr
      refute File.exist?(File.join(root, 'docs/push-system/push-system-implementation-rfc.html'))

      stdout, stderr, result = run_build(root, '--all', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert stdout.start_with?("html_written target=rfc status=PROVISIONAL\n"), stdout
      assert_includes stdout, 'target=blueprint'
      assert_empty stderr
    end
  end

  def test_all_builds_in_fixed_order_and_check_aggregates_target_failures_read_only
    with_dual_fixture do |root|
      outputs = %w[docs/push-system/push-system-implementation-rfc.html
                   docs/architecture/current/Project_Architecture_Blueprint.html].map { |path| File.join(root, path) }
      outputs.each { |path| File.delete(path) }

      stdout, stderr, result = run_build(root, '--all', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal ["html_written target=rfc status=PROVISIONAL\n",
                    "html_written target=blueprint status=PROVISIONAL\n"], stdout.lines
      assert_empty stderr

      outputs.each { |path| File.binwrite(path, "stale #{File.basename(path)}") }
      before = outputs.map { |path| [File.binread(path), File.mtime(path)] }
      stdout, stderr, result = run_build(root, '--all', '--check', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal ["html_stale target=rfc\n", "html_stale target=blueprint\n"], stdout.lines
      assert_empty stderr
      assert_equal before, outputs.map { |path| [File.binread(path), File.mtime(path)] }
    end
  end

  def test_single_targets_do_not_modify_each_others_outputs
    with_dual_fixture do |root|
      rfc = File.join(root, 'docs/push-system/push-system-implementation-rfc.html')
      blueprint = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      rfc_before = [File.binread(rfc), File.mtime(rfc)]
      File.delete(blueprint)

      stdout, stderr, result = run_build(root, 'rfc', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      refute File.exist?(blueprint)
      assert_equal rfc_before, [File.binread(rfc), File.mtime(rfc)]

      stdout, stderr, result = run_build(root, 'blueprint', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert File.file?(blueprint)
      assert_equal rfc_before, [File.binread(rfc), File.mtime(rfc)]
    end
  end

  def test_blueprint_source_identity_rejects_malformed_or_stale_declarations_before_writing
    with_dual_fixture do |root|
      source = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.md')
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      original = File.binread(source)
      declaration = original[/```architecture-source-v1\n(.*?)\n```/m, 1]
      mutations = {
        'missing' => original.sub('architecture-source-v1', 'architecture-source-v0'),
        'unclosed' => original.sub("\n```\n\n## 1.", "\n\n## 1."),
        'duplicate' => original.sub("\n## 1.", "\n```architecture-source-v1\n#{declaration}\n```\n\n## 1."),
        'json' => original.sub('"schema_version": 1', '"schema_version":'),
        'not object' => original.sub(declaration, '[]'),
        'extra field' => original.sub('"schema_version": 1,', '"schema_version": 1, "extra": true,'),
        'nested field' => original.sub('"path": "docs/push-system/push-current-capability-catalog.v1.json",',
                                       '"path": "docs/push-system/push-current-capability-catalog.v1.json", "extra": true,'),
        'schema' => original.sub('"schema_version": 1', '"schema_version": 2'),
        'status' => original.sub('"status": "PROVISIONAL"', '"status": "FINAL"'),
        'role' => original.sub('"role": "current-source-audit"', '"role": "historical"'),
        'path' => original.sub('docs/push-system/push-current-capability-catalog.v1.json', 'docs/push-system/other.json'),
        'manifest path' => original.sub('docs/push-system/push-current-evidence-manifest.v1.json', 'docs/push-system/other.json'),
        'pin' => original.sub(/"baseline_commit": "[0-9a-f]{40}"/, '"baseline_commit": "0000000000000000000000000000000000000000"'),
        'sha' => original.sub(/"sha256": "[0-9a-f]{64}"/, '"sha256": "0000000000000000000000000000000000000000000000000000000000000000"')
      }
      mutations.each do |label, bytes|
        File.binwrite(source, bytes)
        File.delete(output) if File.exist?(output)
        stdout, stderr, result = run_build(root, 'blueprint', '--draft')
        assert_equal 1, result.exitstatus, "#{label}: #{stdout}#{stderr}"
        assert_includes stdout, 'blueprint_source_identity', label
        assert_empty stderr
        refute File.exist?(output), label
      end
    ensure
      File.binwrite(source, original) if source && original
    end
  end

  def test_current_source_evolution_invalidates_blueprint_declaration_before_writing
    with_dual_fixture do |root|
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      File.delete(output)
      catalog_path = File.join(root, 'docs/push-system/push-current-capability-catalog.v1.json')
      catalog = JSON.parse(File.binread(catalog_path))
      catalog['scope'] += ' evolved'
      File.binwrite(catalog_path, JSON.pretty_generate(catalog) + "\n")
      manifest_path = File.join(root, 'docs/push-system/push-current-evidence-manifest.v1.json')
      manifest = JSON.parse(File.binread(manifest_path))
      binding = manifest['bindings'].find { |entry| entry['path'] == 'docs/push-system/push-current-capability-catalog.v1.json' }
      binding['sha256'] = Digest::SHA256.file(catalog_path).hexdigest
      File.binwrite(manifest_path, JSON.pretty_generate(manifest) + "\n")

      stdout, stderr, result = run_build(root, 'blueprint', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal "blueprint_source_identity_mismatch target=blueprint\n", stdout
      assert_empty stderr
      refute File.exist?(output)
    end
  end

  def test_blueprint_check_can_be_stale_while_rfc_is_current
    with_dual_fixture do |root|
      source = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.md')
      File.open(source, 'ab') { |file| file.write("\nbody drift\n") }

      stdout, stderr, result = run_build(root, 'rfc', '--check', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal "html_current target=rfc status=PROVISIONAL\n", stdout

      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      before = [File.binread(output), File.mtime(output)]
      stdout, stderr, result = run_build(root, 'blueprint', '--check', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal "html_stale target=blueprint\n", stdout
      assert_empty stderr
      assert_equal before, [File.binread(output), File.mtime(output)]
    end
  end

  def test_blueprint_rejects_source_output_and_parent_links_without_writing
    with_dual_fixture do |root|
      source = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.md')
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      source_saved = source + '.saved'
      File.rename(source, source_saved)
      File.symlink(source_saved, source)
      before = [File.binread(output), File.mtime(output)]
      assert_cli_failure(root, 'html_source_path_invalid', 'blueprint', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end

    with_dual_fixture do |root|
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      saved = output + '.saved'
      File.rename(output, saved)
      File.symlink(saved, output)
      assert_cli_failure(root, 'html_output_path_invalid', 'blueprint', '--draft')
      assert_equal File.binread(saved), File.binread(output)
    end

    with_dual_fixture do |root|
      output = File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html')
      saved = output + '.saved'
      File.rename(output, saved)
      File.link(saved, output)
      assert_cli_failure(root, 'html_output_hardlink_invalid', 'blueprint', '--draft')
      assert_equal 2, File.stat(output).nlink
    end

    with_dual_fixture do |root|
      directory = File.join(root, 'docs/architecture/current')
      saved = directory + '.saved'
      File.rename(directory, saved)
      File.symlink(saved, directory)
      assert_cli_failure(root, 'html_source_path_invalid', 'blueprint', '--draft')
      refute File.exist?(File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.html.tmp'))
    end
  end

  def test_blueprint_invalid_roots_fail_without_backtraces
    Dir.mktmpdir('blueprint-roots-') do |parent|
      missing = File.join(parent, 'missing')
      regular = File.join(parent, 'regular')
      link = File.join(parent, 'link')
      File.binwrite(regular, 'file')
      File.symlink(parent, link)
      [[missing, 'html_root_missing'], [regular, 'html_root_invalid'], [link, 'html_root_path_invalid']].each do |root, code|
        stdout, stderr, result = run_build(root, 'blueprint', '--draft')
        assert_equal 1, result.exitstatus, stdout + stderr
        assert_equal "#{code} target=blueprint\n", stdout
        assert_empty stderr
        refute_includes stdout + stderr, 'backtrace'
      end
    end
  end

  def test_blueprint_wrapper_preserves_help_and_provisional_failure
    wrapper = File.expand_path('../../render-architecture-blueprint-html.rb', __dir__)
    Dir.mktmpdir('blueprint-wrapper-') do |root|
      [['--help'], ['--root', root]].each do |arguments|
        expected = Open3.capture3(RbConfig.ruby, BUILD, 'blueprint', *arguments)
        actual = Open3.capture3(RbConfig.ruby, wrapper, *arguments)
        assert_equal expected.map { |value| value.respond_to?(:exitstatus) ? value.exitstatus : value },
                     actual.map { |value| value.respond_to?(:exitstatus) ? value.exitstatus : value }
      end
    end
    with_dual_fixture do |root|
      expected = Open3.capture3(RbConfig.ruby, BUILD, 'blueprint', '--check', '--draft', '--root', root)
      actual = Open3.capture3(RbConfig.ruby, wrapper, '--check', '--draft', '--root', root)
      assert_equal expected.map { |value| value.respond_to?(:exitstatus) ? value.exitstatus : value },
                   actual.map { |value| value.respond_to?(:exitstatus) ? value.exitstatus : value }
    end
  end

  def test_build_commands_preserve_the_frozen_blueprint_sentinel
    with_dual_fixture do |root|
      sentinel = File.join(root, 'docs/Project_Architecture_Blueprint.md')
      fixed_time = Time.at(946_684_800)
      File.binwrite(sentinel, "frozen historical blueprint\n")
      File.utime(fixed_time, fixed_time, sentinel)
      before = [File.binread(sentinel), File.mtime(sentinel)]
      commands = [
        ['rfc', '--draft'],
        ['blueprint', '--draft'],
        ['--all', '--check', '--draft']
      ]
      commands.each do |arguments|
        stdout, stderr, result = run_build(root, *arguments)
        assert_equal 0, result.exitstatus, stdout + stderr
        assert_equal before, [File.binread(sentinel), File.mtime(sentinel)]
      end
    end
  end

  def test_build_is_deterministic_and_check_is_strictly_read_only
    with_fixture do |root, _markdown|
      output = File.join(root, 'docs/push-system/push-system-implementation-rfc.html')

      stdout, stderr, result = run_build(root, 'rfc', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      first = File.binread(output)
      fixed_time = Time.at(946_684_800)
      File.utime(fixed_time, fixed_time, output)

      stdout, stderr, result = run_build(root, 'rfc', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal "html_current target=rfc status=PROVISIONAL\n", stdout
      assert_empty stderr
      assert_equal first, File.binread(output)
      assert_equal fixed_time, File.mtime(output)

      stdout, stderr, result = run_build(root, 'rfc', '--check', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_equal "html_current target=rfc status=PROVISIONAL\n", stdout
      assert_empty stderr
      assert_equal fixed_time, File.mtime(output)

      File.binwrite(output, 'stale')
      stale_snapshot = [File.binread(output), File.mtime(output)]
      stdout, stderr, result = run_build(root, 'rfc', '--check', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal "html_stale target=rfc\n", stdout
      assert_empty stderr
      assert_equal stale_snapshot, [File.binread(output), File.mtime(output)]

      File.unlink(output)
      stdout, stderr, result = run_build(root, 'rfc', '--check', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_equal "html_missing target=rfc\n", stdout
      assert_empty stderr
      refute File.exist?(output)

    end
  end

  def test_check_detects_source_and_template_changes_without_writing
    with_fixture do |root, _markdown|
      output = output_path(root)
      assert_successful_build(root)

      File.binwrite(source_path(root), "# 已变化\n")
      before = [File.binread(output), File.mtime(output)]
      assert_cli_failure(root, 'html_stale', 'rfc', '--check', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end

    with_fixture do |root, _markdown|
      output = output_path(root)
      assert_successful_build(root)

      File.open(template_path(root), 'ab') { |file| file.write("<!-- changed -->\n") }
      before = [File.binread(output), File.mtime(output)]
      assert_cli_failure(root, 'html_stale', 'rfc', '--check', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end
  end

  def test_html_records_implementation_hashes_and_check_detects_code_changes
    with_fixture do |root, _markdown|
      fixture_build = install_builder_sources(root)
      stdout, stderr, result = run_cli(fixture_build, root, 'rfc', '--draft')
      assert_equal 0, result.exitstatus, stdout + stderr
      assert_empty stderr

      html = File.binread(output_path(root))
      %w[build.rb html_builder.rb markdown_renderer.rb].each do |name|
        path = File.join(root, 'scripts/architecture-docs', name)
        assert_includes html, Digest::SHA256.hexdigest(File.binread(path)), name
      end

      renderer = File.join(root, 'scripts/architecture-docs/markdown_renderer.rb')
      File.open(renderer, 'ab') { |file| file.write("\n# changed fixture implementation\n") }
      before = [File.binread(output_path(root)), File.mtime(output_path(root))]
      stdout, stderr, result = run_cli(fixture_build, root, 'rfc', '--check', '--draft')
      assert_equal 1, result.exitstatus, stdout + stderr
      assert_includes stdout, 'html_stale'
      assert_empty stderr
      assert_equal before, [File.binread(output_path(root)), File.mtime(output_path(root))]
    end
  end

  def test_draft_build_rejects_mermaid_asset_byte_drift_without_overwriting
    with_fixture do |root, _markdown|
      assert_successful_build(root)
      output = output_path(root)
      before = [File.binread(output), File.mtime(output)]
      File.open(asset_path(root, 'mermaid.min.js'), 'ab') { |file| file.write('changed') }

      assert_cli_failure(root, 'mermaid_asset_bytes_mismatch', 'rfc', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end

    with_fixture do |root, _markdown|
      script_path = asset_path(root, 'mermaid.min.js')
      bytes = File.binread(script_path)
      bytes.setbyte(0, bytes.getbyte(0) ^ 1)
      File.binwrite(script_path, bytes)
      assert_cli_failure(root, 'mermaid_asset_sha_mismatch', 'rfc', '--draft')
    end
  end

  def test_html_embeds_verified_mermaid_bytes_and_provenance
    with_fixture do |root, _markdown|
      assert_successful_build(root)
      html = File.binread(output_path(root))
      script = File.binread(asset_path(root, 'mermaid.min.js'))
      license = File.binread(asset_path(root, 'mermaid.LICENSE'))

      assert_includes html, script
      assert_includes html, license
      assert_includes html, '11.17.2'
      assert_includes html, Digest::SHA256.hexdigest(script)
      assert_includes html, Digest::SHA256.hexdigest(license)
    end
  end

  def test_mermaid_manifest_contract_and_files_are_enforced
    cases = [
      ['mermaid_manifest_structure_invalid', proc { |m| m['extra'] = true }],
      ['mermaid_manifest_schema_version_invalid', proc { |m| m['schema_version'] = 2 }],
      ['mermaid_manifest_version_invalid', proc { |m| m['version'] = '11.17.1' }],
      ['mermaid_manifest_package_integrity_invalid', proc { |m| m['package_integrity'] = 'sha512-wrong' }],
      ['mermaid_manifest_files_invalid', proc { |m| m['files'].pop }],
      ['mermaid_manifest_file_structure_invalid', proc { |m| m['files'][0]['extra'] = true }],
      ['mermaid_manifest_file_invalid', proc { |m| m['files'][0]['path'] = '../mermaid.min.js' }],
      ['mermaid_manifest_file_invalid', proc { |m| m['files'][0]['bytes'] += 1 }],
      ['mermaid_manifest_file_invalid', proc { |m| m['files'][0]['sha256'] = '0' * 64 }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root, _markdown|
        assert_successful_build(root)
        before = [File.binread(output_path(root)), File.mtime(output_path(root))]
        mutate_mermaid_manifest(root, &mutation)
        assert_cli_failure(root, code, 'rfc', '--draft')
        assert_equal before, [File.binread(output_path(root)), File.mtime(output_path(root))]
      end
    end

    with_fixture do |root, _markdown|
      assert_successful_build(root)
      File.unlink(asset_path(root, 'mermaid.LICENSE'))
      assert_cli_failure(root, 'mermaid_asset_missing', 'rfc', '--draft')
    end

    with_fixture do |root, _markdown|
      File.binwrite(asset_path(root, 'mermaid-manifest.v1.json'), '{bad')
      assert_cli_failure(root, 'mermaid_manifest_json_invalid', 'rfc', '--draft')
    end
  end

  def test_draft_does_not_bypass_frozen_input_or_utf8_source_validation
    with_fixture do |root, _markdown|
      assert_successful_build(root)
      before = [File.binread(output_path(root)), File.mtime(output_path(root))]
      File.binwrite(File.join(root, 'docs/input.bin'), "changed")
      assert_cli_failure(root, 'input_bytes_mismatch', 'rfc', '--draft')
      assert_equal before, [File.binread(output_path(root)), File.mtime(output_path(root))]
    end

    with_fixture do |root, _markdown|
      File.binwrite(source_path(root), "# invalid \xff\n".b)
      assert_cli_failure(root, 'html_source_encoding_invalid', 'rfc', '--draft')
      refute File.exist?(output_path(root))
    end

    with_fixture do |root, _markdown|
      File.binwrite(template_path(root), "template \xff".b)
      assert_cli_failure(root, 'html_template_encoding_invalid', 'rfc', '--draft')
    end

  end

  def test_source_template_and_assets_reject_symlinks_and_non_files
    [
      [proc { |root| source_path(root) }, 'html_source_path_invalid'],
      [proc { |root| template_path(root) }, 'html_template_path_invalid'],
      [proc { |root| asset_path(root, 'mermaid.min.js') }, 'mermaid_asset_path_invalid'],
      [proc { |root| asset_path(root, 'mermaid.LICENSE') }, 'mermaid_asset_path_invalid'],
      [proc { |root| asset_path(root, 'mermaid-manifest.v1.json') }, 'mermaid_manifest_path_invalid']
    ].each do |path_fn, code|
      with_fixture do |root, _markdown|
        path = path_fn.call(root)
        saved = path + '.saved'
        File.rename(path, saved)
        File.symlink(saved, path)
        assert_cli_failure(root, code, 'rfc', '--draft')
        refute File.exist?(output_path(root))
      end

      with_fixture do |root, _markdown|
        path = path_fn.call(root)
        File.unlink(path)
        Dir.mkdir(path)
        code = code.sub('path_invalid', 'not_regular')
        assert_cli_failure(root, code, 'rfc', '--draft')
      end
    end
  end

  def test_source_template_asset_and_output_parent_symlinks_are_rejected_without_writes
    with_fixture do |root, _markdown|
      directory = File.join(root, 'docs/push-system')
      saved = replace_directory_with_symlink(directory)

      # The source and frozen-input manifest share this parent. The manifest is
      # checked first, so its rejection proves traversal stopped before source
      # or output processing and no output was created through the link.
      assert_cli_failure(root, 'manifest_path_invalid', 'rfc', '--draft')
      refute File.exist?(File.join(saved, 'push-system-implementation-rfc.html'))
    end

    with_fixture do |root, _markdown|
      assert_successful_build(root)
      directory = File.join(root, 'docs/push-system')
      saved = replace_directory_with_symlink(directory)
      output = File.join(saved, 'push-system-implementation-rfc.html')
      before = [File.binread(output), File.mtime(output)]

      # Output has the same fixed parent as the source and manifest. A second
      # formal CLI case verifies an existing output is not touched.
      assert_cli_failure(root, 'manifest_path_invalid', 'rfc', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end

    with_fixture do |root, _markdown|
      assert_successful_build(root)
      output = output_path(root)
      before = [File.binread(output), File.mtime(output)]
      replace_directory_with_symlink(File.join(root, 'scripts/architecture-docs/templates'))

      assert_cli_failure(root, 'html_template_path_invalid', 'rfc', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end

    with_fixture do |root, _markdown|
      assert_successful_build(root)
      output = output_path(root)
      before = [File.binread(output), File.mtime(output)]
      replace_directory_with_symlink(File.join(root, 'scripts/architecture-docs/assets'))

      assert_cli_failure(root, 'mermaid_manifest_path_invalid', 'rfc', '--draft')
      assert_equal before, [File.binread(output), File.mtime(output)]
    end
  end

  def test_output_rejects_symlinks_hardlinks_and_non_files_without_touching_targets
    with_fixture do |root, _markdown|
      target = File.join(root, 'unrelated')
      File.binwrite(target, 'keep')
      File.symlink(target, output_path(root))
      assert_cli_failure(root, 'html_output_path_invalid', 'rfc', '--draft')
      assert_equal 'keep', File.binread(target)
    end

    with_fixture do |root, _markdown|
      target = File.join(root, 'unrelated')
      File.binwrite(target, 'keep')
      File.link(target, output_path(root))
      assert_cli_failure(root, 'html_output_hardlink_invalid', 'rfc', '--draft')
      assert_equal 'keep', File.binread(target)
    end

    with_fixture do |root, _markdown|
      Dir.mkdir(output_path(root))
      assert_cli_failure(root, 'html_output_not_regular', 'rfc', '--draft')
    end
  end

  def test_markdown_blocks_inline_text_and_links_render_safely
    markdown = <<~'MARKDOWN'
      # Alpha
      ## Duplicate
      ## Duplicate
      ###### Deep

      paragraph first
      second with `multi
      line` and **bold** plus data/** and <SourceRef> and &#215; and &#60;script&#62;.

      - one
      - two
      1. alpha
      2. beta

      > quote
      > next

      [relative](docs/a.md) [fragment](#alpha) [secure](https://example.com/a?x=1&y=2)
      [bad](javascript:alert(1)) ![image](https://example.com/x.png)
      <div onclick="bad()">raw</div>
      ~~unsupported stays visible~~

      <!-- specification comment must not render -->
    MARKDOWN
    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)
      rendered = document_body(html)

      assert_includes rendered, '<h1 id="alpha">Alpha</h1>'
      assert_includes rendered, '<h2 id="duplicate">Duplicate</h2>'
      assert_includes rendered, '<h2 id="duplicate-2">Duplicate</h2>'
      assert_includes rendered, '<h6 id="deep">Deep</h6>'
      assert_includes rendered, "paragraph first\nsecond with <code>multi\nline</code>"
      assert_includes rendered, '<strong>bold</strong>'
      assert_includes rendered, 'data/**'
      assert_includes rendered, '&lt;SourceRef&gt;'
      assert_includes rendered, '&#215;'
      refute_includes rendered, '&amp;#215;'
      assert_includes rendered, '&#60;script&#62;'
      refute_includes rendered, '<script>.'
      assert_includes rendered, '<ul><li>one</li><li>two</li></ul>'
      assert_includes rendered, '<ol><li>alpha</li><li>beta</li></ol>'
      assert_includes rendered, "<blockquote>quote\nnext</blockquote>"
      assert_includes rendered, '<a href="docs/a.md">relative</a>'
      assert_includes rendered, '<a href="#alpha">fragment</a>'
      assert_includes rendered, '<a href="https://example.com/a?x=1&amp;y=2">secure</a>'
      refute_includes rendered.downcase, 'href="javascript:'
      assert_includes rendered, '[bad](javascript:alert(1))'
      assert_includes rendered, '![image](https://example.com/x.png)'
      refute_includes rendered, '<img'
      assert_includes rendered, '&lt;div onclick=&quot;bad()&quot;&gt;raw&lt;/div&gt;'
      assert_includes rendered, '~~unsupported stays visible~~'
      refute_includes rendered, 'specification comment must not render'
    end
  end

  def test_inline_code_protects_comment_markers_and_unclosed_comments_stay_literal
    markdown = <<~'MARKDOWN'
      # Comment and code boundaries

      before `<!--` after
      next line

      cross `left <!--
      right` done

      double ``one ` <!-- two`` after

      visible <!-- closed comment --> suffix

      unclosed <!-- literal marker
      following line remains
    MARKDOWN
    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      rendered = document_body(File.binread(output_path(root)).force_encoding(Encoding::UTF_8))

      assert_includes rendered, "before <code>&lt;!--</code> after\nnext line"
      assert_includes rendered, "cross <code>left &lt;!--\nright</code> done"
      assert_includes rendered, 'double <code>one ` &lt;!-- two</code> after'
      assert_includes rendered, 'visible  suffix'
      refute_includes rendered, 'closed comment'
      assert_includes rendered, "unclosed &lt;!-- literal marker\nfollowing line remains"
    end
  end

  def test_code_span_state_resets_at_blank_paragraph_boundaries_without_breaking_comments
    markdown = [
      '# Paragraph boundaries',
      '',
      'first `unclosed code marker',
      '',
      '<!-- hidden after empty line -->',
      'visible after empty line',
      '',
      'second `unclosed code marker',
      " \t ",
      '<!-- hidden after whitespace line -->',
      'visible after whitespace line',
      '',
      'same paragraph `left <!--',
      'right` remains code',
      '',
      'before <!-- spanning comment',
      '',
      'still hidden --> after',
      '',
      'unclosed <!-- literal comment marker',
      'following literal line'
    ].join("\n") + "\n"

    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      rendered = document_body(File.binread(output_path(root)).force_encoding(Encoding::UTF_8))

      assert_includes rendered, 'first `unclosed code marker'
      assert_includes rendered, 'visible after empty line'
      refute_includes rendered, 'hidden after empty line'
      assert_includes rendered, 'second `unclosed code marker'
      assert_includes rendered, 'visible after whitespace line'
      refute_includes rendered, 'hidden after whitespace line'
      assert_includes rendered, "same paragraph <code>left &lt;!--\nright</code> remains code"
      assert_includes rendered, 'before '
      assert_includes rendered, ' after'
      refute_includes rendered, 'spanning comment'
      refute_includes rendered, 'still hidden'
      assert_includes rendered, "unclosed &lt;!-- literal comment marker\nfollowing literal line"
    end
  end

  def test_tables_fences_and_isolated_table_shaped_lines_preserve_content
    markdown = <<~'MARKDOWN'
      # Structured

      | Name | escaped \| pipe | `code|pipe` |
      | --- | :---: | ---: |
      | a | b\|c | `x|y` |
      | slash | C:\tmp | literal |

      | AwaitingAuthority | SameState | visible orphan one |
      | AwaitingFinalizer | SameState | visible orphan two |

      ```json
      {
        "value": "</script>",
        "spaces": "  keep  "
      }
      ```

      ```sql
      SELECT  *
        FROM table_name;
      ```
    MARKDOWN
    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)
      rendered = document_body(html)

      assert_equal 1, rendered.scan('<table>').length
      assert_equal 3, rendered.scan('<th>').length
      assert_equal 6, rendered.scan('<td>').length
      assert_includes rendered, '<th>escaped | pipe</th>'
      assert_includes rendered, '<th><code>code|pipe</code></th>'
      assert_includes rendered, '<td>b|c</td>'
      assert_includes rendered, '<td><code>x|y</code></td>'
      assert_includes rendered, '<td>C:\tmp</td>'
      assert_includes rendered, '| AwaitingAuthority | SameState | visible orphan one |'
      assert_includes rendered, '| AwaitingFinalizer | SameState | visible orphan two |'
      assert_includes rendered, %(<pre><code class="language-json">{\n  &quot;value&quot;: &quot;&lt;/script&gt;&quot;,\n  &quot;spaces&quot;: &quot;  keep  &quot;\n}\n</code></pre>)
      assert_includes rendered, %(<pre><code class="language-sql">SELECT  *\n  FROM table_name;\n</code></pre>)
      refute_includes rendered, '<script>"'
    end
  end

  def test_numeric_entities_keep_text_semantics_but_remain_literal_in_code_and_diagram_source
    markdown = <<~'MARKDOWN'
      # Entity contexts

      body &#91;
      inline `&#91;`

      ```sql
      SELECT '&#91;';
      ```

      ```mermaid
      flowchart LR
        A["&#91;"] --> B
      ```
    MARKDOWN
    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      rendered = document_body(File.binread(output_path(root)).force_encoding(Encoding::UTF_8))

      assert_includes rendered, 'body &#91;'
      assert_includes rendered, 'inline <code>&amp;#91;</code>'
      assert_includes rendered, "<pre><code class=\"language-sql\">SELECT &#39;&amp;#91;&#39;;\n</code></pre>"
      assert_includes rendered, 'class="mermaid-source"><code>flowchart LR'
      assert_includes rendered, 'A[&quot;&amp;#91;&quot;] --&gt; B'
    end
  end

  def test_heading_ids_are_unique_even_when_a_later_base_matches_an_earlier_suffix
    markdown = "# A\n# A\n# A-2\n# A\n# A-2\n"
    with_fixture(markdown: markdown) do |root, _source|
      assert_successful_build(root)
      rendered = document_body(File.binread(output_path(root)).force_encoding(Encoding::UTF_8))
      ids = rendered.scan(/<h1 id="([^"]+)">/).flatten

      assert_equal %w[a a-2 a-2-2 a-3 a-2-3], ids
      assert_equal ids.length, ids.uniq.length
    end
  end

  def test_repository_template_is_self_contained_and_exposes_page_controls
    repository_template = File.binread(File.expand_path('../templates/document.html.erb', __dir__))
    with_fixture(template: repository_template) do |root, source|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)

      assert_includes html, 'data-status="PROVISIONAL"'
      assert_includes html, 'id="doc-search"'
      assert_includes html, 'id="theme-toggle"'
      assert_includes html, 'id="collapse-all"'
      assert_includes html, 'id="expand-all"'
      assert_includes html, 'id="print-document"'
      assert_includes html, 'id="table-of-contents"'
      assert_includes html, 'id="document-content"'
      assert_includes html, 'id="build-metadata"'
      assert_includes html, 'id="markdown-source"'
      assert_includes html, Base64.strict_encode64(source)
      assert_includes html, '<style>'
      assert_includes html, '<script>'
      assert_includes html, 'mermaid.initialize'
      assert_includes html, "secure: ['secure', 'securityLevel', 'startOnLoad', 'maxTextSize', 'suppressErrorRendering', 'maxEdges', 'htmlLabels', 'flowchart']"
      assert_includes html, "source.replace(/\\bxlink:href\\s*=/gi, 'href=')"
      assert_includes html, "new DOMParser().parseFromString(normalized, 'image/svg+xml')"
      assert_includes html, "querySelectorAll('script,img,image,foreignObject,iframe,object,embed')"
      assert_includes html, "querySelectorAll('a').forEach(anchor => anchor.replaceWith"
      assert_includes html, "['href', 'xlink:href', 'src', 'target'].includes(name)"
      assert_operator html.index('const safeSvg = sanitizeMermaidSvg(rendered.svg)'), :<,
                      html.index('target.innerHTML = safeSvg')
      assert_includes html, '@media print'
      assert_includes html, 'beforeprint'
      assert_includes html, 'requestFullscreen'
      assert_includes html, 'data-document-ready'
      refute_match(/<(?:script|link|img)\b[^>]*(?:src|href)=["']https?:/i, html)
      refute_match(/<link\b/i, html)
    end
  end

  def test_repository_template_metadata_and_embedded_source_are_exact
    repository_template = File.binread(File.expand_path('../templates/document.html.erb', __dir__))
    markdown = "# Metadata\n\ntext </script><script id=\"pwn\">bad()</script>\n"
    with_fixture(markdown: markdown, template: repository_template) do |root, source|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)
      metadata_json = html.match(%r{<script id="build-metadata" type="application/json">(.*?)</script>}m)[1]
      encoded_source = html.match(%r{<script id="markdown-source" type="application/octet-stream">(.*?)</script>}m)[1]
      metadata = JSON.parse(metadata_json)

      assert_equal 'PROVISIONAL', metadata['status']
      assert_equal source.bytesize, metadata.dig('source', 'bytes')
      assert_equal Digest::SHA256.hexdigest(source), metadata.dig('source', 'sha256')
      assert_equal Digest::SHA256.hexdigest(repository_template), metadata.dig('template', 'sha256')
      assert_equal source, Base64.strict_decode64(encoded_source)
      assert_equal '11.17.2', metadata.dig('mermaid', 'version')
      assert_equal 0, metadata['diagram_count']
      assert_includes metadata['publication_boundary'], 'not Implementation-Ready'
      refute_includes html, root
      refute_includes html, '<script id="pwn">'
      assert_includes html, '&lt;/script&gt;&lt;script id=&quot;pwn&quot;&gt;bad()&lt;/script&gt;'
    end
  end

  def test_real_rfc_regression_preserves_structure_and_original_bytes
    source = File.binread(File.expand_path('../../../docs/push-system/push-system-implementation-rfc.md', __dir__))
    repository_template = File.binread(File.expand_path('../templates/document.html.erb', __dir__))
    with_fixture(markdown: source, template: repository_template) do |root, original|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)
      rendered = document_body(html)

      assert_equal 117, rendered.scan(/<h[1-6]\b/).length
      assert_equal 54, rendered.scan('<table>').length
      assert_equal 2, rendered.scan(/<pre><code class="language-(?:json|sql)">/).length
      assert_equal 52, rendered.scan(/<h4\b/).length
      assert_includes rendered, "<code>prepare(RunContext) -&gt; PreparedFacts -&gt; project() -&gt; Ready(PreparedPush) -&gt;\n业务 intent"
      assert_includes rendered, 'Vec&lt;SourceRef&gt;'
      assert_includes rendered, 'D01_LAST_PUSH&#91;code:name&#93;'
      refute_includes rendered, 'D01_LAST_PUSH&amp;#91;'
      assert_includes rendered, 'data/**'
      assert_includes rendered, '| AwaitingAuthority | SameState | authority 恢复器 |'
      assert_includes rendered, '| AwaitingFinalizer | SameState | finalizer |'
      encoded = html.match(%r{<script id="markdown-source" type="application/octet-stream">(.*?)</script>}m)[1]
      assert_equal original, Base64.strict_decode64(encoded)
    end
  end

  def test_mermaid_fixture_covers_success_classes_and_visible_failure_source
    markdown = <<~'MARKDOWN'
      # Diagram fixture

      ```mermaid
      flowchart LR
        A --> B
      ```

      ```mermaid
      sequenceDiagram
        Alice->>Bob: hello
      ```

      ```mermaid
      stateDiagram-v2
        [*] --> Ready
      ```

      ```mermaid
      this is deliberately invalid mermaid
      ```
    MARKDOWN
    repository_template = File.binread(File.expand_path('../templates/document.html.erb', __dir__))
    with_fixture(markdown: markdown, template: repository_template) do |root, source|
      assert_successful_build(root)
      html = File.binread(output_path(root)).force_encoding(Encoding::UTF_8)
      metadata = JSON.parse(html.match(%r{<script id="build-metadata" type="application/json">(.*?)</script>}m)[1])

      assert_equal 4, html.scan('class="diagram"').length
      assert_equal 4, html.scan('data-diagram-state="pending"').length
      assert_equal 4, html.scan('class="mermaid-source"').length
      assert_equal 4, metadata['diagram_count']
      assert_includes html, 'flowchart LR'
      assert_includes html, 'sequenceDiagram'
      assert_includes html, 'stateDiagram-v2'
      assert_includes html, 'this is deliberately invalid mermaid'
      encoded = html.match(%r{<script id="markdown-source" type="application/octet-stream">(.*?)</script>}m)[1]
      assert_equal source, Base64.strict_decode64(encoded)
    end
  end

  def test_invalid_roots_and_invalid_templates_fail_without_stacktraces
    Dir.mktmpdir('architecture-docs-invalid-root') do |directory|
      missing = File.join(directory, 'missing')
      assert_cli_failure(missing, 'html_root_missing', 'rfc', '--draft')
      file = File.join(directory, 'file')
      File.binwrite(file, 'not a root')
      assert_cli_failure(file, 'html_root_invalid', 'rfc', '--draft')
      link = File.join(directory, 'link')
      File.symlink(directory, link)
      assert_cli_failure(link, 'html_root_path_invalid', 'rfc', '--draft')
    end

    with_fixture do |root, _source|
      File.binwrite(template_path(root), '<%= raise "boom" %>')
      assert_cli_failure(root, 'html_template_render_failed', 'rfc', '--draft')
      refute File.exist?(output_path(root))
    end
  end

  private

  def run_build(root, *args)
    run_cli(BUILD, root, *args)
  end

  def run_cli(executable, root, *args)
    Open3.capture3(RbConfig.ruby, executable, *args, '--root', root)
  end

  def install_builder_sources(root)
    source_dir = File.expand_path('..', __dir__)
    destination = File.join(root, 'scripts/architecture-docs')
    %w[build.rb catalog.rb current_audit.rb html_builder.rb markdown_renderer.rb rfc_inputs.rb
       rust_evidence.rb source_catalog.rb].each do |name|
      FileUtils.cp(File.join(source_dir, name), File.join(destination, name))
    end
    File.join(destination, 'build.rb')
  end

  def assert_successful_build(root)
    stdout, stderr, result = run_build(root, 'rfc', '--draft')
    assert_equal 0, result.exitstatus, stdout + stderr
    assert_empty stderr
  end

  def assert_cli_failure(root, code, *args)
    stdout, stderr, result = run_build(root, *args)
    assert_equal 1, result.exitstatus, stdout + stderr
    assert_includes stdout, code
    assert_empty stderr
  end

  def source_path(root)
    File.join(root, 'docs/push-system/push-system-implementation-rfc.md')
  end

  def output_path(root)
    File.join(root, 'docs/push-system/push-system-implementation-rfc.html')
  end

  def template_path(root)
    File.join(root, 'scripts/architecture-docs/templates/document.html.erb')
  end

  def document_body(html)
    article = html.match(%r{<article id="document-content">(.*?)</article>}m)
    return article[1] if article

    html.match(%r{<body>(.*?)<span>}m)[1]
  end

  def replace_directory_with_symlink(directory)
    saved = directory + '.saved'
    File.rename(directory, saved)
    File.symlink(saved, directory)
    saved
  end

  def asset_path(root, name)
    File.join(root, 'scripts/architecture-docs/assets', name)
  end

  def mutate_mermaid_manifest(root)
    path = asset_path(root, 'mermaid-manifest.v1.json')
    manifest = JSON.parse(File.binread(path))
    yield manifest
    File.binwrite(path, JSON.pretty_generate(manifest) + "\n")
  end

  def with_dual_fixture
    fixture = Object.new.extend(CatalogFixture)
    fixture.with_fixture do |root|
      FileUtils.mkdir_p(File.join(root, 'docs/architecture/current'))
      FileUtils.mkdir_p(File.join(root, 'scripts/architecture-docs/assets'))
      FileUtils.mkdir_p(File.join(root, 'scripts/architecture-docs/templates'))
      File.binwrite(source_path(root), "# 中文标题\n\n第一段。\n".encode(Encoding::UTF_8))
      File.binwrite(File.join(root, 'docs/input.bin'), "\x00".b)
      write_rfc_input_manifest(root)
      write_mermaid_fixture(root)
      template = "<!doctype html><html><body><%= body_html %><span><%= source_sha256 %></span><span><%= implementation_sha256.values.join %></span><span><%= mermaid_metadata.values.join %></span><pre><%= mermaid_license %></pre><script><%= mermaid_script %></script><script id=\"build-metadata\" type=\"application/json\"><%= build_metadata_json %></script><script id=\"markdown-source\" type=\"application/octet-stream\"><%= source_base64 %></script></body></html>\n"
      File.binwrite(template_path(root), template)
      File.binwrite(File.join(root, 'docs/architecture/current/Project_Architecture_Blueprint.md'), blueprint_source(root))
      stdout, stderr, result = run_build(root, '--all', '--draft')
      raise stdout + stderr unless result.success?

      yield root
    end
  end

  def blueprint_source(root)
    catalog_path = File.join(root, 'docs/push-system/push-current-capability-catalog.v1.json')
    manifest_path = File.join(root, 'docs/push-system/push-current-evidence-manifest.v1.json')
    catalog = JSON.parse(File.binread(catalog_path))
    declaration = {
      'schema_version' => 1,
      'status' => 'PROVISIONAL',
      'role' => 'current-source-audit',
      'baseline_commit' => catalog.fetch('baseline_commit'),
      'catalog' => {
        'path' => 'docs/push-system/push-current-capability-catalog.v1.json',
        'sha256' => Digest::SHA256.file(catalog_path).hexdigest
      },
      'manifest' => {
        'path' => 'docs/push-system/push-current-evidence-manifest.v1.json',
        'sha256' => Digest::SHA256.file(manifest_path).hexdigest
      }
    }
    "# Fixture Blueprint\n\n```architecture-source-v1\n#{JSON.pretty_generate(declaration)}\n```\n\n## 1. Current\n\n| A | B |\n| --- | --- |\n| 一 | 二 |\n"
  end

  def with_fixture(markdown: nil, template: nil)
    Dir.mktmpdir('architecture-docs-build-test') do |root|
      FileUtils.mkdir_p(File.join(root, 'docs/push-system'))
      FileUtils.mkdir_p(File.join(root, 'scripts/architecture-docs/assets'))
      FileUtils.mkdir_p(File.join(root, 'scripts/architecture-docs/templates'))

      markdown ||= "# 中文标题\n\n第一段。\n".encode(Encoding::UTF_8)
      File.binwrite(File.join(root, 'docs/push-system/push-system-implementation-rfc.md'), markdown)
      File.binwrite(File.join(root, 'docs/input.bin'), "\x00".b)
      write_rfc_input_manifest(root)
      write_mermaid_fixture(root)
      template ||= "<!doctype html><html><body><%= body_html %><span><%= source_sha256 %></span><span><%= implementation_sha256.values.join %></span><span><%= mermaid_metadata.values.join %></span><pre><%= mermaid_license %></pre><script><%= mermaid_script %></script><script id=\"markdown-source\" type=\"application/octet-stream\"><%= source_base64 %></script></body></html>\n"
      File.binwrite(File.join(root, 'scripts/architecture-docs/templates/document.html.erb'), template)

      yield root, markdown
    end
  end

  def write_rfc_input_manifest(root)
    manifest = {
      'schema_version' => 1,
      'status' => 'PROVISIONAL',
      'source_workspace' => 'root-worktree-snapshot',
      'captured_date' => '2026-09-06',
      'inputs' => [{
        'id' => 'fixture-input',
        'path' => 'docs/input.bin',
        'bytes' => 1,
        'sha256' => '6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d',
        'role' => 'binary fixture',
        'authority' => 'test snapshot',
        'conflicts' => []
      }]
    }
    path = File.join(root, 'docs/push-system/rfc-input-manifest.v1.json')
    File.binwrite(path, JSON.pretty_generate(manifest) + "\n")
  end

  def write_mermaid_fixture(root)
    source_root = File.expand_path('../assets', __dir__)
    asset_root = File.join(root, 'scripts/architecture-docs/assets')
    %w[mermaid.min.js mermaid.LICENSE mermaid-manifest.v1.json].each do |name|
      FileUtils.cp(File.join(source_root, name), File.join(asset_root, name))
    end
  end
end
