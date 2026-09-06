#!/usr/bin/env ruby
# frozen_string_literal: true

require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'

CHECK_RFC_INPUTS = File.expand_path('../check-rfc-inputs.rb', __dir__)

class RfcInputsTest < Minitest::Test
  def test_cli_accepts_frozen_binary_bytes_without_writing_inputs
    with_fixture do |root|
      before = snapshot(root)
      out, err, result = run_cli(root)
      assert_equal 0, result.exitstatus, out + err
      assert_equal "rfc_inputs_valid\n", out
      assert_empty err
      assert_equal before, snapshot(root)
    end
  end

  def test_cli_rejects_changed_bytes_and_changed_size_without_repairing_them
    with_fixture do |root|
      File.binwrite(File.join(root, 'docs/input.bin'), "\xff".b)
      before = snapshot(root)
      out, err, result = run_cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out, 'input_sha_mismatch'
      refute_includes out, 'input_bytes_mismatch'
      assert_equal before, snapshot(root)

      File.binwrite(File.join(root, 'docs/input.bin'), "\x00\x00".b)
      before = snapshot(root)
      out, err, result = run_cli(root)
      assert_equal 1, result.exitstatus, out + err
      assert_includes out, 'input_bytes_mismatch'
      assert_includes out, 'input_sha_mismatch'
      assert_equal before, snapshot(root)
    end
  end

  def test_cli_reports_missing_inputs_manifest_and_invalid_roots_without_backtraces
    with_fixture do |root|
      File.unlink(File.join(root, 'docs/input.bin'))
      assert_cli_error(root, 'input_missing')
      File.unlink(manifest_path(root))
      assert_cli_error(root, 'manifest_missing')
      assert_cli_error(File.join(root, 'missing'), 'root_missing')
      File.binwrite(File.join(root, 'file'), 'not a directory')
      assert_cli_error(File.join(root, 'file'), 'root_invalid')
    end
  end

  def test_cli_rejects_duplicate_input_ids_and_paths
    with_fixture do |root|
      mutate_manifest(root) { |manifest| manifest['inputs'] << manifest['inputs'].first.dup }
      assert_cli_error(root, 'duplicate_input_id')
      assert_cli_error(root, 'duplicate_input_path')
    end
  end

  def test_cli_rejects_invalid_manifest_contracts
    cases = [
      ['manifest_schema_invalid', proc { |m| m['schema_version'] = 2 }],
      ['manifest_schema_invalid', proc { |m| m['schema_version'] = 1.0 }],
      ['manifest_status_invalid', proc { |m| m['status'] = 'READY' }],
      ['manifest_source_workspace_invalid', proc { |m| m['source_workspace'] = '/machine/path' }],
      ['manifest_captured_date_invalid', proc { |m| m['captured_date'] = '2026-09-05' }],
      ['manifest_fields_invalid', proc { |m| m.delete('status') }],
      ['manifest_fields_invalid', proc { |m| m['extra'] = true }],
      ['manifest_inputs_invalid', proc { |m| m['inputs'] = [] }],
      ['manifest_inputs_invalid', proc { |m| m['inputs'] = 'bad' }],
      ['input_structure_invalid', proc { |m| m['inputs'][0] = nil }],
      ['input_fields_invalid', proc { |m| m['inputs'].first.delete('role') }],
      ['input_fields_invalid', proc { |m| m['inputs'].first['extra'] = true }],
      ['input_field_invalid', proc { |m| m['inputs'].first['bytes'] = -1 }],
      ['input_field_invalid', proc { |m| m['inputs'].first['bytes'] = 1.0 }],
      ['input_field_invalid', proc { |m| m['inputs'].first['sha256'] = 'not-a-sha' }],
      ['input_field_invalid', proc { |m| m['inputs'].first['path'] = 42 }],
      ['input_field_invalid', proc { |m| m['inputs'].first['authority'] = '' }],
      ['input_field_invalid', proc { |m| m['inputs'].first['conflicts'] = [nil] }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        mutate_manifest(root, &mutation)
        assert_cli_error(root, code)
      end
    end
  end

  def test_cli_reports_invalid_json_and_non_object_manifest
    with_fixture do |root|
      File.binwrite(manifest_path(root), '{bad')
      assert_cli_error(root, 'manifest_json_invalid')
      File.binwrite(manifest_path(root), '[]')
      assert_cli_error(root, 'manifest_structure_invalid')
    end
  end

  def test_cli_rejects_absolute_traversal_and_nul_input_paths
    ['/tmp/input.bin', '../input.bin', 'docs/../../input.bin', "docs/\0input.bin"].each do |path|
      with_fixture do |root|
        mutate_manifest(root) { |m| m['inputs'].first['path'] = path }
        assert_cli_error(root, 'input_path_invalid')
      end
    end
  end

  def test_cli_rejects_symlink_replacements_even_when_the_bytes_match
    with_fixture do |root|
      input = File.join(root, 'docs/input.bin')
      saved = File.join(root, 'docs/saved.bin')
      File.rename(input, saved)
      File.symlink(saved, input)
      assert_cli_error(root, 'input_path_invalid')
      File.unlink(input)
      File.symlink(File.join(root, 'missing'), input)
      assert_cli_error(root, 'input_path_invalid')
    end

    with_fixture do |root|
      Dir.mktmpdir('rfc-inputs-outside') do |outside|
        File.binwrite(File.join(outside, 'input.bin'), "\x00".b)
        File.symlink(outside, File.join(root, 'docs/linked'))
        mutate_manifest(root) { |m| m['inputs'].first['path'] = 'docs/linked/input.bin' }
        assert_cli_error(root, 'input_path_invalid')
      end
    end

    with_fixture do |root|
      File.symlink(File.join(root, 'docs'), File.join(root, 'linked'))
      mutate_manifest(root) { |m| m['inputs'].first['path'] = 'linked/input.bin' }
      assert_cli_error(root, 'input_path_invalid')
    end
  end

  def test_cli_rejects_directories_and_a_symlink_manifest_before_reading
    with_fixture do |root|
      input = File.join(root, 'docs/input.bin')
      File.unlink(input)
      Dir.mkdir(input)
      assert_cli_error(root, 'input_not_regular')
    end

    with_fixture do |root|
      saved = File.join(root, 'saved.json')
      File.rename(manifest_path(root), saved)
      File.symlink(saved, manifest_path(root))
      assert_cli_error(root, 'manifest_path_invalid')
      File.unlink(manifest_path(root))
      Dir.mkdir(manifest_path(root))
      assert_cli_error(root, 'manifest_not_regular')
    end
  end

  def test_cli_rejects_unknown_and_missing_arguments_with_exit_2
    with_fixture do |root|
      [['--root', root, '--unknown'], ['--root', root, 'extra'], [], ['--root']].each do |args|
        out, err, result = Open3.capture3(RbConfig.ruby, CHECK_RFC_INPUTS, *args)
        assert_equal 2, result.exitstatus, out + err
        assert_includes err, 'Usage: check-rfc-inputs.rb --root ROOT'
        assert_empty out
      end
    end
  end

  def test_cli_reports_unreadable_regular_files_without_a_backtrace
    skip 'permission denial cannot be exercised as root' if Process.uid.zero?

    ['docs/input.bin', 'docs/push-system/rfc-input-manifest.v1.json'].each do |relative|
      with_fixture do |root|
        path = File.join(root, relative)
        begin
          File.chmod(0, path)
          code = relative.end_with?('.bin') ? 'input_unreadable' : 'manifest_unreadable'
          assert_cli_error(root, code)
        ensure
          File.chmod(0600, path)
        end
      end
    end
  end

  def test_public_validate_returns_an_array_of_stable_string_errors
    require_relative '../rfc_inputs'
    with_fixture do |root|
      assert_equal [], ArchitectureDocs::RfcInputs.validate(root)
      File.binwrite(File.join(root, 'docs/input.bin'), "\xff".b)
      errors = ArchitectureDocs::RfcInputs.validate(root)
      assert_kind_of Array, errors
      assert errors.all? { |error| error.is_a?(String) }
      assert_match(/\Ainput_sha_mismatch /, errors.first)
      assert_equal errors, ArchitectureDocs::RfcInputs.validate(root)
    end
  end

  private

  def assert_cli_error(root, code)
    out, err, result = run_cli(root)
    assert_equal 1, result.exitstatus, out + err
    assert_includes out, code
    assert_empty err
  end

  def run_cli(root, *args)
    Open3.capture3(RbConfig.ruby, CHECK_RFC_INPUTS, '--root', root, *args)
  end

  def manifest_path(root)
    File.join(root, 'docs/push-system/rfc-input-manifest.v1.json')
  end

  def mutate_manifest(root)
    manifest = JSON.parse(File.binread(manifest_path(root)))
    yield manifest
    File.binwrite(manifest_path(root), JSON.pretty_generate(manifest) + "\n")
  end

  def snapshot(root)
    Dir.glob(File.join(root, '**/*')).select { |path| File.file?(path) }.map do |path|
      [path, File.binread(path), File.mtime(path)]
    end
  end

  def with_fixture
    Dir.mktmpdir('rfc-inputs-test') do |root|
      FileUtils.mkdir_p(File.join(root, 'docs/push-system'))
      File.binwrite(File.join(root, 'docs/input.bin'), "\x00".b)
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
      File.binwrite(manifest_path(root), JSON.pretty_generate(manifest) + "\n")
      yield root
    end
  end
end
