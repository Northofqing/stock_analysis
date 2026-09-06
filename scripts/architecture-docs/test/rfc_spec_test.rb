#!/usr/bin/env ruby
# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'fileutils'
require 'json'
require 'digest'
require 'tmpdir'

class RfcSpecTest < Minitest::Test
  ROOT = File.expand_path('../../..', __dir__)
  CLI = File.join(ROOT, 'scripts/architecture-docs/check-rfc.rb')
  RFC = 'docs/push-system/push-system-implementation-rfc.md'
  DEPENDENCIES = %w[rfc-input-manifest.v1.json push-capability-catalog.v1.json
                    push-evidence-manifest.v1.json grill-decisions-2026-09-02.md].freeze

  def test_public_cli_accepts_the_frozen_domain_contract_in_draft
    out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', ROOT, '--draft')
    assert_equal 0, result.exitstatus, out + err
    assert_equal "rfc_spec_valid\n", out
    assert_empty err
  end

  def test_draft_still_rejects_metadata_counts_and_frozen_dependency_drift
    cases = [
      ['rfc_version_invalid', proc { |m| m['version'] = 'invented-v2' }],
      ['rfc_baseline_invalid', proc { |m| m['source_baseline'] = '0' * 40 }],
      ['rfc_metadata_hash_invalid', proc { |m| m['catalog_sha256'] = '0' * 64 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['kinds'] = 64 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['producers'] = 101 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['units'] = 51 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['evidence'] = 194 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['mapped'] = 25 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['durable_kinds'] = 24 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['unmapped'] = 42 }],
      ['rfc_counts_invalid', proc { |m| m['counts']['states'] = 13 }],
      ['rfc_status_counts_invalid', proc { |m| m['status_counts']['ACTIVE'] = 37 }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_metadata(root, &mutation)
        assert_cli_error(root, code)
      end
    end
    DEPENDENCIES.each do |name|
      with_fixture do |root|
        File.open(File.join(root, 'docs/push-system', name), 'ab') { |file| file.write("\n") }
        assert_cli_error(root, 'rfc_dependency_sha_mismatch')
      end
    end
  end

  def test_domain_contract_sections_fields_and_references_are_enforced
    cases = [
      ['rfc_section_missing', proc { |s| s.sub('## 类型：PreparedFacts（PROPOSED）', '## 已删除事实合同') }],
      ['rfc_section_duplicate', proc { |s| s + "\n## 类型：RunContext（PROPOSED）\n[Q:33]\n" }],
      ['rfc_type_fields_invalid', proc { |s| s.sub(/^\| run_id \|.*\n/, '') }],
      ['rfc_table_invalid', proc { |s| s.sub('| run_id | RunId |', '| run_id | |') }],
      ['rfc_field_type_invalid', proc { |s| s.sub('| run_id | RunId |', '| run_id | bool |') }],
      ['rfc_reference_invalid', proc { |s| s.sub('[Q:33]', '[Q:109]') }],
      ['rfc_reference_invalid', proc { |s| s.sub('[unit:MU-p01]', '[unit:MU-invented]') }],
      ['rfc_reference_invalid', proc { |s| s.sub('[producer:p01-scheduled]', '[producer:invented]') }],
      ['rfc_reference_invalid', proc { |s| s.sub('[evidence:push-kind]', '[evidence:invented]') }],
      ['rfc_placeholder_forbidden', proc { |s| s + "\nTODO: replace missing authority\n" }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_text(root, &mutation)
        assert_cli_error(root, code)
      end
    end
  end

  def test_outcome_authority_mapping_state_and_reason_contracts_fail_closed
    cases = [
      ['rfc_job_variants_invalid', proc { |s| s.sub(/^\| NoData \| \{reason:.*\n/, '') }],
      ['rfc_job_payload_invalid', proc { |s| s.sub('| Ready | PreparedPush |', '| Ready | bool |') }],
      ['rfc_delivery_variants_invalid', proc { |s| s.sub(/^\| PartiallyAccepted \| CompatibilityEvidenceRef.*\n/, '') }],
      ['rfc_delivery_authority_invalid', proc { |s| s.sub('| BestEffortAccepted | CompatibilityEvidenceRef | compat | never |', '| BestEffortAccepted | VerifiedTerminalRef | strong | policy_bound |') }],
      ['rfc_delivery_authority_invalid', proc { |s| s.sub('| PartiallyAccepted | CompatibilityEvidenceRef | compat | never |', '| PartiallyAccepted | CompatibilityEvidenceRef | compat | policy_bound |') }],
      ['rfc_completion_authority_invalid', proc { |s| s.sub('| BestEffortAccepted | CompatibilityObservation | 仅记录本地观察 | None |', '| BestEffortAccepted | CompatibilityObservation | 仅记录本地观察 | AdvanceAccepted |') }],
      ['rfc_mapping_invalid', proc { |s| s.sub('| FactorIC | DailyReport | FactorIC |', '| FactorIC | DailyReport | None |') }],
      ['rfc_mapping_invalid', proc { |s| s.sub(/^\| HoldingPlan \| HoldingPlan.*\n/, '') }],
      ['rfc_mapping_invalid', proc { |s| s.sub('| FactorIC | DailyReport | FactorIC |', '| FactorIC | FactorIC | FactorIC |') }],
      ['rfc_unmapped_invalid', proc { |s| s.sub('| Announcement | ACTIVE | adapt_or_conform |', '| Announcement | INACTIVE | keep_inactive |') }],
      ['rfc_unmapped_invalid', proc { |s| s.sub(/^\| PolicyHit \| INACTIVE.*\n/, '') }],
      ['rfc_states_invalid', proc { |s| s.sub(/^\| Reserved \| Blocked.*\n/, '') }],
      ['rfc_state_projection_invalid', proc { |s| s.sub('| UncertainManualReview | TransportUncertain/AlreadyTerminal | 是 | never |', '| UncertainManualReview | TransportAccepted | 是 | automatic |') }],
      ['rfc_reason_duplicate', proc { |s| s.sub('| input.source_unready |', '| input.source_unavailable |') }],
      ['rfc_reason_namespace_invalid', proc { |s| s.sub('| input.source_unready |', '| source_unready |') }],
      ['rfc_reason_coverage_invalid', proc { |s| s.sub(/^\| operator.unauthorized \|.*\n/, '') }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_text(root, &mutation)
        assert_cli_error(root, code)
      end
    end
  end

  def test_malformed_tables_return_content_errors_without_backtraces
    with_fixture do |root|
      change_text(root) { |s| s.sub('| 代码 | 条件 | 处理 | 依据 |', "|\n| 代码 | 条件 | 处理 | 依据 |") }
      assert_cli_error(root, 'rfc_table_invalid')
    end
    with_fixture do |root|
      change_text(root) { |s| s.sub('| schedule.not_trading_day |', "|\n| schedule.not_trading_day |") }
      assert_cli_error(root, 'rfc_table_invalid')
    end
  end

  def test_modes_and_arguments_are_explicit_and_content_checks_remain_in_strict
    with_fixture do |root|
      assert_cli_error(root, 'rfc_status_provisional', '--check')
      change_metadata(root) { |m| m['counts']['kinds'] = 64 }
      assert_cli_error(root, 'rfc_counts_invalid', '--check')
      [[], ['--root'], ['--root', root], ['--root', root, '--draft', '--unknown'],
       ['--root', root, '--draft', 'extra'], ['--root', root, '--draft', '--check'],
       ['--root', root, '--draft', '--draft']].each do |args|
        out, err, result = Open3.capture3(RbConfig.ruby, CLI, *args)
        assert_equal 2, result.exitstatus, out + err
        assert_empty out
        assert_includes err, 'Usage: check-rfc.rb'
      end
    end
  end

  def test_missing_invalid_and_linked_documents_are_safe_failures
    with_fixture do |root|
      assert_cli_error(File.join(root, 'absent'), 'rfc_root_missing')
      assert_cli_error(File.join(root, RFC), 'rfc_root_invalid')
      File.unlink(File.join(root, RFC))
      assert_cli_error(root, 'rfc_document_missing')
    end
    [RFC, 'docs/push-system/rfc-input-manifest.v1.json'].each do |path|
      with_fixture do |root|
        file = File.join(root, path)
        saved = file + '.saved'
        File.rename(file, saved)
        File.symlink(saved, file)
        assert_cli_error(root, 'rfc_path_invalid')
        File.unlink(file)
        File.symlink(File.join(root, 'absent'), file)
        assert_cli_error(root, 'rfc_path_invalid')
        File.unlink(file)
        Dir.mkdir(file)
        assert_cli_error(root, 'rfc_not_regular')
      end
    end
    with_fixture do |root|
      File.rename(File.join(root, 'docs'), File.join(root, 'saved-docs'))
      File.symlink(File.join(root, 'saved-docs'), File.join(root, 'docs'))
      assert_cli_error(root, 'rfc_path_invalid')
    end
    with_fixture do |root|
      File.binwrite(File.join(root, RFC), "\xff".b)
      assert_cli_error(root, 'rfc_encoding_invalid')
    end
    with_fixture do |root|
      change_text(root) { |s| s.sub('"schema_version": 1', '"schema_version": BAD') }
      assert_cli_error(root, 'rfc_json_invalid')
    end
  end

  def test_permission_denial_returns_an_error_without_a_backtrace
    skip 'permission denial is not observable as root' if Process.uid.zero?
    with_fixture do |root|
      path = File.join(root, RFC)
      begin
        File.chmod(0, path)
        assert_cli_error(root, 'rfc_io_error')
      ensure
        File.chmod(0600, path)
      end
    end
  end

  def test_public_validate_and_cli_do_not_write_or_repair_inputs
    require_relative '../rfc_spec'
    with_fixture do |root|
      before = snapshot(root)
      assert_equal [], ArchitectureDocs::RfcSpec.validate(root)
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, result.exitstatus, out + err
      assert_equal before, snapshot(root)
      change_text(root) { |s| s.sub('[Q:33]', '[Q:0]') }
      before = snapshot(root)
      errors = ArchitectureDocs::RfcSpec.validate(root)
      assert_kind_of Array, errors
      assert errors.all? { |error| error.is_a?(String) }
      assert_includes errors.join("\n"), 'rfc_reference_invalid'
      assert_equal errors, ArchitectureDocs::RfcSpec.validate(root)
      assert_cli_error(root, 'rfc_reference_invalid')
      assert_equal before, snapshot(root)
    end
  end

  def test_semantic_reversal_identity_payload_material_is_rejected
    with_fixture do |root|
      change_text(root) { |s| s.sub('namespace,unit_id,completion_owner,source_contract_id,occurrence,subject,audience', 'namespace,unit_id,completion_owner,source_contract_id,occurrence,subject,audience,payload_sha256') }
      assert_cli_error(root, 'rfc_identity_contract_invalid')
    end
  end

  def test_semantic_reversal_already_terminal_bypass_is_rejected
    with_fixture do |root|
      change_text(root) do |s|
        s.sub('| AlreadyTerminal | VerifiedTerminalRef | strong | policy_bound | CompletionRule::AlreadyTerminal |',
              '| AlreadyTerminal | VerifiedTerminalRef | strong | policy_bound | SkipExactBindingAndAdvanceAll |')
         .sub('| AlreadyTerminal | CompletionRule::AlreadyTerminal | CompletionRule::AlreadyTerminal.schedule | CompletionRule::AlreadyTerminal.cursor | CompletionRule::AlreadyTerminal.forbidden |',
              '| AlreadyTerminal | SkipExactBinding | Close | AdvanceEveryDisposition | None |')
      end
      assert_cli_error(root, 'rfc_completion_binding_invalid')
    end
  end

  def test_semantic_reversal_empty_adapter_contract_is_rejected
    with_fixture do |root|
      change_text(root) { |s| s.sub(/## 适配器一致性合同（PROPOSED）\n.*?(?=## Task2 验证边界)/m, "## 适配器一致性合同（PROPOSED）\n\n[Q:27]\n\n") }
      assert_cli_error(root, 'rfc_adapter_contract_invalid')
    end
  end

  def test_semantic_reversal_run_identity_hash_exclusion_is_rejected
    with_fixture do |root|
      change_text(root) { |s| s.sub(/^\| run_id \|.*$/) { |row| row.sub('| 纳入 |', '| 派生且排除自身 |') } }
      assert_cli_error(root, 'rfc_canonical_rule_invalid')
    end
  end

  def test_semantic_contract_values_cannot_be_replaced_with_weaker_rules
    cases = [
      ['rfc_identity_contract_invalid', proc { |s| s.sub('| intent_id | IntentId | IdentityRule::PreparedPushIntent |', '| intent_id | IntentId | 包含 payload 哈希 |') }],
      ['rfc_identity_contract_invalid', proc { |s| s.sub('| payload_sha256,rendered_sha256,evidence_sha256 |', '| None |') }],
      ['rfc_completion_binding_invalid', proc { |s| s.sub('| Accepted | RequeryExactBinding |', '| Accepted | SkipBinding |') }],
      ['rfc_completion_binding_invalid', proc { |s| s.sub('| Rejected | RequeryExactBinding | RegisteredPolicy | KeepOpen | None |', '| Rejected | RequeryExactBinding | RegisteredPolicy | KeepOpen | AdvanceAccepted |') }],
      ['rfc_completion_binding_invalid', proc { |s| s.sub('| AdvanceManualAccepted | NeverTransportAccepted |', '| AdvanceAccepted | NeverTransportAccepted |') }],
      ['rfc_adapter_contract_invalid', proc { |s| s.sub('| n02_authority | MU-news-flash-aggregate | PreserveWindowReservationAttemptSettlement |', '| n02_authority | MU-news-flash-aggregate | CopyGenericReceipts |') }],
      ['rfc_adapter_contract_invalid', proc { |s| s.sub('| shadow_side_effects | Shadow | None |', '| shadow_side_effects | Shadow | ProviderQuery |') }],
      ['rfc_canonical_rule_invalid', proc { |s| s.sub(/^\| verified_at \|.*$/) { |row| row.sub('| 派生且排除自身 |', '| 纳入 |') } }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_text(root, &mutation)
        assert_cli_error(root, code)
      end
    end
  end

  def test_chinese_contract_headings_and_table_headers_remain_required
    with_fixture do |root|
      change_text(root) { |s| s.sub('## 身份合同（PROPOSED）', '## Identity contract (PROPOSED)') }
      assert_cli_error(root, 'rfc_section_missing')
    end
    with_fixture do |root|
      change_text(root) { |s| s.sub('| 字段 | 类型 | 不变量 | 规范化 |', '| field | type | invariant | canonical |') }
      assert_cli_error(root, 'rfc_table_invalid')
    end
  end

  private

  def with_fixture
    Dir.mktmpdir('rfc-spec-test') do |root|
      inputs = JSON.parse(File.read(File.join(ROOT, 'docs/push-system/rfc-input-manifest.v1.json')))
      paths = [RFC] + DEPENDENCIES.map { |name| 'docs/push-system/' + name } + inputs['inputs'].map { |item| item['path'] }
      paths.uniq.each do |path|
        destination = File.join(root, path)
        FileUtils.mkdir_p(File.dirname(destination))
        FileUtils.copy_file(File.join(ROOT, path), destination)
      end
      yield root
    end
  end

  def change_metadata(root)
    path = File.join(root, RFC)
    text = File.read(path)
    json = text.match(/```json\n(.*?)\n```/m)
    metadata = JSON.parse(json[1])
    yield metadata
    File.write(path, text.sub(json[0], "```json\n#{JSON.pretty_generate(metadata)}\n```"))
  end

  def change_text(root)
    path = File.join(root, RFC)
    before = File.read(path)
    after = yield before
    refute_equal before, after, 'negative fixture must actually change the document'
    File.write(path, after)
  end

  def snapshot(root)
    Dir.glob(File.join(root, '**/*')).select { |path| File.file?(path) }.map do |path|
      [path, Digest::SHA256.file(path).hexdigest, File.mtime(path)]
    end
  end

  def assert_cli_error(root, code, mode = '--draft')
    out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, mode)
    assert_equal 1, result.exitstatus, out + err
    assert_includes out, code
    assert_empty err
  end
end
