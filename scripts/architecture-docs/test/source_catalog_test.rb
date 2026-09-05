#!/usr/bin/env ruby
# frozen_string_literal: true

require 'digest'
require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require_relative '../source_catalog'

CHECK_SOURCES = File.expand_path('../check-sources.rb', __dir__)

class SourceCatalogTest < Minitest::Test
  def test_cli_accepts_a_valid_fixture_and_rejects_changed_source_bytes
    with_fixture do |root|
      out, err, result = run_cli(root)
      assert result.success?, out + err

      File.binwrite(File.join(root, 'docs/source.md'), "changed\n")
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'source_sha_mismatch'
    end
  end

  def test_cli_rejects_duplicate_source_ids_and_paths
    with_fixture do |root|
      mutate_catalog(root) do |catalog|
        duplicate = catalog['sources'].first.dup
        duplicate['path'] = 'docs/other.md'
        catalog['sources'] << duplicate
        File.binwrite(File.join(root, 'docs/other.md'), "# Other\n")
        catalog['sources'].last['sha256'] = Digest::SHA256.hexdigest("# Other\n")
      end
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'duplicate_source_id'

      mutate_catalog(root) do |catalog|
        catalog['sources'].last['id'] = 'other-source'
        catalog['sources'].last['path'] = catalog['sources'].first['path']
        catalog['sources'].last['sha256'] = catalog['sources'].first['sha256']
      end
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'duplicate_source_path'
    end
  end

  def test_cli_rejects_absolute_traversal_and_symlink_escape_paths
    with_fixture do |root|
      outside = Dir.mktmpdir('source-catalog-outside')
      begin
        outside_source = File.join(outside, 'outside.md')
        File.binwrite(outside_source, "outside\n")
        File.symlink(outside_source, File.join(root, 'docs/link.md'))
        File.symlink(outside, File.join(root, 'docs/outside-dir'))

        invalid_paths = ['/tmp/source.md', '../source.md', 'docs/link.md', 'docs/outside-dir/missing.md']
        invalid_paths.each do |invalid_path|
          mutate_catalog(root) { |catalog| catalog['sources'].first['path'] = invalid_path }
          out, err, result = run_cli(root)
          refute result.success?, invalid_path
          assert_includes out + err, 'source_path_invalid', invalid_path
          reset_fixture_catalog(root)
        end
      ensure
        FileUtils.remove_entry(outside)
      end
    end
  end

  def test_cli_rejects_nul_source_and_approved_decision_paths_without_a_backtrace
    with_fixture do |root|
      mutate_catalog(root) { |catalog| catalog['sources'].first['path'] = "docs/\0source.md" }
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'source_path_invalid'
      refute_includes out + err, 'source_catalog.rb:'

      reset_fixture_catalog(root)
      mutate_catalog(root) { |catalog| catalog['approved_decisions']['path'] = "docs/\0decisions.md" }
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_path_invalid'
      refute_includes out + err, 'source_catalog.rb:'
    end
  end

  def test_cli_rejects_non_string_source_and_approved_decision_paths
    with_fixture do |root|
      mutate_catalog(root) { |catalog| catalog['sources'].first['path'] = 42 }
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'source_field_invalid'
      refute_includes out + err, 'source_catalog.rb:'

      reset_fixture_catalog(root)
      mutate_catalog(root) { |catalog| catalog['approved_decisions']['path'] = ['docs/decisions.md'] }
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_field_invalid'
      refute_includes out + err, 'source_catalog.rb:'
    end
  end

  def test_cli_rejects_invalid_approved_decision_encoding_without_a_backtrace
    with_fixture do |root|
      invalid_bytes = "\xFF\n".b
      File.binwrite(File.join(root, 'docs/decisions.md'), invalid_bytes)
      update_decisions_hash(root, invalid_bytes)

      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_encoding_invalid'
      refute_includes out + err, 'source_catalog.rb:'
    end
  end

  def test_cli_rejects_invalid_schema_status_and_missing_required_fields
    with_fixture do |root|
      cases = [
        ['catalog_schema_invalid', proc { |catalog| catalog['schema_version'] = 2 }],
        ['catalog_status_invalid', proc { |catalog| catalog['status'] = 'READY' }],
        ['catalog_field_missing', proc { |catalog| catalog.delete('provenance') }],
        ['approved_decisions_question_count_invalid', proc { |catalog| catalog['approved_decisions']['question_count'] = 1 }],
        ['approved_decisions_field_missing', proc { |catalog| catalog['approved_decisions'].delete('sha256') }],
        ['source_field_missing', proc { |catalog| catalog['sources'].first.delete('ruling') }],
        ['source_field_invalid', proc { |catalog| catalog['sources'].first['conflicts'] = 'none' }],
        ['source_field_invalid', proc { |catalog| catalog['sources'].first['sha256'] = 'not-a-sha' }]
      ]

      cases.each do |reason, mutation|
        mutate_catalog(root, &mutation)
        out, err, result = run_cli(root)
        refute result.success?, reason
        assert_includes out + err, reason
        reset_fixture_catalog(root)
      end
    end
  end

  def test_public_validate_returns_an_array_of_string_errors
    with_fixture do |root|
      File.binwrite(File.join(root, 'docs/source.md'), "changed\n")
      errors = ArchitectureDocs::SourceCatalog.validate(root)
      assert_kind_of Array, errors
      assert errors.all? { |error| error.is_a?(String) }
      assert errors.any? { |error| error.include?('source_sha_mismatch') }
    end
  end

  def test_cli_requires_each_approved_question_q1_through_q108_once
    with_fixture do |root|
      decisions_path = File.join(root, 'docs/decisions.md')
      original = File.binread(decisions_path)

      missing = original.sub('| 108 | A | fixture |', '| 109 | A | fixture |')
      File.binwrite(decisions_path, missing)
      update_decisions_hash(root, missing)
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_question_missing question=108'

      reset_fixture_catalog(root)
      duplicate = original.sub('| 108 | A | fixture |', '| 71 | A | fixture |')
      File.binwrite(decisions_path, duplicate)
      update_decisions_hash(root, duplicate)
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_question_duplicate question=71'
    end
  end

  def test_cli_reports_missing_sources_and_approved_decision_drift
    with_fixture do |root|
      FileUtils.rm(File.join(root, 'docs/source.md'))
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'source_missing'

      File.binwrite(File.join(root, 'docs/source.md'), "# Fixture Source\n")
      File.binwrite(File.join(root, 'docs/decisions.md'), decisions_table.sub('fixture', 'changed'))
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_sha_mismatch'

      FileUtils.rm(File.join(root, 'docs/decisions.md'))
      out, err, result = run_cli(root)
      refute result.success?
      assert_includes out + err, 'approved_decisions_missing'
    end
  end

  def test_cli_returns_exit_2_for_unknown_arguments
    with_fixture do |root|
      out, err, result = run_cli(root, '--unknown')
      assert_equal 2, result.exitstatus, out + err

      out, err, result = run_cli(root, 'unexpected')
      assert_equal 2, result.exitstatus, out + err
    end
  end

  private

  def run_cli(root, *args)
    Open3.capture3(RbConfig.ruby, CHECK_SOURCES, '--root', root, *args)
  end

  def mutate_catalog(root)
    path = File.join(root, 'design-source-catalog.v1.json')
    catalog = JSON.parse(File.binread(path))
    yield catalog
    File.binwrite(path, JSON.pretty_generate(catalog) + "\n")
  end

  def reset_fixture_catalog(root)
    source = File.binread(File.join(root, 'docs/source.md'))
    decisions = File.binread(File.join(root, 'docs/decisions.md'))
    catalog = fixture_catalog(source, decisions)
    File.binwrite(File.join(root, 'design-source-catalog.v1.json'), JSON.pretty_generate(catalog) + "\n")
  end

  def update_decisions_hash(root, decisions)
    mutate_catalog(root) do |catalog|
      catalog['approved_decisions']['sha256'] = Digest::SHA256.hexdigest(decisions)
    end
  end

  def with_fixture
    Dir.mktmpdir('source-catalog-test') do |root|
      FileUtils.mkdir_p(File.join(root, 'docs'))
      source = "# Fixture Source\n"
      decisions = decisions_table
      File.binwrite(File.join(root, 'docs/source.md'), source)
      File.binwrite(File.join(root, 'docs/decisions.md'), decisions)
      catalog = fixture_catalog(source, decisions)
      File.binwrite(File.join(root, 'design-source-catalog.v1.json'), JSON.pretty_generate(catalog) + "\n")
      yield root
    end
  end

  def fixture_catalog(source, decisions)
    {
        'schema_version' => 1,
        'status' => 'PROVISIONAL',
        'provenance' => 'user_workspace_snapshot',
        'approved_decisions' => {
          'path' => 'docs/decisions.md',
          'sha256' => Digest::SHA256.hexdigest(decisions),
          'question_count' => 108
        },
        'sources' => [
          {
            'id' => 'fixture-source',
            'path' => 'docs/source.md',
            'sha256' => Digest::SHA256.hexdigest(source),
            'title' => 'Fixture Source',
            'self_version' => nil,
            'self_status' => nil,
            'ruling' => '测试裁决',
            'conflicts' => [],
            'superseded_by' => []
          }
        ]
    }
  end

  def decisions_table
    first = (1..55).map { |number| "| #{number} | A | fixture | fixture |" }
    second = (56..108).map { |number| "| #{number} | A | fixture |" }
    ([
      '## Q1--Q55',
      '| 问题 | 选择 | 已确认约束 | RFC 落点 |',
      '| ---: | :---: | --- | --- |',
      *first,
      '## Q56--Q108',
      '| 问题 | 选择 | 已确认约束 |',
      '| ---: | :---: | --- |',
      *second
    ].join("\n") + "\n")
  end
end
