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

  def test_strict_reports_exact_release_blockers_and_preserves_content_errors
    with_fixture do |root|
      expected = %w[rfc_status_provisional wbs_status_provisional rfc_html_missing ci_rfc_gate_missing]
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
      assert_equal 1, result.exitstatus, out + err
      assert_equal expected, out.lines.map(&:strip)
      assert_empty err
      change_metadata(root) { |m| m['counts']['kinds'] = 64 }
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
      assert_equal 1, result.exitstatus, out + err
      assert_equal ['rfc_counts_invalid'] + expected, out.lines.map(&:strip)
      assert_empty err
    end
  end

  def test_strict_release_artifacts_accept_regular_html_and_a_real_ci_run_step
    with_fixture do |root|
      File.write(File.join(root, 'docs/push-system/push-system-implementation-rfc.html'), '<html></html>')
      FileUtils.mkdir_p(File.join(root, '.github/workflows'))
      File.write(File.join(root, '.github/workflows/ci.yml'), "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          ruby scripts/architecture-docs/check.rb --check\n")
      2.times do
        out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
        assert_equal 1, result.exitstatus, out + err
        assert_equal %w[rfc_status_provisional wbs_status_provisional], out.lines.map(&:strip)
        assert_empty err
      end
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, result.exitstatus, out + err
      assert_equal "rfc_spec_valid\n", out
    end
  end

  def test_strict_ci_rejects_comments_prose_other_commands_and_invalid_yaml
    [
      "# ruby scripts/architecture-docs/check.rb --check\non: push\njobs: {}\n",
      "on: push\ndescription: ruby scripts/architecture-docs/check.rb --check\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - name: ruby scripts/architecture-docs/check.rb --check\n        run: echo skipped\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ruby scripts/architecture-docs/check.rb --check\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check-rfc.rb --check\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          cat <<'TEXT'\n          ruby scripts/architecture-docs/check.rb --check\n          TEXT\n",
      "on: push\njobs: [broken\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n        run: ruby scripts/architecture-docs/check.rb --check\n",
      "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - if: false\n        run: ruby scripts/architecture-docs/check.rb --check\n"
    ].each do |workflow|
      with_fixture do |root|
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        File.write(File.join(root, '.github/workflows/ci.yml'), workflow)
        assert_cli_error(root, 'ci_rfc_gate_missing', '--check')
      end
    end
  end

  def test_ci_mapping_recognizes_existing_workflow_without_changing_its_triggers
    original = File.read(File.join(ROOT, '.github/workflows/ci.yml'))
    marker = "      - name: Format\n"
    assert_includes original, marker
    workflow = original.sub(marker,
      "      - name: Check architecture documents\n" \
      "        run: ruby scripts/architecture-docs/check.rb --check\n\n" + marker)
    refute_equal original, workflow
    with_fixture do |root|
      FileUtils.mkdir_p(File.join(root, '.github/workflows'))
      File.write(File.join(root, '.github/workflows/ci.yml'), workflow)
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
      assert_equal 1, status.exitstatus, out + err
      assert_equal %w[rfc_status_provisional wbs_status_provisional rfc_html_missing], out.lines.map(&:strip)
      assert_empty err
    end
    assert_equal original, File.read(File.join(ROOT, '.github/workflows/ci.yml'))
  end

  def test_ci_mapping_accepts_empty_configuration_and_literal_branch_filters
    [
      "on:\n  push:",
      "on:\n  pull_request: {}\n  workflow_dispatch: null",
      "'on':\n  push: ~\n  pull_request: Null\n  workflow_dispatch: NULL",
      "on:\n  push:\n    branches: [main, master]",
      "on:\n  pull_request:\n    branches: ['release/**', main]",
      "on:\n  push:\n    branches: ['123', 'on']"
    ].each do |trigger|
      with_fixture do |root|
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        workflow = trigger + "\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n"
        File.write(File.join(root, '.github/workflows/ci.yml'), workflow)
        out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
        assert_equal 1, status.exitstatus, out + err
        assert_equal %w[rfc_status_provisional wbs_status_provisional rfc_html_missing], out.lines.map(&:strip)
        assert_empty err
      end
    end
  end

  {
    'unknown_event' => "on:\n  schedule:",
    'duplicate_event' => "on:\n  push:\n  push:",
    'quoted_empty_config' => "on:\n  push: ''",
    'quoted_null_config' => "on:\n  push: 'null'",
    'false_config' => "on:\n  push: false",
    'sequence_config' => "on:\n  push: []",
    'unknown_filter' => "on:\n  push:\n    mystery: [main]",
    'dispatch_branch_filter' => "on:\n  workflow_dispatch:\n    branches: [main]",
    'scalar_branches' => "on:\n  push:\n    branches: main",
    'empty_branches' => "on:\n  push:\n    branches: []",
    'null_branch' => "on:\n  push:\n    branches: [null]",
    'numeric_branch' => "on:\n  push:\n    branches: [123]",
    'boolean_branch' => "on:\n  push:\n    branches: [on]",
    'empty_branch' => "on:\n  push:\n    branches: ['']",
    'expression_branch' => "on:\n  push:\n    branches: ['${{ github.ref_name }}']",
    'nested_branch' => "on:\n  push:\n    branches: [[main]]",
    'duplicate_branch' => "on:\n  push:\n    branches: [main, main]",
    'only_negative_branch' => "on:\n  push:\n    branches: ['!main']",
    'cancelled_literal_branches' => "on:\n  push:\n    branches: [main, '!main']",
    'cancelled_all_branches' => "on:\n  push:\n    branches: ['**', '!**']",
    'mixed_positive_and_negative_filters' => "on:\n  pull_request:\n    branches: ['release/**', '!release/private/**']",
    'duplicate_filter' => "on:\n  push:\n    branches: [main]\n    branches: [master]",
    'tagged_mapping' => "on: !!map\n  push:",
    'tagged_branch' => "on:\n  push:\n    branches: [!!str main]",
    'alias_config' => "on:\n  push: &filter {}\n  pull_request: *filter"
  }.each do |name, trigger|
    define_method("test_ci_mapping_rejects_#{name}") do
      with_fixture do |root|
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        workflow = trigger + "\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n"
        File.write(File.join(root, '.github/workflows/ci.yml'), workflow)
        assert_cli_error(root, 'ci_rfc_gate_missing', '--check')
      end
    end
  end

  ci_execution_bypasses = {
    'job_expression_false'=>['job', "if: '${{ false }}'"],
    'step_expression_false'=>['step', "if: '${{ false }}'"],
    'job_boolean_false'=>['job', 'if: false'],
    'job_string_true'=>['job', "if: 'true'"],
    'step_string_true'=>['step', "if: 'true'"],
    'job_null_if'=>['job', 'if: null'],
    'step_null_if'=>['step', 'if: null'],
    'job_continue'=>['job', 'continue-on-error: true'],
    'step_continue'=>['step', 'continue-on-error: true'],
    'job_continue_expression'=>['job', "continue-on-error: '${{ false }}'"],
    'step_continue_string'=>['step', "continue-on-error: 'false'"],
    'step_echo_shell'=>['step', 'shell: echo {0}'],
    'step_shell_expression'=>['step', "shell: '${{ matrix.shell }}'"],
    'step_shell_null'=>['step', 'shell: null'],
    'step_working_directory'=>['step', 'working-directory: /tmp'],
    'step_working_directory_dot'=>['step', 'working-directory: .'],
    'workflow_default_shell'=>['workflow', "defaults:\n  run:\n    shell: echo {0}"],
    'job_default_shell'=>['job', "defaults:\n  run:\n    shell: echo {0}"],
    'workflow_default_directory'=>['workflow', "defaults:\n  run:\n    working-directory: /tmp"],
    'job_default_directory'=>['job', "defaults:\n  run:\n    working-directory: /tmp"],
    'workflow_defaults_null'=>['workflow', 'defaults: null'],
    'job_defaults_empty'=>['job', 'defaults: {}'],
    'workflow_unsupported_defaults'=>['workflow', "defaults:\n  unknown: true"],
    'job_unsupported_defaults'=>['job', "defaults:\n  run:\n    unknown: true"]
  }
  ci_execution_bypasses.each do |name, pair|
    define_method("test_wave2_ci_execution_rejects_#{name}") do
      scope, fragment = pair
      with_fixture do |root|
        workflow = "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n"
        case scope
        when 'workflow'
          workflow = fragment + "\n" + workflow
        when 'job'
          workflow = workflow.sub("    steps:\n", fragment.lines.map { |line| '    ' + line }.join.rstrip + "\n    steps:\n")
        when 'step'
          workflow += fragment.lines.map { |line| '        ' + line }.join.rstrip + "\n"
        end
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        File.write(File.join(root, '.github/workflows/ci.yml'), workflow)
        assert_cli_error(root, 'ci_rfc_gate_missing', '--check')
      end
    end
  end

  {
    'missing_runner'=>proc { |text| text.sub("    runs-on: ubuntu-latest\n", '') },
    'expression_runner'=>proc { |text| text.sub('runs-on: ubuntu-latest', "runs-on: '${{ matrix.os }}'") },
    'unsupported_runner'=>proc { |text| text.sub('runs-on: ubuntu-latest', 'runs-on: self-hosted') },
    'job_shell'=>proc { |text| text.sub("    steps:\n", "    shell: bash\n    steps:\n") },
    'job_uses'=>proc { |text| text.sub("    steps:\n", "    uses: other/workflow.yml\n    steps:\n") },
    'job_env'=>proc { |text| text.sub("    steps:\n", "    env:\n      PATH: /tmp\n    steps:\n") },
    'workflow_env'=>proc { |text| "env:\n  PATH: /tmp\n" + text },
    'step_env'=>proc { |text| text + "        env:\n          PATH: /tmp\n" },
    'step_timeout'=>proc { |text| text + "        timeout-minutes: 0\n" },
    'job_timeout'=>proc { |text| text.sub("    steps:\n", "    timeout-minutes: 0\n    steps:\n") },
    'job_container'=>proc { |text| text.sub("    steps:\n", "    container: busybox\n    steps:\n") }
  }.each do |name, mutation|
    define_method("test_wave2_narrow_execution_rejects_#{name}") do
      with_fixture do |root|
        workflow = "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n"
        changed = mutation.call(workflow)
        refute_equal workflow, changed
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        File.write(File.join(root, '.github/workflows/ci.yml'), changed)
        assert_cli_error(root, 'ci_rfc_gate_missing', '--check')
      end
    end
  end

  def test_wave2_ci_execution_accepts_minimal_bash_sh_and_explicit_safe_booleans
    [nil, 'bash', 'sh'].each do |shell|
      with_fixture do |root|
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        step_shell = shell ? "        shell: #{shell}\n" : ''
        File.write(File.join(root, '.github/workflows/ci.yml'),
          "on: push\njobs:\n  docs:\n    if: true\n    continue-on-error: false\n    runs-on: ubuntu-latest\n    steps:\n      - if: true\n        continue-on-error: false\n        run: ruby scripts/architecture-docs/check.rb --check\n" + step_shell)
        out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
        assert_equal 1, status.exitstatus, out + err
        assert_equal %w[rfc_status_provisional wbs_status_provisional rfc_html_missing], out.lines.map(&:strip)
        assert_empty err
      end
    end
  end

  workflow_envelope_mutations = {
    'missing_on'=>proc { |text| text.sub("on: push\n", '') },
    'null_trigger'=>proc { |text| text.sub('on: push', 'on: null') },
    'empty_trigger'=>proc { |text| text.sub('on: push', "on: ''") },
    'empty_sequence'=>proc { |text| text.sub('on: push', 'on: []') },
    'expression_trigger'=>proc { |text| text.sub('on: push', "on: '${{ github.event_name }}'") },
    'unknown_trigger'=>proc { |text| text.sub('on: push', 'on: made_up_event') },
    'unknown_sequence_event'=>proc { |text| text.sub('on: push', 'on: [push, made_up_event]') },
    'null_sequence_event'=>proc { |text| text.sub('on: push', 'on: [push, null]') },
    'nested_sequence'=>proc { |text| text.sub('on: push', 'on: [push, [pull_request]]') },
    'duplicate_sequence_event'=>proc { |text| text.sub('on: push', 'on: [push, push]') },
    'empty_mapping_trigger'=>proc { |text| text.sub('on: push', 'on: {}') },
    'unknown_top_key'=>proc { |text| "unknown: true\n" + text },
    'top_timeout'=>proc { |text| "timeout-minutes: 0\n" + text },
    'top_env'=>proc { |text| "env: {}\n" + text },
    'top_defaults'=>proc { |text| "defaults: {}\n" + text },
    'duplicate_on'=>proc { |text| "on: pull_request\n" + text },
    'duplicate_jobs'=>proc { |text| text + text.sub("on: push\n", '') },
    'duplicate_step_key'=>proc { |text| text.sub('- run:', "- run: echo skipped\n        run:") },
    'yaml11_if_yes'=>proc { |text| text + "        if: yes\n" },
    'yaml11_continue_no'=>proc { |text| text + "        continue-on-error: no\n" },
    'literal_true_key'=>proc { |text| text.sub('on: push', 'true: push') },
    'quoted_true_key'=>proc { |text| text.sub('on: push', "'true': push") },
    'missing_jobs'=>proc { |text| text.sub(/^jobs:\n.*\z/m, '') },
    'empty_jobs'=>proc { |text| text.sub(/^jobs:\n.*\z/m, "jobs: {}\n") },
    'invalid_job_id'=>proc { |text| text.sub('  docs:', '  123:') },
    'invalid_name'=>proc { |text| "name: {}\n" + text },
    'duplicate_name'=>proc { |text| "name: first\nname: second\n" + text },
    'multiple_documents'=>proc { |text| text + "---\non: push\njobs: {}\n" }
  }
  workflow_envelope_mutations.each do |name, mutation|
    define_method("test_wave3_workflow_envelope_rejects_#{name}") do
      with_fixture do |root|
        workflow = "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n"
        changed = mutation.call(workflow)
        refute_equal workflow, changed
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        File.write(File.join(root, '.github/workflows/ci.yml'), changed)
        assert_cli_error(root, 'ci_rfc_gate_missing', '--check')
      end
    end
  end

  def test_wave3_real_workflow_scalar_sequence_and_literal_on_keys
    ["on: push", "on: pull_request", "on: workflow_dispatch",
     "on: [push, pull_request, workflow_dispatch]", "on:\n  - push\n  - pull_request",
     "'on': push", '"on": workflow_dispatch'].each do |trigger|
      with_fixture do |root|
        FileUtils.mkdir_p(File.join(root, '.github/workflows'))
        File.write(File.join(root, '.github/workflows/ci.yml'),
          "name: RFC contract\n#{trigger}\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n")
        out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--check')
        assert_equal 1, status.exitstatus, out + err
        assert_equal %w[rfc_status_provisional wbs_status_provisional rfc_html_missing], out.lines.map(&:strip)
        assert_empty err
      end
    end
  end

  def test_strict_release_artifacts_reject_directories_and_symlink_paths
    {'docs/push-system/push-system-implementation-rfc.html' => 'rfc_html_missing',
     '.github/workflows/ci.yml' => 'ci_rfc_gate_missing'}.each do |relative, reason|
      with_fixture do |root|
        path = File.join(root, relative)
        FileUtils.mkdir_p(File.dirname(path))
        saved = path + '.saved'
        File.write(saved, "on: push\njobs:\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruby scripts/architecture-docs/check.rb --check\n")
        File.symlink(saved, path)
        assert_cli_error(root, reason, '--check')
        File.unlink(path)
        File.symlink(path + '.missing', path)
        assert_cli_error(root, reason, '--check')
        File.unlink(path)
        Dir.mkdir(path)
        assert_cli_error(root, reason, '--check')
        Dir.rmdir(path)
        File.rename(saved, path)
        parent = File.dirname(path)
        File.rename(parent, parent + '.saved')
        File.symlink(parent + '.saved', parent)
        assert_cli_error(root, reason, '--check')
      end
    end
  end

  def test_public_cli_accepts_the_frozen_domain_contract_in_draft
    out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', ROOT, '--draft')
    assert_equal 0, result.exitstatus, out + err
    assert_equal "rfc_spec_valid\n", out
    assert_empty err
  end

  def test_wbs_is_a_generated_provisional_bridge_not_a_second_estimate_authority
    with_fixture do |root|
      assert_cli_error(root, 'wbs_status_provisional', '--check')
      change_text(root) { |text| text.sub('828.99h', '828.98h') }
      assert_cli_error(root, 'wbs_rfc_stale')
    end
    with_fixture do |root|
      change_text(root) { |text| text.sub('<!-- RFC-WBS-BEGIN -->', '') }
      assert_cli_error(root, 'wbs_markers_invalid')
    end
  end

  def test_shadow_owner_rule_applies_to_shadow_actor_not_entire_unit
    with_fixture do |root|
      path = File.join(root, RFC)
      text = File.read(path)
      pattern = /^\| Shadow \| (?:PhysicalOwner|ShadowActorPhysicalOwner) \| None \| \[Q:13\] \|$/
      assert_equal 1, text.scan(pattern).length
      scoped = '| Shadow | ShadowActorPhysicalOwner | None | [Q:13] |'
      File.write(path, text.sub(pattern, scoped))
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, result.exitstatus, out + err
      assert_equal "rfc_spec_valid\n", out
      assert_empty err

      change_text(root) { |document| document.sub(scoped, '| Shadow | PhysicalOwner | None | [Q:13] |') }
      assert_cli_error(root, 'rfc_activation_operations_invalid')
    end
  end

  # 只突变唯一规范表；每个反例先证明原表和目标存在，排除重复章节造成的假绿。
  rollout_mutations = [
    ['调度身份', 'rfc_schedule_identity_invalid', 'ScheduleOccurrence', 'source_contract_id |', 'source_contract_id,activation_generation |'],
    ['运行就绪判定', 'rfc_readiness_invalid', 'CoreUnready', 'OperationalReadinessSnapshot', 'LogsOnly'],
    ['运行就绪判定', 'rfc_readiness_invalid', 'ProducerUnready', '| false |', '| true |'],
    ['运行就绪判定', 'rfc_readiness_invalid', 'BlockedOnInput', '| true | true |', '| true | false |'],
    ['就绪查询与恢复合同', 'rfc_readiness_query_invalid', 'missing_active_contract', 'EscalateProducerUnready', 'RemainBlockedOnInput'],
    ['就绪查询与恢复合同', 'rfc_readiness_query_invalid', 'log_pager', 'ProjectionOnlyNeverReadinessAuthority', 'LogsAreAuthority'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'SingleControl', 'AuthenticatedOnlineUserOrProductionAllowlistedOperator', 'AnonymousOrFreeText'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'SingleControl', 'V1BaselineOneMayApproveAndExecute', 'MandatoryTwoOperators'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'DualControl', 'DistinctAuthenticatedPreparerAndApproverCannotDowngrade', 'SameIdentityAllowed'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'dry_run_and_refusal', 'NoDBNoJournalNoOwnerChangeNoProviderNoLLMNoSinkNoOrder', 'WriteJournal'],
    ['证据保留类别', 'rfc_retention_invalid', 'NonTerminal', 'NeverAutoDelete', 'DeleteAfter90Days'],
    ['证据保留类别', 'rfc_retention_invalid', 'DeliveryAuditRegulatory', 'StrictlyGreaterThanFiveYears', '1825Days'],
    ['证据保留类别', 'rfc_retention_invalid', 'ModelDecisionTrade', 'NoUnifiedFiveYearMaximum', 'FiveYearMaximum'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'common_fence', 'LegacyAndNewSchedulerProducerDispatcherFinalizer', 'NewSchedulerOnly'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'emergency_rollback', 'NewGenerationCASAppendJournalBlockLaterPromotionToday', 'ReuseOldGeneration'],
    ['影子精确比较', 'rfc_shadow_compare_invalid', 'exclusions', 'attempt_id,latency,diagnostic_timestamp', 'attempt_id,latency,diagnostic_timestamp,business_date'],
    ['调度恢复策略', 'rfc_schedule_recovery_invalid', 'non_trading_reason', 'EvaluationOnlyNoOccurrenceNoNoDataOrDisabledIntent', 'CreateNoDataIntent'],
    ['就绪查询与恢复合同', 'rfc_readiness_query_invalid', 'activation.producer_unready', 'ProducerUnready |', 'BlockedOnInput |'],
    ['就绪查询与恢复合同', 'rfc_readiness_query_invalid', 'input.source_unready', 'RegisteredContractOccurrenceEvidenceUnavailable', 'MissingProducerContract'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'quota_transaction', 'BEGIN IMMEDIATE', 'MemoryMutex'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'quota_calendar', 'CalendarBoundUTCStartInclusiveEndExclusive', 'LocalWallClockDate'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'quota_query', 'AllUnitsPromotionJournalOccurredAt', 'CurrentUnitOnly'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'quota_query', 'RejectAnyActivateOrRollbackInBusinessDateInterval', 'IgnoreRollback'],
    ['物理所有权与晋级合同', 'rfc_activation_operations_invalid', 'quota_apply', 'SameImmediateTransaction', 'SeparateTransactions'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'refusal_audit', 'IndependentControlPlaneAuditSinkOnly', 'BusinessSink'],
    ['操作员权限', 'rfc_operator_authorization_invalid', 'dry_run_refusal_storage', 'NoWrites', 'WritesAllowed'],
    ['风险波次顺序', 'rfc_risk_waves_invalid', '1', 'CLI report typed BestEffort result', 'PaperBuy'],
    ['故障环境与验收边界', 'rfc_fault_environment_invalid', 'Production', 'ApprovedNormalTypedReceiptAndSameDecisionIdempotentReplay', 'KillDatabase'],
    ['类型：ScheduleOccurrence', 'rfc_type_fields_invalid', 'calendar_id', nil, nil],
    ['类型：OperationalReadinessSnapshot', 'rfc_type_fields_invalid', 'recovery_event_id', nil, nil],
    ['业务验收样本', 'rfc_sample_bindings_invalid', 'historical_backfill_2026-08-31', '[unit:MU-review-r11]', '[unit:MU-review-r04]'],
    ['业务验收样本', 'rfc_sample_bindings_invalid', 'news_ai_cross_batch', '[producer:news-ai-same-tick]', '[producer:d01-manual]'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'historical_backfill_2026-08-31', 'TomorrowWatch+PositionReview', 'ReviewBackfill'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'news_ai_cross_batch', 'NewsToIdea;producer=news-ai-same-tick', 'NewsAI'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'r03_blocked_input', 'IndustryChain;ReviewTask=R03', 'R03'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'r08_retryability', 'EventCalendar;ReviewTask=R08', 'R08'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'n02_receipt_time', '§F01/§F10', '#f01-f10'],
    ['业务验收样本', 'rfc_acceptance_samples_invalid', 'n02_receipt_time', 'comprehensive-reanalysis-2026-09-05.md', 'missing.md'],
    ['操作员请求与输出', 'rfc_operator_wire_invalid', 'Response | mutation_journal_event_ref', 'AppliedMutationOnlyNullForInspectDryRunRefusal', 'AuditEnvelopeInMutationJournal'],
    ['操作员请求与输出', 'rfc_operator_wire_invalid', 'Response | operator_audit_event_ref', nil, nil],
    ['调度恢复策略', 'rfc_schedule_recovery_invalid', 'schema_version', 'ScheduleOccurrence/v1', 'ScheduleOccurrence/v2']
  ]
  {
    '调度生命周期' => ['rfc_schedule_lifecycle_invalid', ['Expected | Eligible', 'Eligible | Prepared', 'Prepared | Closed', 'Expected | Missed', 'Eligible | Missed', 'Expected | Deferred', 'Eligible | Deferred', 'Expected | BlockedOnInput', 'Eligible | BlockedOnInput', 'BlockedOnInput | Eligible', 'BlockedOnInput | Missed', 'BlockedOnInput | Deferred', 'Deferred | Eligible']],
    '调度恢复策略' => ['rfc_schedule_recovery_invalid', %w[ExpireWithoutCatchUp SameBusinessDayBeforeDeadline DeferToNextEligibleSession RecoverPersistedOnly coalesce non_trading_day independent_trigger INACTIVE STARVED OPT-IN]],
    '操作员命令' => ['rfc_operator_commands_invalid', %w[inspect reconcile resolve-uncertain promote rollback]],
    '通用晋级门禁' => ['rfc_rollout_gates_invalid', %w[unit failure crash shadow dedup rollback]],
    '业务验收样本' => ['rfc_acceptance_samples_invalid', %w[historical_backfill_2026-08-31 n02_receipt_time g5b_test_namespace news_ai_cross_batch paper_sell_254_2026-09-01 attribution_g5b_sink_fail r03_blocked_input r08_retryability no_data_disabled_uncertain cross_db_conflict_rollback]],
    '清理资格与安全' => ['rfc_cleanup_invalid', %w[terminal_binding transition_journal_audit retention_expiry legal_hold disclosure backup_integrity nonterminal_uncertain_resolution worm_mutation secrets_and_unnecessary_content]]
  }.each do |section, (error, keys)|
    keys.each { |key| rollout_mutations << [section, error, key, nil, nil] }
  end
  %w[provider_second_call llm_recompute business_db_write durable_db_write cursor_advance candidate_watchlist_outcome paper_order_fill transport_send].each do |key|
    rollout_mutations << ['影子副作用', 'rfc_shadow_effects_invalid', key, 'Forbidden', 'Allowed']
  end
  %w[inspect reconcile resolve-uncertain promote rollback].each do |key|
    rollout_mutations << ['操作员命令', 'rfc_operator_commands_invalid', key, "| #{key} |", "| renamed-#{key} |"]
  end
  %w[paper_buy_29_2026-09-04 watchdog_nonbaseline].each do |key|
    rollout_mutations << ['非基线回放样本', 'rfc_nonbaseline_samples_invalid', key, 'NON_BASELINE_REPLAY_ONLY', 'CURRENT_65_KIND']
    rollout_mutations << ['非基线回放样本', 'rfc_nonbaseline_samples_invalid', key, 'NoCatalogUnitNoBaselineCapabilityNoProducerActivationNoWaveChange', 'CreateUnitActivateProducer']
    rollout_mutations << ['非基线回放样本', 'rfc_nonbaseline_samples_invalid', key, nil, nil]
  end
  rollout_mutations.each_with_index do |(section, error, key, from, to), index|
    define_method("test_rollout_mutation_#{index}_#{key.gsub(/\W+/, '_')}") do
      with_fixture do |root|
        change_text(root) do |s|
          pattern = /^## #{Regexp.escape(section)}（PROPOSED）\n.*?(?=^## |\z)/m
          assert_equal 1, s.scan(pattern).length
          s.sub(pattern) do |body|
            row_pattern = /^\| #{Regexp.escape(key)} \|.*\n/
            assert_equal 1, body.scan(row_pattern).length
            body.sub(row_pattern) { |row| from ? row.sub(from, to) : '' }
          end
        end
        require_relative '../rfc_spec'
        errors = ArchitectureDocs::RfcSpec.validate(root)
        refute_includes errors.join("\n"), 'rfc_section_duplicate'
        assert_cli_error(root, error)
      end
    end
  end

  occurrence_cas_mutations = [
    ['类型：ScheduleOccurrence', 'version', nil, nil, 'rfc_type_fields_invalid type=ScheduleOccurrence'],
    ['类型：ScheduleOccurrence', 'version', 'u64', 'bool', 'rfc_field_type_invalid type=ScheduleOccurrence field=version'],
    ['类型：ScheduleOccurrence', 'version', 'ScheduleVersionRule::v1', 'UsePushIntentVersion', 'rfc_schedule_version_invalid'],
    ['调度身份', 'ScheduleOccurrence', 'source_contract_id |', 'source_contract_id,version |', 'rfc_schedule_identity_invalid'],
    ['调度版本与转换提交', 'storage_owner', 'BusinessDBSameTransactionIndependentOfPushIntentVersion', 'PushIntentVersionOnly', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'initial_version', 'Zero', 'One', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'create_conflict', 'ReadExistingNeverOverwriteOrReset', 'ResetVersionToZero', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'request_guard', 'ExactIdFromStatusExpectedVersion', 'IdOnly', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'fence_guard', 'CurrentUnitGenerationManifestOwnerAndExpectedGeneration', 'CachedFenceAllowed', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'success_version', 'CheckedExpectedVersionPlusOne', 'KeepVersion', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'atomic_commit', 'StateVersionReasonAndTransitionEvidenceOneBusinessTransaction', 'SeparateEvidenceCommit', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'zero_rows', 'NoStateOrVersionWriteNoEventNoPrepareProviderLLMSinkCursorOrder', 'AppendEventAnyway', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'conflict_recovery', 'RereadOccurrenceAndCurrentFenceReevaluateNeverBlindRetry', 'RetrySameRequest', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'identity_version', 'ExcludedFromScheduleOccurrenceId', 'IncludedInIdentity', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'overflow', 'RefuseNoWritesNoEvents', 'WrapToZero', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'commit_ack_unknown', 'RequeryOccurrenceAndVersionEventBeforeAnyNewRequest', 'RetryImmediately', 'rfc_schedule_version_invalid'],
    ['类型：ScheduleOccurrenceTransitionRequest', 'expected_version', 'ScheduleVersionRule::v1', 'UsePushIntentVersion', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'initial_state', 'Expected', 'Eligible', 'rfc_schedule_version_invalid'],
    ['调度版本与转换提交', 'lifecycle_guard', 'RegisteredEdgeReasonAuthorityWindowAndEvidence', 'SkipWindowAndEvidence', 'rfc_schedule_version_invalid'],
    ['调度身份', 'ScheduleOccurrence', 'activation_generation,version,expected_version,build', 'activation_generation,build', 'rfc_schedule_identity_invalid']
  ]
  %w[schedule_occurrence_id from_status to_status expected_version expected_generation fence_token reason evidence_refs].each do |field|
    occurrence_cas_mutations << ['类型：ScheduleOccurrenceTransitionRequest', field, nil, nil, 'rfc_type_fields_invalid type=ScheduleOccurrenceTransitionRequest']
  end
  occurrence_cas_mutations.each_with_index do |(section, key, from, to, error), index|
    define_method("test_occurrence_cas_mutation_#{index}_#{key}") do
      with_fixture do |root|
        change_text(root) do |s|
          pattern = /^## #{Regexp.escape(section)}（PROPOSED）\n.*?(?=^## |\z)/m
          assert_equal 1, s.scan(pattern).length
          s.sub(pattern) do |body|
            row_pattern = /^\| #{Regexp.escape(key)} \|.*\n/
            assert_equal 1, body.scan(row_pattern).length
            body.sub(row_pattern) { |row| from ? row.sub(from, to) : '' }
          end
        end
        require_relative '../rfc_spec'
        errors = ArchitectureDocs::RfcSpec.validate(root)
        refute_includes errors.join("\n"), 'rfc_section_duplicate'
        assert_cli_error(root, error)
      end
    end
  end

  def test_occurrence_cas_contract_cannot_be_removed
    with_fixture do |root|
      change_text(root) do |s|
        s.sub(/^## 调度版本与转换提交（PROPOSED）\n.*?(?=^## |\z)/m, '')
      end
      assert_cli_error(root, 'rfc_schedule_version_invalid')
    end
  end

  def test_occurrence_cas_contract_allows_prose_and_row_reordering
    require_relative '../rfc_spec'
    with_fixture do |root|
      change_text(root) do |s|
        s.sub(/^## 调度版本与转换提交（PROPOSED）\n.*?(?=^## |\z)/m) do |body|
          lines = body.lines
          positions = lines.each_index.select { |i| lines[i].match?(/^\| [a-z_]+ \|/) }
          assert_equal 14, positions.length
          reversed = positions.map { |i| lines[i] }.reverse
          positions.each_with_index { |position, i| lines[position] = reversed[i] }
          lines.join + "\n补充说明：展示次序与中文解释不改变原子提交规则。\n\n"
        end
      end
      assert_equal [], ArchitectureDocs::RfcSpec.validate(root)
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, result.exitstatus, out + err
    end
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

  def test_rollout_contract_allows_explanation_and_table_row_reordering
    require_relative '../rfc_spec'
    with_fixture do |root|
      change_text(root) do |s|
        s.sub(/^## 影子副作用（PROPOSED）\n.*?(?=^## |\z)/m) do |body|
          lines = body.lines
          indexes = lines.each_index.select { |i| lines[i].match?(/^\| [a-z_]+ \|/) }
          assert_equal 8, indexes.length
          reversed = indexes.map { |i| lines[i] }.reverse
          indexes.each_with_index { |position, i| lines[position] = reversed[i] }
          lines.join + "\n补充说明：规范行可换展示次序，许可仍由闭集校验。\n\n"
        end
      end
      assert_equal [], ArchitectureDocs::RfcSpec.validate(root)
      out, err, result = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, result.exitstatus, out + err
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

  def test_wave1_manual_not_delivered_is_a_real_non_sending_terminal
    with_database do |db, ddl|
      seed_authority(db)
      sql_ok(db, not_delivered_transaction)
      assert_equal "NotDelivered|2\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      assert_equal "ManualConfirmedNotDelivered|decision-1|operator-audit-1\n",
        sql_ok(db, "SELECT terminal_disposition,terminal_decision_id,operator_audit_ref FROM push_intent_transitions WHERE to_state='NotDelivered';")
      sql_ok(db, ddl)
      assert_equal "NotDelivered|2\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
    end
  end

  def test_wave1_activation_success_requires_exact_reason_for_all_six_actions
    with_database do |db, _ddl|
      [['Disabled','Initialize'],['Shadow','EnterShadow'],['Active','Activate'],
       ['Draining','Drain'],['Disabled','Disable'],['Disabled','Rollback']].each_with_index do |pair, index|
        generation = index + 1
        extra = generation == 6 ? {'rollback_target_sha256'=>"'#{manifest_hash(1)}'"} : {}
        sql_ok(db, insert_manifest(generation, pair[0], extra))
        %w[transport.uncertain activation.bogus].each do |reason|
          sql_rejected(db, insert_journal(generation, pair[1], extra.merge('reason'=>"'#{reason}'")))
        end
        sql_ok(db, insert_journal(generation, pair[1], extra))
      end
      assert_equal "6|6\n", sql_ok(db, "SELECT count(*),sum(reason='activation.applied') FROM push_promotion_journal;")
    end
  end

  %w[运行里程碑 外部兼容 裁决追踪].each do |contract|
    define_method("test_wave1_public_cli_requires_#{contract}") do
      with_fixture do |root|
        path = File.join(root, RFC)
        text = File.read(path)
        name = "#{contract}（PROPOSED）"
        text = text.sub(/^## #{Regexp.escape(name)}\n.*?(?=^## |\z)/m, '')
        File.write(path, text)
        assert_cli_error(root, "rfc_section_missing name=#{name}")
      end
    end
  end

  def test_wave1_not_delivered_requires_exact_terminal_group_reason_and_decision
    [
      {'terminal_ref_id'=>'NULL'}, {'terminal_binding_sha256'=>'NULL'},
      {'terminal_disposition'=>"'Accepted'"}, {'terminal_decision_id'=>"'other-decision'"},
      {'operator_audit_ref'=>'NULL'}, {'operator_audit_sha256'=>'NULL'},
      {'operator_audit_sha256'=>"'bad'"}, {'reason'=>"'operator.resolution_conflict'"}
    ].each do |fields|
      with_database do |db, _ddl|
        seed_authority(db)
        sql_rejected(db, not_delivered_transaction('AwaitingAuthority', 1, fields))
        assert_equal "AwaitingAuthority|1\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
        assert_equal "1\n", sql_ok(db, 'SELECT count(*) FROM push_intent_transitions;')
      end
    end
    with_database do |db, _ddl|
      seed_authority(db)
      sql_rejected(db, not_delivered_transaction('AwaitingAuthority', 1, {}, 'operator.resolution_conflict'))
      sql_rejected(db, not_delivered_transaction('AwaitingAuthority', 0))
      assert_equal "AwaitingAuthority|1\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
    end
  end

  def test_wave1_not_delivered_allows_only_same_decision_uncertain_isolation_and_is_immutable
    with_database do |db, _ddl|
      seed_authority(db)
      sql_ok(db, business_edge_transaction('AwaitingAuthority', 'ResolutionRequired', 1, 'transport.uncertain'))
      sql_ok(db, business_edge_transaction('ResolutionRequired', 'ResolutionRequired', 2, 'intent.lease_held'))
      sql_ok(db, not_delivered_transaction('ResolutionRequired', 3))
      assert_equal "NotDelivered|4\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      sql_rejected(db, "UPDATE push_intent_transitions SET terminal_disposition='Accepted' WHERE to_state='NotDelivered';")
      sql_rejected(db, "DELETE FROM push_intent_transitions WHERE to_state='NotDelivered';")
      sql_rejected(db, business_edge_transaction('NotDelivered', 'AwaitingAuthority', 4, 'intent.dispatch_claimed'))
      sql_rejected(db, business_edge_transaction('NotDelivered', 'ResolutionRequired', 4, 'intent.payload_conflict'))
    end
    %w[intent.payload_conflict operator.resolution_conflict].each do |cause|
      with_database do |db, _ddl|
        seed_authority(db)
        sql_ok(db, business_edge_transaction('AwaitingAuthority', 'ResolutionRequired', 1, cause))
        sql_rejected(db, not_delivered_transaction('ResolutionRequired', 2))
        assert_equal "ResolutionRequired|2\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      end
    end
  end

  def test_wave1_not_delivered_cannot_revoke_accepted_or_close_non_send_origins
    %w[AwaitingFinalizer Completed].each do |accepted_state|
      with_database do |db, _ddl|
        seed_authority(db)
        sql_ok(db, business_edge_transaction('AwaitingAuthority', 'AwaitingFinalizer', 1, 'intent.authority_verified'))
        version = 2
        if accepted_state == 'Completed'
          sql_ok(db, business_edge_transaction('AwaitingFinalizer', 'Completed', 2, 'finalizer.completed',
            'terminal_ref_id'=>"'accepted-ref'",'terminal_binding_sha256'=>"'#{'a' * 64}'"))
          version = 3
        end
        sql_rejected(db, not_delivered_transaction(accepted_state, version))
        sql_ok(db, business_edge_transaction(accepted_state, 'ResolutionRequired', version,
          accepted_state == 'Completed' ? 'intent.payload_conflict' : 'transport.uncertain'))
        sql_rejected(db, not_delivered_transaction('ResolutionRequired', version + 1))
        assert_equal "ResolutionRequired|#{version+1}\n", sql_ok(db, 'SELECT state,version FROM push_intents;')
      end
    end
    %w[NoData Disabled].each do |kind|
      with_database do |db, _ddl|
        sql_ok(db, insert_intent('job_decision_kind'=>"'#{kind}'",'state'=>"'#{kind}'",
          'reason'=>kind == 'NoData' ? "'intent.no_data'" : "'policy.disabled'",
          'prepared_push_bytes'=>'NULL','rendered_bytes'=>'NULL','payload_sha256'=>'NULL','rendered_sha256'=>'NULL'))
        sql_rejected(db, not_delivered_transaction(kind, 0))
        sql_ok(db, business_edge_transaction(kind, 'ResolutionRequired', 0, 'intent.payload_conflict'))
        sql_rejected(db, not_delivered_transaction('ResolutionRequired', 1))
      end
    end
    with_database do |db, _ddl|
      sql_ok(db, insert_intent)
      sql_rejected(db, not_delivered_transaction('PendingDispatch', 0))
    end
  end

  def test_wave1_raw_cli_rejects_previous_v1_signature_without_business_changes
    ddl = File.binread(File.join(ROOT, SQL))
    current = ddl.match(/schema_signature='([a-f0-9]{64})'/)[1]
    previous = 'ae30ae6f6a0d8fa805fc7594137c6d27e6af3a879dc3625fa76d8146eb5a94e5'
    refute_equal previous, current
    Dir.mktmpdir('rfc-previous-v1') do |dir|
      db = File.join(dir, 'previous.sqlite3')
      sql_ok(db, ddl.gsub(current, previous))
      sql_ok(db, insert_intent)
      before = sql_ok(db, 'SELECT name,sql FROM sqlite_master ORDER BY name; SELECT * FROM push_intents;')
      out, err, status = Open3.capture3('/usr/bin/sqlite3', db, stdin_data: ddl)
      refute_equal 0, status.exitstatus, out
      assert_includes err, 'activation.manifest_mismatch'
      assert_equal before, sql_ok(db, 'SELECT name,sql FROM sqlite_master ORDER BY name; SELECT * FROM push_intents;')
    end
  end

  wave1_mutations = [
    ['不投递终态合同','entry','任意状态均可不投递','rfc_not_delivered_contract_invalid'],
    ['不投递终态合同','verification','无需认证或重验','rfc_not_delivered_contract_invalid'],
    ['不投递终态合同','commit','先改状态后异步事件','rfc_not_delivered_contract_invalid'],
    ['不投递终态合同','cursor','推进游标并计入成功','rfc_not_delivered_contract_invalid'],
    ['不投递终态合同','gate','自动批准晋级','rfc_not_delivered_contract_invalid'],
    ['运行里程碑','FoundationReady','OnlyCodeNoGreybox','rfc_runtime_milestones_invalid'],
    ['运行里程碑','P0ProductionVerified','AllNullWavesAutoApproved','rfc_runtime_milestones_invalid'],
    ['运行里程碑','ArchitectureReleaseCandidate','OnlyP0Needed','rfc_runtime_milestones_invalid'],
    ['运行里程碑','ProgramProductionVerified','SomeUnitsAndStaleEvidenceAllowed','rfc_runtime_milestones_invalid'],
    ['运行退出验收','unit_inventory','65 kinds 即 65 owners','rfc_runtime_exit_invalid'],
    ['运行退出验收','nullable_waves','null 自动成为下一波','rfc_runtime_exit_invalid'],
    ['运行退出验收','foundation_greybox','不同 decision 的日志即可','rfc_runtime_exit_invalid'],
    ['运行退出验收','unit_greybox','conformance 强制新 owner','rfc_runtime_exit_invalid'],
    ['运行退出验收','parallel_tests','忽略默认并行失败','rfc_runtime_exit_invalid'],
    ['运行退出验收','backup_restore','两个库跨库原子快照无需 Test 恢复','rfc_runtime_exit_invalid'],
    ['运行退出验收','old_path_delete','首个 Accepted 即删除','rfc_runtime_exit_invalid'],
    ['运行退出验收','release_pipeline','接管同版本删除旧路径且无尾部清理','rfc_runtime_exit_invalid'],
    ['运行退出验收','program_exit','允许 unresolved Uncertain 和 receipt 缺口','rfc_runtime_exit_invalid'],
    ['运行退出验收','failure_retained','NotDelivered 抵消失败','rfc_runtime_exit_invalid'],
    ['运行退出验收','publication_boundary','HTML 通过即生产完成','rfc_runtime_exit_invalid'],
    ['外部兼容','cli','MayChangeExitOrArguments','rfc_external_compatibility_invalid'],
    ['外部兼容','config','MayChangeDefaultOrScope','rfc_external_compatibility_invalid'],
    ['外部兼容','subscription','MayDropRequiredChannels','rfc_external_compatibility_invalid'],
    ['外部兼容','template','MayRerenderAndChangeWording','rfc_external_compatibility_invalid'],
    ['外部兼容','authority','COMPATMayAdvanceCursor','rfc_external_compatibility_invalid']
  ]
  wave1_mutations.each_with_index do |(section, key, reversed, error), index|
    define_method("test_wave1_semantic_mutation_#{index}_#{key}") do
      with_fixture do |root|
        change_text(root) do |text|
          pattern = /^## #{Regexp.escape(section)}（PROPOSED）\n.*?(?=^## |\z)/m
          text.sub(pattern) do |body|
            body.sub(/^\| #{Regexp.escape(key)} \|.*$/) do |line|
              cells = line.split('|', -1)
              column = section == '运行里程碑' ? 4 : 3
              cells[column] = " #{reversed} "
              cells.join('|')
            end
          end
        end
        assert_cli_error(root, error)
      end
    end
  end

  def test_wave1_trace_rejects_missing_duplicate_wrong_choice_locus_and_refs
    mutations = {
      'missing'=>[proc { |row| '' }, 'rfc_trace_coverage_invalid'],
      'duplicate'=>[proc { |row| row + row }, 'rfc_trace_coverage_invalid'],
      'choice'=>[proc { |row| row.sub('| 4 | A |', '| 4 | B |') }, 'rfc_trace_choice_invalid'],
      'locus'=>[proc { |row| row.sub('外部兼容（PROPOSED）', '不存在的规范章节') }, 'rfc_trace_locus_invalid'],
      'empty_locus'=>[proc { |row| row.sub('外部兼容（PROPOSED）', '') }, 'rfc_table_invalid'],
      'ref'=>[proc { |row| row.sub('[acceptance:cli]', '[acceptance:invented]') }, 'rfc_trace_refs_invalid'],
      'q_only'=>[proc { |row| row.sub('[acceptance:cli]', '[Q:4]') }, 'rfc_trace_refs_invalid'],
      'prose_ref'=>[proc { |row| row.sub('[acceptance:cli]', '[acceptance:cli] 已通过生产') }, 'rfc_trace_refs_invalid']
    }
    mutations.each do |name, pair|
      with_fixture do |root|
        change_text(root) { |text| text.sub(/^\| 4 \| A \|.*\n/, &pair[0]) }
        assert_cli_error(root, pair[1])
      end
    end
  end

  def test_wave1_public_cli_accepts_prose_and_normative_row_reordering
    with_fixture do |root|
      change_text(root) do |text|
        text = text.sub("## 裁决追踪（PROPOSED）\n", "## 裁决追踪（PROPOSED）\n\n本段解释可调整，不改变规范选择。\n")
        %w[运行里程碑 运行退出验收 外部兼容 裁决追踪].each do |section|
          text = text.sub(/^## #{Regexp.escape(section)}（PROPOSED）\n.*?(?=^## |\z)/m) do |body|
            rows = body.lines.grep(/^\| /).drop(2)
            remaining = rows.reverse
            count = 0
            body.lines.map do |line|
              if line.start_with?('| ')
                count += 1
                count > 2 ? remaining.shift : line
              else
                line
              end
            end.join
          end
        end
        text
      end
      out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', root, '--draft')
      assert_equal 0, status.exitstatus, out + err
      assert_equal "rfc_spec_valid\n", out
      assert_empty err
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
      'illegal_state' => [proc { |s| s.gsub("'Disabled','ResolutionRequired')", "'Disabled','ResolutionRequired','Bogus')").sub("NEW.state='PendingDispatch'", "NEW.state IN ('PendingDispatch','Bogus')") },
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

  def business_edge_transaction(from, to, version, reason, overrides = {})
    event = transition_sql({'event_id'=>"'#{sha_id('edge-' + (version+1).to_s)}'",'from_state'=>"'#{from}'",'to_state'=>"'#{to}'",
      'expected_version'=>version.to_s,'result_version'=>(version+1).to_s,
      'previous_sha256'=>version.zero? ? 'NULL' : "'#{version.to_s * 64}'",
      'canonical_sha256'=>"'#{(version+1).to_s * 64}'",'reason'=>"'#{reason}'",
      'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL','occurred_at'=>'10'}.merge(overrides))
    "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='#{to}',version=version+1,reason='#{reason}',updated_at=10 WHERE intent_id='#{sha_id('intent-1')}' AND version=#{version} AND state='#{from}' AND lease_generation=0; #{event} COMMIT;"
  end

  def seed_authority(db)
    sql_ok(db, insert_intent)
    event = transition_sql('event_id'=>"'#{sha_id('event-1')}'",'from_state'=>"'PendingDispatch'",'to_state'=>"'AwaitingAuthority'",
      'expected_version'=>'0','result_version'=>'1','previous_sha256'=>'NULL','canonical_sha256'=>"'#{'1' * 64}'",
      'terminal_ref_id'=>'NULL','terminal_binding_sha256'=>'NULL','reason'=>"'intent.dispatch_claimed'")
    sql_ok(db, "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='AwaitingAuthority',version=1,reason='intent.dispatch_claimed',updated_at=2; #{event} COMMIT;")
  end

  def not_delivered_transaction(from = 'AwaitingAuthority', version = 1, overrides = {}, reason = 'operator.not_delivered')
    event = transition_sql({'event_id'=>"'#{sha_id('manual-' + version.to_s)}'",'from_state'=>"'#{from}'",'to_state'=>"'NotDelivered'",
      'expected_version'=>version.to_s,'result_version'=>(version+1).to_s,'previous_sha256'=>"'#{version.to_s * 64}'",
      'canonical_sha256'=>"'#{(version+1).to_s * 64}'",'actor'=>"'operator'",'reason'=>"'#{reason}'",
      'terminal_disposition'=>"'ManualConfirmedNotDelivered'",'terminal_decision_id'=>"'decision-1'",
      'operator_audit_ref'=>"'operator-audit-1'",'operator_audit_sha256'=>"'#{'b' * 64}'",'occurred_at'=>'10'}.merge(overrides))
    "BEGIN IMMEDIATE; UPDATE push_intents SET previous_state=state,state='NotDelivered',version=version+1,reason='#{reason}',updated_at=10 WHERE intent_id='#{sha_id('intent-1')}' AND version=#{version} AND state='#{from}' AND lease_generation=0; #{event} COMMIT;"
  end

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
    values = values.merge(overrides)
    unless values.key?('terminal_disposition')
      values['terminal_disposition'] = values['to_state'] == "'Completed'" ? "'Accepted'" : 'NULL'
    end
    insert_row('push_intent_transitions', values)
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
      paths << 'docs/push-system/push-system-wbs.v1.json'
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
