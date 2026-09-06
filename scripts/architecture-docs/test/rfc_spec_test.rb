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
  SQL = 'docs/push-system/push-system-foundation.v1.sql'
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

  def test_sql_file_is_required_by_the_public_cli
    with_fixture do |root|
      path = File.join(root, SQL)
      File.unlink(path) if File.exist?(path)
      assert_cli_error(root, 'rfc_document_missing')
    end
  end

  def test_public_cli_rejects_weakened_persistence_invariant_profiles
    changes = [
      ['Ready 必须整组非空且不可变','Ready 可以缺少字节'],
      ['初始 NoData/Disabled 必须整组 NULL；隔离后仍保持 NULL','非发送必须伪造 PreparedPush'],
      ['必须等于本次 CAS 后的 intent.reason','允许任意事件 reason'],
      ['.bail on；持久化 DDL 前快照对象，不用事后补建掩盖缺失','先补建再检查兼容'],
      ['同时校验 TEXT 类型、字符长度、BLOB 字节长度与小写十六进制','只检查字符长度和 GLOB']
    ]
    changes.each do |before, after|
      with_fixture do |root|
        change_text(root) { |body| body.sub(before, after) }
        assert_cli_error(root, 'rfc_persistence_invariants_invalid')
      end
    end
  end

  def test_public_cli_rejects_sql_byte_hash_marker_and_fence_drift
    cases = [
      ['rfc_sql_bytes_mismatch', proc { |s| s.sub('-- PROPOSED：', '-- 注释变化：') }],
      ['rfc_sql_hash_invalid', proc { |s| s.sub(/SQL SHA-256：[a-f0-9]{64}/, 'SQL SHA-256：' + '0' * 64) }],
      ['rfc_sql_region_invalid', proc { |s| s + "\n<!-- RFC-SQL-BEGIN -->\n" }],
      ['rfc_sql_region_invalid', proc { |s| s.sub('<!-- RFC-SQL-END -->', '') }],
      ['rfc_sql_region_invalid', proc { |s| s.sub('```sql', '```SQL') }],
      ['rfc_sql_region_invalid', proc { |s| s + "\n```sql\nSELECT 1;\n```\n" }],
      ['rfc_sql_bytes_mismatch', proc { |s| s.sub("COMMIT;\n```", "COMMIT;\r\n```") }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_text(root, &mutation)
        assert_cli_error(root, code)
      end
    end
    with_fixture do |root|
      path = File.join(root, SQL)
      before = File.binread(path)
      after = before + "\n"
      refute_equal before, after
      File.binwrite(path, after)
      assert_cli_error(root, 'rfc_sql_bytes_mismatch')
    end
  end

  def test_public_cli_rejects_business_activation_authority_protocol_and_fault_reversals
    cases = [
      ['rfc_business_protocol_invalid', proc { |s| s.sub('| 同一事务执行完成事实 CAS 与事件 |', '| 先提交 CAS 后补事件 |') }],
      ['rfc_activation_protocol_invalid', proc { |s| s.sub('| 新 generation CAS 与同 Unit 兼容历史目标 |', '| 改写旧 generation |') }],
      ['rfc_authority_protocol_invalid', proc { |s| s.sub('| 自动重发或自动清理 |', '| 允许重发 |') }],
      ['rfc_cross_database_protocol_invalid', proc { |s| s.sub('| CAS 零行追加或事件失败仍提交 |', '| None |') }],
      ['rfc_fault_matrix_invalid', proc { |s| s.sub(/^\| after_business_commit \|.*\n/, '') }],
      ['rfc_persistence_reason_missing', proc { |s| s.sub(/^\| finalizer.completed \|.*\n/, '') }],
      ['rfc_outbox_bytes_invalid', proc { |s| s.sub('| 只读取原字节；禁止重新 provider/LLM/render |', '| 重启重新调用 provider/LLM/render |') }]
    ]
    cases.each do |code, mutation|
      with_fixture do |root|
        change_text(root, &mutation)
        assert_cli_error(root, code)
      end
    end
  end

  def test_sql_schema_is_executable_versioned_and_repeatable_without_data_loss
    with_database do |db, ddl|
      assert_equal "1\n", sql_ok(db, 'SELECT version FROM push_foundation_schema;')
      objects = sql_ok(db, "SELECT type || ':' || name FROM sqlite_master WHERE name LIKE 'push_%' ORDER BY name;")
      %w[push_intents push_intent_transitions push_activation_manifests push_promotion_journal].each do |name|
        assert_includes objects, 'table:' + name
      end
      assert_includes objects, 'index:push_intents_recovery'
      assert_operator objects.scan('trigger:').length, :>=, 9
      assert_equal "1\n", sql_ok(db, 'PRAGMA foreign_keys;')
      refute_empty sql_ok(db, 'PRAGMA foreign_key_list(push_intent_transitions);')
      refute_empty sql_ok(db, 'PRAGMA foreign_key_list(push_promotion_journal);')
      sql_ok(db, insert_intent)
      sql_ok(db, ddl)
      assert_equal "#{sha_id('intent-1')}|PendingDispatch|0\n", sql_ok(db, 'SELECT intent_id,state,version FROM push_intents;')
    end
  end

  def test_initial_non_sending_facts_require_no_prepared_push_group
    %w[NoData Disabled].each do |kind|
      with_database do |db, ddl|
        group = %w[prepared_push_bytes rendered_bytes payload_sha256 rendered_sha256].map { |key| [key, 'NULL'] }.to_h
        values = group.merge('state'=>"'#{kind}'", 'reason'=>kind == 'NoData' ? "'intent.no_data'" : "'policy.disabled'")
        values['job_decision_kind'] = "'#{kind}'"
        sql_ok(db, insert_intent(values))
        sql_ok(db, ddl)
        assert_equal "#{kind}|1|1\n", sql_ok(db, 'SELECT state,prepared_push_bytes IS NULL,rendered_bytes IS NULL FROM push_intents;')
      end
    end
  end

  def test_business_transition_reason_must_match_intent_and_the_legal_edge
    with_database do |db, _ddl|
      sql_ok(db, insert_intent)
      sql_rejected(db, "UPDATE push_intents SET previous_state=state,state='AwaitingAuthority',version=1,reason='policy.disabled';")
    end
  end

  def test_event_reason_cannot_disagree_with_successful_business_cas
    with_database do |db, _ddl|
      seed_finalizer(db)
      sql_rejected(db, "BEGIN IMMEDIATE; #{finalization_update} #{transition_sql('reason'=>"'policy.disabled'")} COMMIT;")
      assert_equal "AwaitingFinalizer|2\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
    end
  end

  def test_ddl_rejects_an_existing_wrong_schema_version_without_changing_it
    Dir.mktmpdir('rfc-sql-incompatible') do |dir|
      db = File.join(dir, 'wrong.sqlite3')
      sql_ok(db, "CREATE TABLE push_foundation_schema(version INTEGER,description TEXT); INSERT INTO push_foundation_schema VALUES(999,'wrong');")
      sql_rejected(db, File.binread(File.join(ROOT, SQL)))
      assert_equal "999|wrong\n", sql_ok(db, 'SELECT * FROM push_foundation_schema;')
      assert_equal "0\n", sql_ok(db, "SELECT count(*) FROM sqlite_master WHERE name='push_intents';")
    end
  end

  def test_ddl_rejects_a_weakened_same_name_trigger_before_touching_business_rows
    with_database do |db, ddl|
      sql_ok(db, insert_intent)
      before = sql_ok(db, "SELECT sql FROM sqlite_master WHERE name='push_intents_immutable';")
      sql_ok(db, 'DROP TRIGGER push_intents_immutable; CREATE TRIGGER push_intents_immutable BEFORE UPDATE ON push_intents BEGIN SELECT 1; END;')
      refute_equal before, sql_ok(db, "SELECT sql FROM sqlite_master WHERE name='push_intents_immutable';")
      sql_rejected(db, ddl)
      assert_equal "PendingDispatch|0\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
    end
  end

  def test_hashes_reject_embedded_nul_and_non_text_values
    with_database do |db, _ddl|
      nul = "CAST(X'#{('a' * 64 + "\0Z").unpack1('H*')}' AS TEXT)"
      refute_equal "'#{'a' * 64}'", nul
      sql_rejected(db, insert_intent('payload_sha256'=>nul))
      sql_rejected(db, insert_intent('payload_sha256'=>"X'#{('a' * 64).unpack1('H*')}'"))
    end
  end

  def test_intent_and_event_identities_are_sha256_text_not_opaque_labels
    with_database do |db, _ddl|
      sql_rejected(db, insert_intent('intent_id'=>"'short-intent'"))
    end
  end

  def test_transition_event_id_cannot_be_a_short_label
    with_database do |db, _ddl|
      seed_finalizer(db)
      sql_rejected(db, "BEGIN IMMEDIATE; #{finalization_update} #{transition_sql('event_id'=>"'short-event'")} COMMIT;")
    end
  end

  def test_promotion_event_id_cannot_be_a_short_label
    with_database do |db, _ddl|
      sql_ok(db, insert_manifest(1, 'Disabled'))
      sql_rejected(db, insert_journal(1, 'Initialize', 'event_id'=>"'short-promotion'"))
    end
  end

  def test_outbox_preserves_immutable_dispatch_bytes_before_durable_reserve
    with_database do |db, ddl|
      columns = sql_ok(db, 'PRAGMA table_info(push_intents);')
      assert_includes columns, 'prepared_push_bytes'
      assert_includes columns, 'rendered_bytes'
      sql_ok(db, insert_intent)
      expected = "7B7D|00FF\n"
      assert_equal expected, sql_ok(db, 'SELECT hex(prepared_push_bytes),hex(rendered_bytes) FROM push_intents;')
      sql_ok(db, ddl)
      assert_equal expected, sql_ok(db, 'SELECT hex(prepared_push_bytes),hex(rendered_bytes) FROM push_intents;')
      %w[prepared_push_bytes rendered_bytes].each do |field|
        sql_rejected(db, "UPDATE push_intents SET #{field}=X'42',previous_state=state,version=1;")
        sql_rejected(db, insert_intent(field=>"X'42'").sub('INSERT INTO', 'INSERT OR REPLACE INTO'))
      end
      assert_equal expected, sql_ok(db, 'SELECT hex(prepared_push_bytes),hex(rendered_bytes) FROM push_intents;')
    end
  end

  def test_non_send_group_is_complete_immutable_and_cannot_enter_send_finalization
    %w[NoData Disabled].each do |kind|
      with_database do |db, ddl|
        group = %w[prepared_push_bytes rendered_bytes payload_sha256 rendered_sha256].map { |key| [key,'NULL'] }.to_h
        values = group.merge('job_decision_kind'=>"'#{kind}'",'state'=>"'#{kind}'",
          'reason'=>kind == 'NoData' ? "'intent.no_data'" : "'policy.disabled'")
        sql_rejected(db, insert_intent(values.reject { |key, _| group.key?(key) }))
        group.each_key do |field|
          material = field.end_with?('_bytes') ? "X'42'" : "'#{'a' * 64}'"
          sql_rejected(db, insert_intent(values.merge(field=>material)))
          sql_rejected(db, insert_intent(field=>'NULL'))
        end
        sql_ok(db, insert_intent(values))
        event = transition_sql('event_id'=>"'#{sha_id('non-send')}'",'from_state'=>"'#{kind}'",'to_state'=>"'ResolutionRequired'",
          'expected_version'=>'0','result_version'=>'1','previous_sha256'=>'NULL','canonical_sha256'=>"'#{'1' * 64}'",
          'reason'=>"'intent.payload_conflict'",'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL')
        sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='ResolutionRequired',version=1,reason='intent.payload_conflict',updated_at=2 WHERE version=0 AND lease_generation=0; #{event} COMMIT;")
        sql_ok(db, ddl)
        assert_equal "#{kind}|ResolutionRequired|1|1\n", sql_ok(db, 'SELECT job_decision_kind,state,prepared_push_bytes IS NULL,payload_sha256 IS NULL FROM push_intents;')
        sql_rejected(db, "UPDATE push_intents SET previous_state=state,state='AwaitingFinalizer',version=2,reason='intent.authority_verified';")
        sql_rejected(db, "UPDATE push_intents SET previous_state=state,state='Completed',version=2,reason='finalizer.completed';")
        sql_rejected(db, "UPDATE push_intents SET previous_state=state,version=2,reason='intent.dispatch_claimed',job_decision_kind='Ready';")
      end
    end
  end

  def test_ready_derived_no_data_retains_the_original_material_and_reason_binding
    with_database do |db, ddl|
      sql_ok(db, insert_intent)
      event = transition_sql('event_id'=>"'#{sha_id('ready-no-data')}'",'from_state'=>"'PendingDispatch'",'to_state'=>"'NoData'",
        'expected_version'=>'0','result_version'=>'1','previous_sha256'=>'NULL','canonical_sha256'=>"'#{'1' * 64}'",
        'reason'=>"'intent.no_data'",'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL')
      sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='NoData',version=1,reason='intent.no_data',updated_at=2 WHERE version=0; #{event} COMMIT;")
      sql_ok(db, ddl)
      assert_equal "Ready|NoData|7B7D|00FF\n", sql_ok(db, 'SELECT job_decision_kind,state,hex(prepared_push_bytes),hex(rendered_bytes) FROM push_intents;')
      assert_equal "intent.no_data|intent.no_data\n", sql_ok(db, 'SELECT i.reason,t.reason FROM push_intents i JOIN push_intent_transitions t ON t.intent_id=i.intent_id AND t.result_version=i.version;')
    end
  end

  def test_raw_sqlite_cli_guards_do_not_repair_or_change_incompatible_databases
    cases = [
      "DROP TRIGGER push_intents_immutable;",
      "DROP TRIGGER push_foundation_objects_update; CREATE TRIGGER push_foundation_objects_update BEFORE UPDATE ON push_foundation_objects BEGIN SELECT 1; END;",
      "CREATE TRIGGER extra_hook BEFORE UPDATE ON push_intents BEGIN SELECT 1; END;",
      "CREATE INDEX extra_index ON push_intents(state);"
    ]
    cases.each do |mutation|
      with_database do |db, ddl|
        sql_ok(db, insert_intent)
        before = sql_ok(db, "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name;")
        sql_ok(db, mutation)
        changed = sql_ok(db, "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name;")
        refute_equal before, changed
        # 等价于 sqlite3 TEMP_DB < file：没有 helper 的 -bail 或 PRAGMA 前缀。
        out, err, result = Open3.capture3('/usr/bin/sqlite3', db, stdin_data: ddl)
        refute_equal 0, result.exitstatus, out
        assert_includes err, 'activation.manifest_mismatch'
        assert_equal changed, sql_ok(db, "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name;")
        assert_equal "PendingDispatch|0\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      end
    end
    Dir.mktmpdir('rfc-sql-signature') do |dir|
      db = File.join(dir, 'wrong-signature.sqlite3')
      sql_ok(db, "CREATE TABLE push_foundation_schema(version INTEGER,description TEXT,schema_signature TEXT); INSERT INTO push_foundation_schema VALUES(1,'push-foundation-v1','#{'0' * 64}');")
      before = sql_ok(db, 'SELECT name,sql FROM sqlite_master ORDER BY name;')
      out, _err, result = Open3.capture3('/usr/bin/sqlite3', db, stdin_data: File.binread(File.join(ROOT, SQL)))
      refute_equal 0, result.exitstatus, out
      assert_equal before, sql_ok(db, 'SELECT name,sql FROM sqlite_master ORDER BY name;')
    end
  end

  def test_raw_sqlite_cli_keeps_unrelated_legacy_tables_and_can_repeat_in_one_connection
    Dir.mktmpdir('rfc-sql-legacy') do |dir|
      db = File.join(dir, 'legacy.sqlite3')
      sql_ok(db, "CREATE TABLE push_legacy(value TEXT); INSERT INTO push_legacy VALUES('preserve'); CREATE INDEX push_legacy_index ON push_legacy(value); CREATE TRIGGER push_legacy_trigger BEFORE UPDATE ON push_legacy BEGIN SELECT 1; END;")
      ddl = File.binread(File.join(ROOT, SQL))
      out, err, result = Open3.capture3('/usr/bin/sqlite3', db, stdin_data: ddl + ddl)
      assert_equal 0, result.exitstatus, out + err
      assert_equal "preserve\n", sql_ok(db, 'SELECT value FROM push_legacy;')
      assert_equal "25\n", sql_ok(db, 'SELECT count(*) FROM push_foundation_objects;')
    end
  end

  def test_every_persisted_hash_check_rejects_nul_and_blob_values
    with_database do |db, _ddl|
      definitions = sql_ok(db, "SELECT sql FROM sqlite_master WHERE type='table' AND name LIKE 'push_%';")
      columns = definitions.lines.select { |line| line.match?(/^  (\w+_sha256|schema_signature|build_commit|intent_id|event_id) TEXT/) }
      assert_operator columns.length, :>=, 23
      columns.each_with_index do |line, index|
        name = line.strip.split.first
        declaration = line.strip.sub(/,\z/, '').sub(' PRIMARY KEY', '').sub(/ REFERENCES \w+\(\w+\)/, '')
        length = name == 'build_commit' ? 40 : 64
        valid = name == 'schema_signature' ? sql_ok(db, 'SELECT schema_signature FROM push_foundation_schema;').strip : 'a' * length
        sql_ok(db, "CREATE TABLE hash_probe_#{index}(#{declaration});")
        sql_ok(db, "INSERT INTO hash_probe_#{index} VALUES('#{valid}');")
        nul = "CAST(X'#{(valid + "\0Z").unpack1('H*')}' AS TEXT)"
        refute_equal "'#{valid}'", nul
        sql_rejected(db, "INSERT INTO hash_probe_#{index} VALUES(#{nul});")
        sql_rejected(db, "INSERT INTO hash_probe_#{index} VALUES(X'#{valid.unpack1('H*')}');")
      end
      %w[intent.created intent.dispatch_claimed].each do |reason|
        sql_rejected(db, insert_intent('reason'=>"CAST(X'#{(reason + "\0hidden").unpack1('H*')}' AS TEXT)"))
      end
      sql_rejected(db, insert_intent('business_date'=>"CAST(X'#{("2026-09-06\0hidden").unpack1('H*')}' AS TEXT)"))
    end
  end

  def test_sql_rejects_illegal_states_hashes_reasons_times_and_foreign_keys
    with_database do |db, _ddl|
      ["state='Accepted'", "payload_sha256='GG'", "payload_sha256='#{'A' * 64}'",
       "reason='invented.ok'", "reason='intent.bad.code'", "created_at=-1",
       "prepared_push_bytes=X''", "rendered_bytes='not-a-blob'",
       "business_date='2026-02-30'", "lease_owner='foreign'"].each do |assignment|
        field, value = assignment.split('=', 2)
        sql_rejected(db, insert_intent(field => value))
      end
      sql_ok(db, insert_intent)
      sql_rejected(db, transition_sql('intent_id' => "'missing'"))
      sql_rejected(db, insert_manifest(1, 'Bogus'))
      sql_rejected(db, insert_manifest(1, 'Disabled', 'catalog_sha256' => "'bad'"))
      sql_ok(db, insert_manifest(1, 'Disabled'))
      sql_rejected(db, insert_journal(1, 'Invented'))
      sql_rejected(db, insert_journal(1, 'Initialize', 'actor' => "''"))
      sql_rejected(db, insert_journal(1, 'Initialize', 'to_manifest_sha256' => "'#{'f' * 64}'"))
    end
  end

  def test_intent_identity_and_material_cannot_be_replaced_or_updated
    with_database do |db, _ddl|
      sql_ok(db, insert_intent)
      %w[namespace unit_id completion_owner occurrence_key source_contract_id source_contract_sha256
         payload_sha256 rendered_sha256 evidence_sha256 template_sha256 durable_decision_id subject audience].each do |field|
        value = field.end_with?('sha256') ? "'#{'b' * 64}'" : "'changed'"
        sql_rejected(db, "UPDATE push_intents SET #{field}=#{value},version=version+1,previous_state=state WHERE intent_id='#{sha_id('intent-1')}';")
      end
      drift = insert_intent('payload_sha256' => "'#{'b' * 64}'")
      refute_equal insert_intent, drift
      sql_rejected(db, drift)
      sql_rejected(db, drift.sub('INSERT INTO', 'INSERT OR REPLACE INTO'))
      alias_identity = insert_intent('intent_id'=>"'#{sha_id('other-id')}'", 'payload_sha256'=>"'#{'b' * 64}'")
      refute_equal drift, alias_identity
      sql_rejected(db, alias_identity.sub('INSERT INTO', 'INSERT OR REPLACE INTO'))
      assert_equal "#{'a' * 64}\n", sql_ok(db, 'SELECT payload_sha256 FROM push_intents;')
    end
  end

  def test_finalization_cas_and_transition_commit_together_and_zero_cas_appends_nothing
    with_database do |db, _ddl|
      seed_finalizer(db)
      sql_ok(db, "BEGIN IMMEDIATE;\n#{finalization_update}\n#{transition_sql}\nCOMMIT;")
      assert_equal "Completed|3\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      assert_equal "3\n", sql_ok(db, 'SELECT count(*) FROM push_intent_transitions;')
      # 公开 SQLite 事务接口中的 changes() 门禁；零行不能尝试追加事件。
      sql_ok(db, "BEGIN IMMEDIATE;\n#{finalization_update}\n#{transition_sql.sub(' VALUES (', ' SELECT ').sub(/\);\z/, ' WHERE changes()=1;')}\nCOMMIT;")
      assert_equal "3\n", sql_ok(db, 'SELECT count(*) FROM push_intent_transitions;')
    end
  end

  def test_transition_failure_rolls_back_the_prior_cas_and_committed_events_are_immutable
    with_database do |db, _ddl|
      seed_finalizer(db)
      [transition_sql('result_version' => '99'), transition_sql('canonical_sha256' => "'BAD'"),
       transition_sql('terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL'),
       transition_sql('previous_sha256'=>"'#{'f' * 64}'"),
       transition_sql('event_id' => "'#{sha_id('event-2')}'")].each do |event|
        refute_equal transition_sql, event
        sql_rejected(db, "BEGIN IMMEDIATE;\n#{finalization_update}\n#{event}\nCOMMIT;")
        assert_equal "AwaitingFinalizer|2\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
        assert_equal "2\n", sql_ok(db, 'SELECT count(*) FROM push_intent_transitions;')
      end
      sql_rejected(db, "UPDATE push_intent_transitions SET actor='forged';")
      sql_rejected(db, 'DELETE FROM push_intent_transitions;')
    end
  end

  def test_activation_is_versioned_and_journal_is_an_independent_append_only_fact
    with_database do |db, _ddl|
      sql_ok(db, insert_manifest(1, 'Disabled'))
      assert_equal "0\n", sql_ok(db, 'SELECT count(*) FROM push_promotion_journal;')
      sql_ok(db, insert_journal(1, 'Initialize'))
      sql_rejected(db, insert_manifest(2, 'Active'))
      sql_ok(db, insert_manifest(2, 'Shadow'))
      sql_ok(db, insert_journal(2, 'EnterShadow'))
      sql_ok(db, insert_manifest(3, 'Active'))
      sql_ok(db, insert_journal(3, 'Activate'))
      sql_ok(db, insert_manifest(4, 'Disabled', 'rollback_target_sha256' => "'#{manifest_hash(1)}'"))
      sql_ok(db, insert_journal(4, 'Rollback', 'rollback_target_sha256' => "'#{manifest_hash(1)}'"))
      %w[push_activation_manifests push_promotion_journal].each do |table|
        sql_rejected(db, "UPDATE #{table} SET unit_id='forged';")
        sql_rejected(db, "DELETE FROM #{table};")
      end
      assert_equal "4|4\n", sql_ok(db, 'SELECT (SELECT count(*) FROM push_activation_manifests),count(*) FROM push_promotion_journal;')
    end
  end

  def test_lease_takeover_requires_expiry_generation_and_version_cas
    with_database do |db, _ddl|
      seed_finalizer(db)
      sql_rejected(db, "UPDATE push_intents SET reason='intent.dispatch_claimed',previous_state=state,version=version+1,lease_owner='other',lease_generation=2,lease_until=200,updated_at=5 WHERE version=2 AND lease_generation=1;")
      sql_rejected(db, "UPDATE push_intents SET reason='intent.dispatch_claimed',previous_state=state,version=version+1,lease_owner='other',lease_until=200,updated_at=100 WHERE version=2 AND lease_generation=1;")
      sql_ok(db, "UPDATE push_intents SET reason='intent.dispatch_claimed',previous_state=state,version=version+1,lease_owner='other',lease_generation=2,lease_until=200,updated_at=100 WHERE version=99 AND lease_generation=1;")
      assert_equal "finalizer|1|2\n", sql_ok(db, 'SELECT lease_owner,lease_generation,version FROM push_intents;')
      event = transition_sql('from_state'=>"'AwaitingFinalizer'",'to_state'=>"'AwaitingFinalizer'",
        'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL','occurred_at'=>'100','reason'=>"'intent.dispatch_claimed'")
      sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET reason='intent.dispatch_claimed',previous_state=state,version=version+1,lease_owner='other',lease_generation=2,lease_until=200,updated_at=100 WHERE version=2 AND lease_generation=1 AND lease_until<=100; #{event} COMMIT;")
      assert_equal "other|2|3\n", sql_ok(db, 'SELECT lease_owner,lease_generation,version FROM push_intents;')
    end
  end

  def test_normal_activation_cycle_and_cross_unit_journal_binding
    with_database do |db, _ddl|
      [['Disabled','Initialize'],['Shadow','EnterShadow'],['Active','Activate'],
       ['Draining','Drain'],['Disabled','Disable']].each_with_index do |pair, index|
        generation = index + 1
        sql_ok(db, insert_manifest(generation, pair[0]))
        sql_rejected(db, insert_journal(generation, pair[1], 'unit_id'=>"'other-unit'"))
        sql_rejected(db, insert_journal(generation, pair[1], 'evidence_sha256'=>"'#{'b' * 64}'"))
        sql_ok(db, insert_journal(generation, pair[1]))
      end
      assert_equal "5|Disable\n", sql_ok(db, 'SELECT generation,action FROM push_promotion_journal ORDER BY generation DESC LIMIT 1;')
      assert_empty sql_ok(db, 'PRAGMA foreign_key_check;')
    end
  end

  def test_sql_supports_versioned_manual_resolution_without_replacing_identity
    with_database do |db, _ddl|
      seed_finalizer(db)
      [['AwaitingFinalizer','ResolutionRequired','intent.payload_conflict'],
       ['ResolutionRequired','AwaitingFinalizer','intent.authority_verified']].each_with_index do |row, index|
        version = index + 3
        event = transition_sql('event_id'=>"'#{sha_id('event-' + version.to_s)}'",'from_state'=>"'#{row[0]}'",'to_state'=>"'#{row[1]}'",
          'expected_version'=>(version-1).to_s,'result_version'=>version.to_s,
          'previous_sha256'=>"'#{(version-1).to_s * 64}'",'canonical_sha256'=>"'#{version.to_s * 64}'",
          'actor'=>"'operator'",'reason'=>"'#{row[2]}'",'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL','occurred_at'=>'10')
        sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='#{row[1]}',version=version+1,reason='#{row[2]}',updated_at=10 WHERE intent_id='#{sha_id('intent-1')}' AND version=#{version-1} AND lease_generation=1; #{event} COMMIT;")
      end
      assert_equal "#{sha_id('intent-1')}|decision-1|AwaitingFinalizer|4\n", sql_ok(db, 'SELECT intent_id,durable_decision_id,state,version FROM push_intents;')
      sql_rejected(db, "UPDATE push_intents SET previous_state=state,state='PendingDispatch',version=5;")
    end
  end

  def test_sql_guard_oracles_detect_real_weakened_schema_mutants
    # 这些缺陷库仍在独立临时目录。每个 mutation 先证明改变字节；
    # 捕获的是公开 SQLite 行为断言的真实失败，不调用 validator 私有 helper。
    ddl = File.binread(File.join(ROOT, SQL))
    mutants = {
      'missing_table' => [proc { |s| s.gsub('push_foundation_schema', 'absent_schema') },
                          proc { |db| assert_equal "1\n", sql_ok(db, 'SELECT version FROM push_foundation_schema;') }],
      'illegal_state' => [proc { |s| s.gsub("'ResolutionRequired'", "'ResolutionRequired','Bogus'").sub("NEW.state='PendingDispatch'", "NEW.state IN ('PendingDispatch','Bogus')") },
                         proc { |db| sql_rejected(db, insert_intent('state'=>"'Bogus'")) }],
      'payload_overwrite' => [proc { |s| s.sub(/CREATE TRIGGER IF NOT EXISTS push_intents_immutable\n.*?END;\n/m, '') },
                             proc { |db| sql_ok(db, insert_intent); sql_rejected(db, "UPDATE push_intents SET reason='intent.dispatch_claimed',previous_state=state,version=1,payload_sha256='#{'b' * 64}';") }],
      'journal_update' => [proc { |s| s.sub(/CREATE TRIGGER IF NOT EXISTS push_promotion_journal_update\n.*?END;\n/m, '') },
                          proc { |db| sql_ok(db, insert_manifest(1,'Disabled')); sql_ok(db, insert_journal(1,'Initialize')); sql_rejected(db, "UPDATE push_promotion_journal SET actor='forged';") }],
      'journal_delete' => [proc { |s| s.sub(/CREATE TRIGGER IF NOT EXISTS push_promotion_journal_delete\n.*?END;\n/m, '') },
                          proc { |db| sql_ok(db, insert_manifest(1,'Disabled')); sql_ok(db, insert_journal(1,'Initialize')); sql_rejected(db, 'DELETE FROM push_promotion_journal;') }],
      'not_repeatable' => [proc { |s| s.sub('CREATE TABLE IF NOT EXISTS push_intents', 'CREATE TABLE push_intents') },
                          proc { |db, mutant| sql_ok(db, insert_intent); sql_ok(db, mutant) }]
    }
    mutants.each do |name, pair|
      mutation, oracle = pair
      mutant = mutation.call(ddl)
      removed = {'payload_overwrite'=>'push_intents_immutable', 'journal_update'=>'push_promotion_journal_update',
                 'journal_delete'=>'push_promotion_journal_delete'}[name]
      # 同时构造错误的首次登记清单，才能单独检验业务保护；真实已登记库另测拒绝漂移。
      mutant = mutant.sub("  ('#{removed}','trigger'),\n", '') if removed
      refute_equal ddl, mutant, name
      Dir.mktmpdir('rfc-sql-mutant') do |dir|
        db = File.join(dir, 'mutant.sqlite3')
        sql_ok(db, mutant)
        assert_raises(Minitest::Assertion, name) do
          oracle.call(db, mutant)
        end
      end
    end
  end

  def test_sql_document_paths_reject_symlinks_directories_and_missing_files
    with_fixture do |root|
      file = File.join(root, SQL)
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

  private

  def with_database
    Dir.mktmpdir('rfc-sql-test') do |dir|
      db = File.join(dir, 'foundation.sqlite3')
      ddl = File.exist?(File.join(ROOT, SQL)) ? File.binread(File.join(ROOT, SQL)) : ''
      sql_ok(db, ddl)
      yield db, ddl
    end
  end

  def sqlite(db, script)
    Open3.capture3('/usr/bin/sqlite3', '-batch', '-bail', db,
                   stdin_data: "PRAGMA foreign_keys=ON;\nPRAGMA recursive_triggers=ON;\n" + script)
  end

  def sql_ok(db, script)
    out, err, result = sqlite(db, script)
    assert_equal 0, result.exitstatus, err
    out
  end

  def sql_rejected(db, script)
    out, err, result = sqlite(db, script)
    refute_equal 0, result.exitstatus, out
    refute_empty err
  end

  def sha_id(label)
    Digest::SHA256.hexdigest(label)
  end

  def insert_row(table, values)
    "INSERT INTO #{table} (#{values.keys.join(',')}) VALUES (#{values.values.join(',')});"
  end

  def insert_intent(overrides = {})
    values = {'intent_id'=>"'#{sha_id('intent-1')}'",'namespace'=>"'test'",'unit_id'=>"'MU-p01'",'business_date'=>"'2026-09-06'",
      'occurrence_family'=>"'preopen'",'occurrence_key'=>"'P01'",'completion_owner'=>"'owner-1'",
      'source_contract_id'=>"'source-v1'",'subject'=>"'market'",'audience'=>"'test'",'durable_decision_id'=>"'decision-1'",
      'job_decision_kind'=>"'Ready'",'prepared_push_bytes'=>"X'7B7D'",'rendered_bytes'=>"X'00FF'",
      'state'=>"'PendingDispatch'",'reason'=>"'intent.created'",'created_at'=>'1','updated_at'=>'1'}
    %w[payload rendered evidence template source_contract].each { |name| values[name + '_sha256'] = "'#{'a' * 64}'" }
    insert_row('push_intents', values.merge(overrides))
  end

  def transition_sql(overrides = {})
    values = {'event_id'=>"'#{sha_id('event-3')}'",'intent_id'=>"'#{sha_id('intent-1')}'",'from_state'=>"'AwaitingFinalizer'",'to_state'=>"'Completed'",
      'expected_version'=>'2','result_version'=>'3','previous_sha256'=>"'#{'2' * 64}'",'canonical_sha256'=>"'#{'3' * 64}'",
      'actor'=>"'finalizer'",'reason'=>"'finalizer.completed'",'terminal_ref_id'=>"'ref-1'",
      'terminal_binding_sha256'=>"'#{'a' * 64}'",'occurred_at'=>'4'}
    insert_row('push_intent_transitions', values.merge(overrides))
  end

  def seed_finalizer(db)
    sql_ok(db, insert_intent)
    [['PendingDispatch','AwaitingAuthority'], ['AwaitingAuthority','AwaitingFinalizer']].each_with_index do |pair, index|
      version = index + 1
      event = transition_sql('event_id'=>"'#{sha_id('event-' + version.to_s)}'",'from_state'=>"'#{pair[0]}'",'to_state'=>"'#{pair[1]}'",
        'expected_version'=>index.to_s,'result_version'=>version.to_s,'previous_sha256'=>index.zero? ? 'NULL' : "'#{'1' * 64}'",
        'canonical_sha256'=>"'#{version.to_s * 64}'",'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL',
        'reason'=>index.zero? ? "'intent.dispatch_claimed'" : "'intent.authority_verified'")
      lease = index.zero? ? ",lease_owner='finalizer',lease_until=100,lease_generation=1" : ''
      sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='#{pair[1]}',version=version+1,updated_at=#{version + 1},reason=#{index.zero? ? "'intent.dispatch_claimed'" : "'intent.authority_verified'"}#{lease} WHERE version=#{index}; #{event} COMMIT;")
    end
  end

  def finalization_update
    "UPDATE push_intents SET reason='finalizer.completed',previous_state=state,state='Completed',version=version+1,updated_at=4 WHERE intent_id='#{sha_id('intent-1')}' AND state='AwaitingFinalizer' AND version=2 AND lease_owner='finalizer' AND lease_generation=1 AND lease_until>4;"
  end

  def manifest_hash(generation)
    generation.to_s(16).rjust(64, '0')
  end

  def insert_manifest(generation, state, overrides = {})
    values = {'manifest_sha256'=>"'#{manifest_hash(generation)}'",'unit_id'=>"'MU-p01'",'generation'=>generation.to_s,
      'previous_manifest_sha256'=>generation == 1 ? 'NULL' : "'#{manifest_hash(generation - 1)}'",
      'desired_state'=>"'#{state}'",'physical_owner'=>"'owner-1'",'build_commit'=>"'#{'a' * 40}'",'approved_by'=>"'operator'",'approved_at'=>'1',
      'window_start'=>'1','window_end'=>'100','created_at'=>'1'}
    %w[build catalog business_schema durable_schema template source_contract evidence].each { |name| values[name + '_sha256'] = "'#{'a' * 64}'" }
    insert_row('push_activation_manifests', values.merge(overrides))
  end

  def insert_journal(generation, action, overrides = {})
    insert_row('push_promotion_journal', {'event_id'=>"'#{sha_id('promotion-' + generation.to_s)}'",'unit_id'=>"'MU-p01'",'generation'=>generation.to_s,
      'from_manifest_sha256'=>generation == 1 ? 'NULL' : "'#{manifest_hash(generation - 1)}'",
      'to_manifest_sha256'=>"'#{manifest_hash(generation)}'",'actor'=>"'operator'",'action'=>"'#{action}'",
      'window_start'=>'1','window_end'=>'100','evidence_sha256'=>"'#{'a' * 64}'",'occurred_at'=>'2',
      'previous_sha256'=>generation == 1 ? 'NULL' : "'#{manifest_hash(generation - 1)}'",'canonical_sha256'=>"'#{manifest_hash(generation)}'",
      'reason'=>"'activation.applied'"}.merge(overrides))
  end

  def with_fixture
    Dir.mktmpdir('rfc-spec-test') do |root|
      inputs = JSON.parse(File.read(File.join(ROOT, 'docs/push-system/rfc-input-manifest.v1.json')))
      paths = [RFC] + DEPENDENCIES.map { |name| 'docs/push-system/' + name } + inputs['inputs'].map { |item| item['path'] }
      paths << SQL if File.file?(File.join(ROOT, SQL))
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
