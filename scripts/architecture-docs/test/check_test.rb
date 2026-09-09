#!/usr/bin/env ruby
# frozen_string_literal: true

require 'minitest/autorun'
require 'fileutils'
require 'digest'
require 'json'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require_relative 'support/document_check_fixture'

class DocumentCheckTest < Minitest::Test
  ROOT = File.expand_path('../../..', __dir__)
  CLI = File.join(ROOT, 'scripts/architecture-docs/check.rb')

  def test_draft_runs_all_real_components_for_historical_code_aligned_fixture
    DocumentCheckFixture.with_fixture do |root|
      before = snapshot(root)
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--draft', '--root', root)

      assert_equal 0, status.exitstatus, out + err
      assert_includes out, 'architecture_docs_valid'
      assert_includes out, 'html_targets=rfc'
      assert_empty err
      assert_equal before, snapshot(root)
    end
  end

  def test_catalog_source_symbol_decision_and_input_drift_keep_specific_errors
    DocumentCheckFixture.with_fixture do |root|
      catalog = read_json(root, 'docs/push-system/push-capability-catalog.v1.json')
      catalog['kinds'].shift
      duplicate_kind = catalog['kinds'].first.fetch('kind')
      catalog['kinds'] << catalog['kinds'].first.dup
      write_json(root, 'docs/push-system/push-capability-catalog.v1.json', catalog)

      source_catalog = read_json(root, 'design-source-catalog.v1.json')
      source_path = source_catalog['sources'].first.fetch('path')
      File.open(File.join(root, source_path), 'ab') { |file| file.write("\nsource drift\n") }

      manifest = read_json(root, 'docs/push-system/push-evidence-manifest.v1.json')
      symbol_entry = manifest['evidence'].find { |entry| entry['id'] == 'a10-source' }
      symbol_path = File.join(root, symbol_entry.fetch('path'))
      symbol_lines = File.binread(symbol_path).lines
      symbol_lines[symbol_entry.fetch('start_line') - 1] = ' ' + symbol_lines[symbol_entry.fetch('start_line') - 1]
      File.binwrite(symbol_path, symbol_lines.join)

      line_entry = manifest['evidence'].find { |entry| entry['id'] == 'account-hook' }
      line_path = File.join(root, line_entry.fetch('path'))
      line_bytes = File.binread(line_path).lines
      line_bytes.insert(line_entry.fetch('start_line') - 1, "\n")
      File.binwrite(line_path, line_bytes.join)

      missing_entry = manifest['evidence'].find { |entry| entry['id'] == 'account-plan' }
      missing_entry['symbol'] = 'document_check_missing_symbol'
      write_json(root, 'docs/push-system/push-evidence-manifest.v1.json', manifest)

      decisions = File.join(root, 'docs/push-system/grill-decisions-2026-09-02.md')
      decision_bytes = File.binread(decisions)
      changed_decisions = decision_bytes.sub(/^\| 7 \|.*\n/, '')
      refute_equal decision_bytes, changed_decisions
      File.binwrite(decisions, changed_decisions)

      input = File.join(root, 'docs/Project_Architecture_Blueprint.md')
      File.open(input, 'ab') { |file| file.write("\ninput drift\n") }

      out, err, status = run_check(root, '--draft')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, "duplicate_kind id=#{duplicate_kind}"
      assert_includes out, 'enum_coverage_mismatch'
      assert_includes out, "source_sha_mismatch path=#{source_path}"
      assert_includes out, 'symbol_sha_mismatch id=a10-source'
      assert_includes out, 'symbol_lines_mismatch id=account-hook'
      assert_includes out, 'symbol_missing symbol=document_check_missing_symbol id=account-plan'
      assert_includes out, 'approved_question_missing question=7'
      assert_includes out, 'input_sha_mismatch id=architecture-blueprint-markdown'
      assert_equal 1, out.lines.count { |line| line.start_with?('input_sha_mismatch id=architecture-blueprint-markdown') }
      assert_includes out, 'rfc_dependency_sha_mismatch'
      assert_empty err
    end
  end

  def test_wbs_requires_all_foundation_rows
    DocumentCheckFixture.with_fixture do |root|
      wbs = read_json(root, 'docs/push-system/push-system-wbs.v1.json')
      wbs['foundation_workstreams']&.reject! { |row| row['id'] == 'W21' }
      wbs['foundation_work_packages']&.reject! { |row| row['id'] == 'W21' }
      write_json(root, 'docs/push-system/push-system-wbs.v1.json', wbs)

      assert_check_error(root, 'wbs_foundation_set_invalid')
    end
  end

  def test_wbs_staleness_is_reported_after_html_is_formally_rebuilt
    DocumentCheckFixture.with_fixture do |root|
      rfc = File.join(root, 'docs/push-system/push-system-implementation-rfc.md')
      before = File.binread(rfc)
      after = before.sub('| W21 | 发布编排'.b, '| W21 | 陈旧发布编排'.b)
      refute_equal before, after
      File.binwrite(rfc, after)
      DocumentCheckFixture.run_ruby(root, 'scripts/architecture-docs/build.rb', 'rfc', '--root', root, '--draft')

      out, err, status = run_check(root, '--draft')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'wbs_rfc_stale'
      refute_includes out, 'html_stale'
      assert_empty err
    end
  end

  def test_missing_generated_documents_are_reported_independently
    DocumentCheckFixture.with_fixture do |root|
      File.delete(File.join(root, 'docs/push-system/push-capability-catalog.md'))
      File.delete(File.join(root, 'docs/push-system/push-system-implementation-rfc.html'))

      out, err, status = run_check(root, '--draft')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'markdown_missing'
      assert_includes out, 'html_missing target=rfc'
      assert_empty err
    end
  end

  def test_html_byte_template_implementation_and_asset_drift_are_detected
    mutations = {
      'html bytes' => proc { |root| File.open(File.join(root, 'docs/push-system/push-system-implementation-rfc.html'), 'ab') { |file| file.write('changed') } },
      'template' => proc { |root| File.open(File.join(root, 'scripts/architecture-docs/templates/document.html.erb'), 'ab') { |file| file.write("\n") } },
      'implementation' => proc { |root| File.open(File.join(root, 'scripts/architecture-docs/markdown_renderer.rb'), 'ab') { |file| file.write("\n# changed fixture implementation\n") } }
    }
    mutations.each do |label, mutation|
      DocumentCheckFixture.with_fixture do |root|
        mutation.call(root)
        out, err, status = run_check(root, '--draft', local: label == 'implementation')
        assert_equal 1, status.exitstatus, "#{label}: #{out}#{err}"
        assert_includes out, 'html_stale target=rfc'
        assert_empty err
      end
    end
    DocumentCheckFixture.with_fixture do |root|
      File.open(File.join(root, 'scripts/architecture-docs/assets/mermaid.min.js'), 'ab') { |file| file.write('changed') }
      assert_check_error(root, 'mermaid_asset_bytes_mismatch path=mermaid.min.js')
    end
  end

  def test_document_links_and_linked_parent_are_rejected
    DocumentCheckFixture.with_fixture do |root|
      markdown = File.join(root, 'docs/push-system/push-capability-catalog.md')
      markdown_saved = markdown + '.saved'
      File.rename(markdown, markdown_saved)
      File.link(markdown_saved, markdown)
      html = File.join(root, 'docs/push-system/push-system-implementation-rfc.html')
      html_saved = html + '.saved'
      File.rename(html, html_saved)
      File.symlink(html_saved, html)

      out, err, status = run_check(root, '--draft')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'markdown_path_invalid'
      assert_includes out, 'html_output_path_invalid'
      assert_empty err
    end
    DocumentCheckFixture.with_fixture do |root|
      templates = File.join(root, 'scripts/architecture-docs/templates')
      saved = templates + '.saved'
      File.rename(templates, saved)
      File.symlink(saved, templates)

      assert_check_error(root, 'html_template_path_invalid')
    end
  end

  def test_fixed_catalog_pair_hardlink_is_rejected
    DocumentCheckFixture.with_fixture do |root|
      manifest = File.join(root, 'docs/push-system/push-evidence-manifest.v1.json')
      saved = manifest + '.saved'
      File.rename(manifest, saved)
      File.link(saved, manifest)

      assert_check_error(root, 'push_path_invalid path=docs/push-system/push-evidence-manifest.v1.json')
    end
  end

  def test_strict_preserves_release_errors_detects_dirty_state_and_is_read_only
    DocumentCheckFixture.with_fixture do |root|
      before = snapshot(root)
      out, err, status = run_check(root, '--check')
      assert_equal 1, status.exitstatus, out + err
      assert_equal strict_release_errors.sort, error_lines(out).sort
      assert_empty err
      assert_equal before, snapshot(root)

      File.binwrite(File.join(root, 'untracked-document-check-fixture'), 'dirty')
      out, err, status = run_check(root, '--check')
      assert_equal 1, status.exitstatus, out + err
      assert_equal (strict_release_errors + ['worktree_dirty']).sort, error_lines(out).sort
      assert_empty err
    end
  end

  def test_strict_checks_markdown_staleness_despite_provisional_release_errors
    DocumentCheckFixture.with_fixture do |root|
      markdown = File.join(root, 'docs/push-system/push-capability-catalog.md')
      File.open(markdown, 'ab') { |file| file.write("\nstale\n") }
      DocumentCheckFixture.git(root, 'add', markdown)
      DocumentCheckFixture.git(root, '-c', 'user.name=Document Check Test', '-c', 'user.email=document-check@example.invalid',
                               'commit', '-qm', 'commit stale generated catalog')
      before = snapshot(root)

      out, err, status = run_check(root, '--check')
      assert_equal 1, status.exitstatus, out + err
      assert_equal (strict_release_errors + ['markdown_stale']).sort, error_lines(out).sort
      assert_empty err
      assert_equal before, snapshot(root)
    end
  end

  def test_default_root_is_relative_to_installed_script_not_calling_directory
    DocumentCheckFixture.with_fixture do |root|
      Dir.mktmpdir('document-check-cwd-') do |calling_directory|
        before = snapshot(root)
        local_cli = File.join(root, 'scripts/architecture-docs/check.rb')
        out, err, status = Open3.capture3(RbConfig.ruby, local_cli, '--draft', chdir: calling_directory)
        assert_equal 0, status.exitstatus, out + err
        assert_includes out, 'architecture_docs_valid html_targets=rfc'
        assert_empty err
        assert_equal before, snapshot(root)
      end
    end
  end

  def test_cli_requires_exactly_one_mode_and_rejects_nonliteral_options
    [%w[], %w[--unknown], %w[--dra], %w[--draft --check], %w[--check --check],
     %w[--draft --root one --root two], %w[--draft extra], %w[--help --draft]].each do |arguments|
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, *arguments)

      assert_equal 2, status.exitstatus, "#{arguments.inspect}: #{out}#{err}"
      assert_empty out
      assert_includes err, 'Usage: check.rb'
    end

    [%w[--help], %w[-h]].each do |arguments|
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, *arguments)

      assert_equal 0, status.exitstatus, out + err
      assert_includes out, 'Usage: check.rb'
      assert_empty err
    end
  end

  def test_invalid_roots_are_content_failures_without_backtraces
    Dir.mktmpdir('document-check-root-') do |parent|
      missing = File.join(parent, 'missing')
      regular = File.join(parent, 'regular')
      link = File.join(parent, 'link')
      File.binwrite(regular, 'not a directory')
      File.symlink(parent, link)
      [[missing, 'root_missing'], [regular, 'root_invalid'], [link, 'root_path_invalid']].each do |root, code|
        out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--draft', '--root', root)

        assert_equal 1, status.exitstatus, out + err
        assert_includes out, "#{code} path=#{root}"
        refute_includes out + err, 'Traceback'
        assert_empty err
      end
    end
  end

  def test_explicit_relative_root_is_resolved_from_calling_directory
    Dir.mktmpdir('document-check-relative-root-') do |calling_directory|
      FileUtils.mkdir_p(File.join(calling_directory, 'fixture'))
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--draft', '--root', 'fixture', chdir: calling_directory)

      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'manifest_missing path=docs/push-system/rfc-input-manifest.v1.json'
      refute_includes out, 'root_missing'
      assert_empty err
    end
  end

  def test_ci_declares_one_strict_gate_before_rust_and_fetches_history
    workflow = File.binread(File.join(ROOT, '.github/workflows/ci.yml'))
    gate = 'run: ruby scripts/architecture-docs/check.rb --check'

    assert_equal 1, workflow.scan(gate).length
    assert_operator workflow.index(gate), :<, workflow.index('uses: dtolnay/rust-toolchain@stable')
    assert_match(/uses: actions\/checkout@v4\n\s+with:\n\s+fetch-depth: 0/, workflow)
  end

  private

  def run_check(root, mode, local: false)
    executable = local ? File.join(root, 'scripts/architecture-docs/check.rb') : CLI
    Open3.capture3(RbConfig.ruby, executable, mode, '--root', root)
  end

  def assert_check_error(root, code, mode = '--draft', local: false)
    out, err, status = run_check(root, mode, local: local)
    assert_equal 1, status.exitstatus, out + err
    assert_includes out, code
    assert_empty err
  end

  def error_lines(output)
    output.lines.map(&:strip).reject { |line| line.empty? || line == 'html_targets=rfc' }
  end

  def strict_release_errors
    ['provisional path=docs/push-system/push-capability-catalog.v1.json',
     'provisional path=docs/push-system/push-evidence-manifest.v1.json',
     'rfc_status_provisional', 'wbs_status_provisional']
  end

  def read_json(root, relative)
    JSON.parse(File.binread(File.join(root, relative)))
  end

  def write_json(root, relative, object)
    File.binwrite(File.join(root, relative), JSON.pretty_generate(object) + "\n")
  end

  def snapshot(root)
    tracked = DocumentCheckFixture.git(root, 'ls-files', '-z').split("\0")
    paths = tracked.map { |relative| File.join(root, relative) }
    paths << File.join(root, '.git/index')
    paths.each_with_object({}) do |path, result|
      stat = File.stat(path)
      result[path.delete_prefix(root + '/')] = [Digest::SHA256.file(path).hexdigest, stat.size, stat.mtime.to_r]
    end
  end
end
