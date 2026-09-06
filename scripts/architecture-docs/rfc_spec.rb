# frozen_string_literal: true

require 'json'
require 'digest'
require 'pathname'
require 'yaml'
require_relative 'rfc_inputs'
require_relative 'wbs'

module ArchitectureDocs
  module RfcSpec
    RFC_PATH = 'docs/push-system/push-system-implementation-rfc.md'
    SQL_PATH = 'docs/push-system/push-system-foundation.v1.sql'
    BASELINE = '07781bf386aafdf202851ae928efee8920387058'
    DEPENDENCIES = {
      'input_manifest_sha256' => ['rfc-input-manifest.v1.json', '6a74428f1cc18cc1b0800ab86be19e3d8afdaafd0107d7656a2d3a5857f18aab'],
      'catalog_sha256' => ['push-capability-catalog.v1.json', '0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3'],
      'evidence_manifest_sha256' => ['push-evidence-manifest.v1.json', '54dc705961da7a6deb458009d2125ee612257d82bad3c14b65d25642e09b64fa'],
      'decisions_sha256' => ['grill-decisions-2026-09-02.md', '55354916a4b03401afa771e2f4e149bc1189222fc3c76d89aeb5ad79c086e794']
    }.freeze
    COUNTS = {'kinds' => 65, 'producers' => 102, 'units' => 52, 'evidence' => 195,
              'mapped' => 26, 'durable_kinds' => 23, 'unmapped' => 39, 'states' => 14}.freeze
    STATUSES = {'ACTIVE' => 36, 'INACTIVE' => 22, 'STARVED' => 5, 'OPT-IN' => 2}.freeze
    class Invalid < StandardError; end
    TYPE_FIELDS = {
      'ScheduleOccurrence' => {
        'schedule_occurrence_id' => 'Sha256',
        'namespace' => 'Namespace',
        'unit_id' => 'UnitId',
        'producer_id' => 'ProducerId',
        'schedule_or_trigger_id' => 'NonEmptyText',
        'calendar_id' => 'NonEmptyText',
        'business_date' => 'Date',
        'occurrence_family' => 'NonEmptyText',
        'occurrence_key' => 'NonEmptyText',
        'completion_owner' => 'CatalogOwnerRef',
        'source_contract_id' => 'NonEmptyText',
        'window_start' => 'UtcMicros',
        'window_end' => 'UtcMicros',
        'catch_up_policy' => 'CatchUpPolicy',
        'status' => 'ScheduleStatus',
        'version' => 'u64',
        'reason' => 'ReasonCode',
        'created_at' => 'UtcMicros',
        'updated_at' => 'UtcMicros',
      },
      'ScheduleOccurrenceTransitionRequest' => {
        'schedule_occurrence_id' => 'Sha256',
        'from_status' => 'ScheduleStatus',
        'to_status' => 'ScheduleStatus',
        'expected_version' => 'u64',
        'expected_generation' => 'u64',
        'fence_token' => 'ActivationFence',
        'reason' => 'ReasonCode',
        'evidence_refs' => 'Vec<EvidenceRef>'
      },
      'OperationalReadinessSnapshot' => {
        'snapshot_id' => 'Sha256',
        'captured_at' => 'UtcMicros',
        'business_date' => 'Date',
        'build_commit' => 'GitSha40',
        'activation_generation' => 'u64',
        'scope' => 'ReadinessScope',
        'status' => 'ReadinessStatus',
        'reason' => 'ReasonCode',
        'dependency_refs' => 'Vec<DependencyRef>',
        'affected_unit_ids' => 'Vec<UnitId>',
        'affected_producer_ids' => 'Vec<ProducerId>',
        'recovery_event_id' => 'RecoveryEventId',
        'evidence_refs' => 'Vec<EvidenceRef>',
        'liveness' => 'bool',
        'deployment_ready' => 'bool',
        'exit_disposition' => 'ExitDisposition',
      },
      'RunContext' => {
        'schema_version' => 'u32',
        'run_id' => 'RunId',
        'unit_id' => 'UnitId',
        'namespace' => 'Namespace',
        'business_date' => 'Date',
        'calendar_date' => 'Date',
        'phase' => 'PhaseEpic',
        'trigger' => 'Trigger',
        'occurrence' => 'OccurrenceId',
        'captured_business_time' => 'UtcMicros',
        'activation_generation' => 'u64',
        'build_commit' => 'GitSha40',
        'catalog_sha256' => 'Sha256',
        'source_contract_version' => 'NonEmptyText',
        'template_version' => 'NonEmptyText'
      },
      'PreparedFacts' => {
        'run_context_sha256' => 'Sha256',
        'source_contract_id' => 'NonEmptyText',
        'source_contract_version' => 'NonEmptyText',
        'source_refs' => 'Vec<SourceRef>',
        'canonical_facts' => 'ExactBytes',
        'facts_sha256' => 'Sha256',
        'provider_observed_at' => 'Vec<SourceTime>',
        'verified_empty' => 'bool',
        'model_output_refs' => 'Vec<ModelOutputRef>'
      },
      'SemanticProjection' => {
        'audience' => 'AudienceId',
        'monitor_kind' => 'Option<MonitorKind>',
        'sub_kind' => 'SubKind',
        'occurrence' => 'OccurrenceId',
        'business_subject' => 'SubjectId',
        'severity' => 'Severity',
        'suppression' => 'Suppression',
        'completion_policy_id' => 'NonEmptyText',
        'completion_policy_version' => 'NonEmptyText',
        'evidence_fingerprint' => 'Sha256',
        'template_id' => 'NonEmptyText',
        'template_version' => 'NonEmptyText',
        'canonical_bytes' => 'ExactBytes',
        'sha256' => 'Sha256'
      },
      'PreparedPush' => {
        'intent_id' => 'IntentId',
        'decision_id' => 'DecisionId',
        'unit_id' => 'UnitId',
        'occurrence' => 'OccurrenceId',
        'subject' => 'SubjectId',
        'run_context_sha256' => 'Sha256',
        'prepared_facts_sha256' => 'Sha256',
        'semantic_projection_sha256' => 'Sha256',
        'source_binding' => 'SourceBinding',
        'rendered_bytes' => 'ExactBytes',
        'rendered_sha256' => 'Sha256'
      },
      'VerifiedTerminalRef' => {
        'ref_id' => 'TerminalRefId',
        'authority_class' => 'AuthorityClass',
        'namespace' => 'Namespace',
        'decision_id' => 'DecisionId',
        'attempt_id' => 'Option<AttemptId>',
        'intent_id' => 'IntentId',
        'unit_id' => 'UnitId',
        'occurrence' => 'OccurrenceId',
        'business_date' => 'Date',
        'subject' => 'SubjectId',
        'audience' => 'AudienceId',
        'template_id' => 'NonEmptyText',
        'template_version' => 'NonEmptyText',
        'rendered_sha256' => 'Sha256',
        'terminal_disposition' => 'TerminalDisposition',
        'evidence_sha256' => 'Sha256',
        'durable_schema_version' => 'NonEmptyText',
        'verified_at' => 'UtcMicros',
        'binding_sha256' => 'Sha256'
      },
      'CompatibilityEvidenceRef' => {
        'compat_id' => 'CompatId',
        'intent_id' => 'IntentId',
        'unit_id' => 'UnitId',
        'occurrence' => 'OccurrenceId',
        'configured_channels' => 'Vec<ChannelId>',
        'attempted_channels' => 'Vec<ChannelId>',
        'weak_outcomes' => 'Vec<WeakOutcome>',
        'local_evidence_sha256' => 'Sha256',
        'observed_at' => 'UtcMicros',
        'not_authoritative' => 'TrueLiteral'
      },
      'CompletionPolicy' => {
        'id' => 'NonEmptyText',
        'version' => 'NonEmptyText',
        'completion_owner' => 'CatalogOwnerRef',
        'advance_event' => 'AdvanceEvent',
        'schedule_close_policy' => 'ScheduleClosePolicy',
        'notification_cursor_policy' => 'CursorPolicy',
        'no_data_policy' => 'NoDataPolicy',
        'disabled_policy' => 'DisabledPolicy',
        'retry_policy' => 'RetryPolicy',
        'uncertain_manual_policy' => 'UncertainPolicy',
        'already_terminal_policy' => 'AlreadyTerminalPolicy',
        'allowed_authority' => 'Vec<AuthorityClass>',
        'finalizer_kind' => 'FinalizerKind',
        'retention_class' => 'RetentionClass'
      }
    }.freeze
    REQUIRED_SECTIONS = ['元数据', '范围与事实权限', '阅读导航与规范化规则（PROPOSED）',
      '类型：JobDecision（PROPOSED）', '类型：DeliveryResult（PROPOSED）', '业务完成分支（PROPOSED）',
      'monitor 到 durable 的映射（CURRENT）', '未直接映射的 monitor 类型（CURRENT 状态；PROPOSED 处置）',
      'durable 状态（CURRENT）与应用投影（PROPOSED）', '类型：ReasonCode（PROPOSED）',
      '适配器一致性合同（PROPOSED）', 'Task2 验证边界'].freeze
    MAPPINGS = [
      ["HoldingPlan","HoldingPlan","None"],
      ["HoldingEvent","HoldingEvent","None"],
      ["T0Advice","T0Advice","None"],
      ["CandidateTriggered","CandidateTriggered","None"],
      ["PreopenNewsHot","PreopenNewsHot","None"],
      ["CloseCall","CloseCall","None"],
      ["ForbiddenOps","ForbiddenOps","None"],
      ["PaperTrade","PaperTrade","None"],
      ["ReviewMarket","ReviewMarket","None"],
      ["ReviewLhb","ReviewLhb","None"],
      ["ReviewSignal","ReviewSignal","None"],
      ["ReviewFailure","ReviewFailure","None"],
      ["TomorrowWatch","TomorrowWatch","None"],
      ["EventCalendar","EventCalendar","None"],
      ["ReviewProviderTopN","ReviewProviderTopN","None"],
      ["SectorTop","SectorTop","None"],
      ["SectorAnomaly","SectorAnomaly","None"],
      ["IndustryChain","IndustryChain","None"],
      ["PositionReview","PositionReview","None"],
      ["ReviewBacktest","ReviewBacktest","None"],
      ["WatchlistTracking","WatchlistTracking","None"],
      ["CatalystReview","CatalystReview","None"],
      ["FactorIC","DailyReport","FactorIC"],
      ["SectorTier","DailyReport","SectorTier"],
      ["CapitalVerify","DailyReport","CapitalVerify"],
      ["DailyReport","DailyReport","按请求选择 FactorIC/SectorTier/CapitalVerify 或 None"]
    ].freeze
    JOB_PAYLOADS = {
      'Ready' => 'PreparedPush', 'NoData' => '{reason:ReasonCode,evidence_sha256:Sha256}',
      'Disabled' => '{reason:ReasonCode}', 'BlockedOnInput' => '{reason:ReasonCode,retry_after:Option<UtcMicros>}',
      'Suppressed' => '{reason:ReasonCode,eligible_after:Option<UtcMicros>}',
      'RetryableFailure' => '{reason:ReasonCode,retry_after:Option<UtcMicros>}', 'PermanentFailure' => '{reason:ReasonCode}'
    }.freeze
    DELIVERY = {
      'TransportAccepted' => %w[VerifiedTerminalRef strong policy_bound],
      'TransportRejected' => %w[VerifiedTerminalRef strong never],
      'TransportUncertain' => %w[VerifiedTerminalRef strong never],
      'AlreadyTerminal' => %w[VerifiedTerminalRef strong policy_bound],
      'BestEffortAccepted' => %w[CompatibilityEvidenceRef compat never],
      'PartiallyAccepted' => %w[CompatibilityEvidenceRef compat never],
      'NoChannelConfigured' => %w[ReasonCode compat never],
      'AllChannelsFailed' => %w[CompatibilityEvidenceRef compat never],
      'Blocked' => %w[ReasonCode none never]
    }.freeze
    STATE_PROJECTIONS = {
      'Reserved' => %w[Blocked 否 lease_fenced_first_attempt 否],
      'AttemptInFlight' => %w[Blocked 否 never_until_reconciled 否],
      'AcceptedAuditPending' => %w[Blocked 否 never after_authority_sealed],
      'AcceptedTaskTransitionPending' => %w[Blocked 否 never after_authority_sealed],
      'Delivered' => %w[TransportAccepted/AlreadyTerminal 是 never accepted_binding_only],
      'RejectedAuditPending' => %w[Blocked 否 never_until_reconciled 否],
      'RejectedTaskTransitionPending' => %w[Blocked 否 never_until_reconciled 否],
      'RejectedDurable' => %w[TransportRejected/AlreadyTerminal 是 explicit_authorization_only rejection_proposal_no_cursor],
      'UncertainAuditPending' => %w[Blocked 否 never 否],
      'UncertainTaskTransitionPending' => %w[Blocked 否 never 否],
      'UncertainManualReview' => %w[TransportUncertain/AlreadyTerminal 是 never quarantine_no_cursor],
      'ManualRejectedAuditPending' => %w[Blocked 否 never 否],
      'ManualRejectedTaskTransitionPending' => %w[Blocked 否 never 否],
      'ManualResolvedRejected' => %w[AlreadyTerminal 是 never manual_not_delivered_no_cursor]
    }.freeze
    REASONS = %w[
      schedule.occurrence_conflict
      schedule.window_open
      schedule.deferred
      input.source_recovered
      activation.ready
      schedule.not_trading_day
      schedule.window_not_open
      schedule.window_expired
      schedule.occurrence_closed
      input.source_unavailable
      input.source_unready
      input.evidence_invalid
      input.no_verified_batch
      input.account_snapshot_missing
      input.namespace_violation
      policy.disabled
      policy.starved
      policy.opt_in_disabled
      policy.cooldown_active
      policy.daily_budget_full
      policy.suppressed
      intent.payload_conflict
      intent.expected_version_conflict
      intent.lease_held
      intent.transition_conflict
      transport.rejected
      transport.uncertain
      transport.no_channel_configured
      transport.all_channels_failed
      transport.partially_accepted
      finalizer.terminal_ref_invalid
      finalizer.binding_mismatch
      finalizer.cas_conflict
      finalizer.deadline_exceeded
      finalizer.transition_append_failed
      activation.manifest_mismatch
      activation.generation_conflict
      activation.owner_conflict
      activation.core_unready
      activation.producer_unready
      shadow.semantic_diff
      shadow.side_effect_attempted
      operator.not_delivered
      operator.unauthorized
      operator.evidence_invalid
      operator.resolution_conflict
    ].freeze
    SEMANTIC_CONTRACTS = {
      '身份合同（PROPOSED）' => {
        header: ["规则","函数","有序材料","排除材料","冲突处置","依据"],
        rows: [
          ["PreparedPushIntent","SHA256CanonicalTuple","namespace,unit_id,completion_owner,source_contract_id,occurrence,subject,audience","payload_sha256,rendered_sha256,evidence_sha256","ResolutionRequired"],
          ["TerminalBinding","SHA256CanonicalTuple","ref_id,authority_class,namespace,decision_id,attempt_id,intent_id,unit_id,occurrence,business_date,subject,audience,template_id,template_version,rendered_sha256,terminal_disposition,evidence_sha256,durable_schema_version","verified_at,binding_sha256","Blocked"]
        ],
        error: 'rfc_identity_contract_invalid'
      },
      '终态完成合同（PROPOSED）' => {
        header: ["处置","绑定校验","游标必要策略","时段提案","游标提案","禁止行为","依据"],
        rows: [
          ["Accepted","RequeryExactBinding","AllowedAuthority+BoundCursor+AcceptedBoundOnly","OnAccepted","AdvanceAccepted","NeverInferFromVariant"],
          ["ManualConfirmedAccepted","RequeryExactBinding","AllowedAuthority+BoundCursor+AcceptedBoundOnly+AcceptedOrManualBound","OnAccepted","AdvanceManualAccepted","NeverTransportAccepted"],
          ["Rejected","RequeryExactBinding","RegisteredPolicy","KeepOpen","None","NeverAdvanceOrBlindRetry"],
          ["Uncertain","RequeryExactBinding","QuarantineThenVerifiedManual","KeepOpen","None","NeverAdvanceOrBlindRetry"],
          ["ManualConfirmedNotDelivered","RequeryExactBinding","RegisteredPolicy","KeepOpen","None","NeverAdvance"]
        ],
        error: 'rfc_completion_binding_invalid'
      },
      '适配器一致性合同（PROPOSED）' => {
        header: ["规则","适用对象","规范值","依据"],
        rows: [
          ["p01_owner_group","MU-p01","SharedBusinessOccurrenceOwner"],
          ["n02_authority","MU-news-flash-aggregate","PreserveWindowReservationAttemptSettlement"],
          ["application_contract","GenericCounted,P01Dedicated,N02Dedicated","OneApplicationResultAndFinalizerContract"],
          ["facts_instance","Active,Shadow","SameImmutablePreparedFactsIncludingModelOutputs"],
          ["projection","project","Pure"],
          ["shadow_compare","Shadow","JobDecision,SemanticProjection.sha256,rendered_sha256,ReasonCode,completion_proposal"],
          ["shadow_exclusions","Shadow","attempt_id,latency,diagnostic_timestamp"],
          ["shadow_side_effects","Shadow","None"],
          ["payload_drift","SameIntent","ResolutionRequired"],
          ["empty_source","Prepare","VerifiedEmptyOnly"],
          ["weak_authority","COMPAT,LocalAudit,SinkAttempt,Ok,Log","NeverTransportAccepted"]
        ],
        error: 'rfc_adapter_contract_invalid'
      }
    }.freeze
    PERSISTENCE_CONTRACTS = {
      '持久化条件组与兼容守卫（PROPOSED）' => {
        header: ["规则","对象","规范值","依据"],
        rows: [
          ["decision_origin","job_decision_kind","Ready/NoData/Disabled 是不可变的初始业务决定"],
          ["ready_group","prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256","Ready 必须整组非空且不可变"],
          ["non_send_group","prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256","初始 NoData/Disabled 必须整组 NULL；隔离后仍保持 NULL"],
          ["ready_non_send_state","Ready→NoData/Disabled","保留原 Ready 字节与哈希，不改变 job_decision_kind"],
          ["edge_reason","push_intents.reason","仅允许业务转换表的逐边 ReasonCode，不接受同命名空间任意代码"],
          ["event_reason","push_intent_transitions.reason","必须等于本次 CAS 后的 intent.reason"],
          ["canonical_identifiers","intent_id,transition.event_id,promotion.event_id","64 位小写十六进制 TEXT；应用重算身份内容"],
          ["hash_storage","Sha256/GitSha40","同时校验 TEXT 类型、字符长度、BLOB 字节长度与小写十六进制"],
          ["compat_entry","SQLite CLI schema script",".bail on；持久化 DDL 前快照对象，不用事后补建掩盖缺失"],
          ["compat_inventory","25 个明确 name/type","metadata 与保护 trigger 纳管；拒绝挂在纳管表上的额外 trigger/index；保留独立无关表"],
          ["compat_trust","v1 固定兼容签名与冻结定义登记","比较已登记入口对象字节，不认证同时伪造 metadata 与保护对象的恶意管理员"]
        ],
        error: 'rfc_persistence_invariants_invalid'
      },
      '业务 outbox 字节恢复合同（PROPOSED）' => {
        header: %w[规则 材料 规范值 依据],
        rows: [
          ['prepared_snapshot','prepared_push_bytes','首次 PreparedPush 规范化字节不可变保存'],
          ['first_render','rendered_bytes','首次 render 原始字节不可变保存'],
          ['content_binding','payload_sha256,rendered_sha256','应用重算 SHA 与长度并核对快照绑定；SQLite 仅检查格式'],
          ['restart_reuse','prepared_push_bytes,rendered_bytes','只读取原字节；禁止重新 provider/LLM/render'],
          ['drift','SameIntent','保留原字节并隔离 ResolutionRequired；禁止 UPDATE/REPLACE 覆盖']
        ],
        error: 'rfc_outbox_bytes_invalid'
      },
      "业务意图转换（PROPOSED）" => {
        header: ["起点","终点","发起者","前置条件","持久副作用","禁止副作用","ReasonCode","依据"],
        rows: [
          ["None","PendingDispatch","应用","已冻结事实与稳定身份","插入版本零 intent/outbox","派发先于提交","intent.created"],
          ["None","NoData","应用","已验证为空且策略允许","插入版本零并保留空证据","伪造终态引用或推进通知游标","intent.no_data"],
          ["None","Disabled","应用","显式禁用且策略允许","插入版本零禁用事实","把未就绪当禁用或推进通知游标","policy.disabled"],
          ["PendingDispatch","AwaitingAuthority","dispatcher","有效 lease 与 expected-version CAS","同库 CAS 并追加事件","先发后存或新建逃逸身份","intent.dispatch_claimed"],
          ["PendingDispatch","NoData","应用","冻结空证据与策略及版本 CAS","同库 CAS 并追加事件；保留原 Ready 材料","把来源错误当空或推进通知游标","intent.no_data"],
          ["PendingDispatch","Disabled","应用","显式禁用及版本 CAS","同库 CAS 并追加事件；保留原 Ready 材料","清除待处理事实或推进通知游标","policy.disabled"],
          ["AwaitingAuthority","AwaitingFinalizer","authority 适配器","私有重查精确绑定且策略允许","同库 CAS 并追加事件","仅凭日志或结果枚举晋级","intent.authority_verified"],
          ["AwaitingFinalizer","Completed","finalizer","再次精确绑定且策略允许及版本 CAS","同一事务执行完成事实 CAS 与事件","跨库原子性或跳过事件","finalizer.completed"],
          ["AwaitingAuthority/ResolutionRequired","NotDelivered","已认证操作员与私有 authority 适配器","不投递终态合同的来源、精确绑定、独立审计及版本 CAS 全通过","同库 CAS 与不可变处置事件；解除未决阻断但保留失败","推进游标、重发、撤销 Accepted 或计入成功","operator.not_delivered"],
          ["PendingDispatch/AwaitingAuthority/AwaitingFinalizer/Completed/NoData/Disabled","ResolutionRequired","应用或 finalizer","材料或版本冲突并以重读版本 CAS","保留原材料与终态历史并阻断 Unit 晋级","覆盖材料或撤销既有游标","intent.payload_conflict/intent.expected_version_conflict/finalizer.cas_conflict"],
          ["AwaitingAuthority/AwaitingFinalizer","ResolutionRequired","私有 authority 适配器","未知或处置冲突经重查且版本 CAS","隔离并保留原 decision 与处置证据","自动重发或自动推进通知游标","transport.uncertain/operator.resolution_conflict"],
          ["ResolutionRequired","AwaitingFinalizer","已认证操作员与私有 authority 适配器","Ready 来源且处置清除冲突与原身份精确接受绑定、策略及版本 CAS","保留处置证据并只恢复最终化资格","自动解封或再次发送","intent.authority_verified"],
          ["PendingDispatch/AwaitingAuthority/AwaitingFinalizer/ResolutionRequired","SameState","lease 管理者","owner/until/generation 与版本 CAS","版本加一并追加事件","抢占未过期外来 lease","intent.lease_held/intent.dispatch_claimed"],
          ["AwaitingAuthority","SameState","authority 恢复器","原 decision 与版本 CAS","仅记录拒绝或审计阻塞并追加事件","盲重发或提前最终化","transport.rejected/finalizer.terminal_ref_invalid"],
          ["AwaitingFinalizer","SameState","finalizer","原绑定重查失败与版本 CAS","只保留阻塞原因并追加事件","推进完成或绕过重验","finalizer.terminal_ref_invalid"]
        ],
        error: 'rfc_business_protocol_invalid'
      },
      "激活转换（PROPOSED）" => {
        header: ["起点","终点","发起者","前置条件","持久副作用","禁止副作用","ReasonCode","依据"],
        rows: [
          ["None","Disabled","已认证操作员","批准身份与初始 generation=1","新 manifest 与 Initialize journal","仅凭 manifest 宣称执行","activation.applied"],
          ["Disabled","Shadow","已认证操作员","全部版本绑定与 generation CAS","新 manifest 与 EnterShadow journal","shadow 外部副作用","activation.applied"],
          ["Shadow","Active","已认证操作员","证据通过且无 ResolutionRequired 与 generation CAS","新 manifest 与 Activate journal","双物理 owner 或自动批准","activation.applied"],
          ["Active","Draining","已认证操作员","generation CAS 与停止新增派发","新 manifest 与 Drain journal","删除未决事实或中断恢复","activation.applied"],
          ["Draining","Disabled","已认证操作员","排空证据与 generation CAS","新 manifest 与 Disable journal","把未决状态当已完成","activation.applied"],
          ["Disabled/Shadow/Active/Draining","RollbackTarget","已认证操作员","新 generation CAS 与同 Unit 兼容历史目标","新 manifest 与 Rollback journal 恢复目标 owner","改写历史或破坏性 schema 回滚","activation.applied"]
        ],
        error: 'rfc_activation_protocol_invalid'
      },
      "权威处置与最终化资格（PROPOSED）" => {
        header: ["起点","终点","发起者","前置条件","持久副作用","禁止副作用","ReasonCode","依据"],
        rows: [
          ["Accepted","AwaitingFinalizer","私有 authority 适配器","终态已封存且 TerminalBinding 与 CompletionPolicy 均通过","仅记录完成提案后进入步骤六","把远端接受当业务完成","intent.authority_verified"],
          ["ManualConfirmedAccepted","AwaitingFinalizer","私有 authority 适配器","精确绑定且 AcceptedOrManualBound 策略允许","独立人工接受指标与完成提案","伪装 TransportAccepted","intent.authority_verified"],
          ["AlreadyTerminal","DispositionDependent","私有 authority 适配器","重查 TerminalBinding 并逐处置执行终态完成合同","接受仅推进完成；不投递仅按专门合同收敛","全处置推进或省略绑定","intent.authority_verified/operator.not_delivered/transport.rejected/transport.uncertain"],
          ["AcceptedAuditPending/AcceptedTaskTransitionPending","AwaitingAuthority","恢复器","authority 尚未封存","仅恢复审计与 authority 内部转换","重发或业务最终化","finalizer.terminal_ref_invalid"],
          ["Rejected","AwaitingAuthority","dispatcher","当前显式重试授权及原 decision 与 lease CAS","仅授权时申请新 attempt","盲重试或推进游标","transport.rejected"],
          ["Uncertain","ResolutionRequired","恢复器","权威不确定性已确认","隔离并等待已认证人工解析","自动重发或自动清理","transport.uncertain"],
          ["ManualConfirmedNotDelivered","NotDelivered","已认证操作员与私有 authority 适配器","不投递终态合同的来源、精确绑定、独立审计及版本 CAS 全通过","同库追加不投递终态事实；保留失败指标","推进游标、重发或冒充接受","operator.not_delivered"],
          ["COMPAT/Blocked","AwaitingAuthority","应用","无强 authority 终态","仅保留弱证据或阻塞诊断","构造 VerifiedTerminalRef 或权威完成","finalizer.terminal_ref_invalid"]
        ],
        error: 'rfc_authority_protocol_invalid'
      },
      "跨库恢复顺序（PROPOSED）" => {
        header: ["步骤","执行者","幂等键","已提交可见事实","重启扫描","下一合法动作","禁止行为","依据"],
        rows: [
          ["1","业务应用","intent_id","业务 intent/outbox 本地提交","按稳定身份查询是否已存在","比较不可变材料并取得 lease","派发先于提交或跨库事务"],
          ["2","dispatcher","durable_decision_id","durable reserve/claim","按原 decision 查询 reservation 与 lease","确认未尝试且满足 fencing 后进入步骤三","外来 lease 抢占或新建身份"],
          ["3","authority","durable_decision_id+attempt_id","durable attempt 先于外部尝试记录","查询 attempt 与不确定状态","仅已有合法 attempt 执行一次或进入恢复","在途未知结果盲重发"],
          ["4","authority","durable_decision_id+attempt_id","durable terminal 本地提交并封存","查询原 terminal 及未封存审计","封存后进入步骤五","以 sink attempt 或审计日志冒充终态"],
          ["5","私有 authority 适配器","IdentityRule::TerminalBinding","只读重验不产生新投递事实","从原 authority 再查引用与绑定","资格允许才进入步骤六","复制回执或跨事务复用未重验引用"],
          ["6","finalizer","intent_id+expected_version+event_id","一个业务事务的 CAS 与 transition 共同提交","查业务状态版本和稳定事件","失败整体回滚并重查；成功进入步骤七","CAS 零行追加或事件失败仍提交"],
          ["7","业务应用","intent_id+result_version","提交后的确认与独立完成指标","查询既有 Completed/NotDelivered 和事件","幂等返回原终态事实与独立指标","丢失确认导致二次发送或完成"]
        ],
        error: 'rfc_cross_database_protocol_invalid'
      },
      "故障与提交确认矩阵（PROPOSED）" => {
        header: ["故障标识","已提交事实","恢复扫描","重发许可","幂等键","目标状态","ReasonCode","依据"],
        rows: [
          ["before_intent_commit","无新业务事实","按 intent_id 重算后查库","仅首次且完整门禁通过","intent_id","PendingDispatch","intent.created"],
          ["after_intent_commit","intent/outbox","扫描未完成 intent","查询 durable 后仅允许首次","intent_id+durable_decision_id","PendingDispatch","intent.created"],
          ["before_claim","intent/outbox","按原 decision 查询 reservation","仅确认无 attempt 后首次","durable_decision_id","AwaitingAuthority","intent.dispatch_claimed"],
          ["after_claim","reservation","查询 claim 与有效 lease","仅确认未尝试且 lease 有效","durable_decision_id","AwaitingAuthority","intent.dispatch_claimed"],
          ["before_attempt","reservation","查询是否已记录 attempt","仅确认未尝试且 lease 有效","durable_decision_id+attempt_id","AwaitingAuthority","intent.dispatch_claimed"],
          ["after_attempt","attempt 可能已外发","查询原 attempt 并协调未知结果","否","durable_decision_id+attempt_id","ResolutionRequired","transport.uncertain"],
          ["before_terminal_commit","attempt 或待封存审计","恢复原 authority 并查询未知结果","否","durable_decision_id+attempt_id","AwaitingAuthority/ResolutionRequired","transport.uncertain"],
          ["after_terminal_commit","durable Accepted terminal；确认可能丢失","查询原 terminal 并重新验证绑定","否","IdentityRule::TerminalBinding","AwaitingFinalizer","intent.authority_verified"],
          ["before_reverify","durable terminal","私有 authority 重查绑定与资格","否","IdentityRule::TerminalBinding","AwaitingFinalizer","intent.authority_verified"],
          ["after_reverify","durable Accepted terminal；引用仅在内存","重新查询而非恢复内存引用","否","IdentityRule::TerminalBinding","AwaitingFinalizer","finalizer.terminal_ref_invalid"],
          ["after_business_cas","旧业务提交事实；CAS 尚未提交","事务恢复回滚后查状态版本","否","intent_id+expected_version+event_id","AwaitingFinalizer","finalizer.transition_append_failed"],
          ["after_transition_append","旧业务提交事实；事件尚未提交","事务恢复回滚后查状态与事件","否","intent_id+expected_version+event_id","AwaitingFinalizer","finalizer.transition_append_failed"],
          ["after_business_commit","Completed 与事件；确认可能丢失","查既有终态及稳定事件并幂等确认","否","intent_id+result_version+event_id","Completed","finalizer.completed"],
          ["sqlite_busy","最后一次提交事实","有界退避后查两库原身份","不得仅因 busy 重发","intent_id+durable_decision_id","SameState","intent.lease_held"],
          ["foreign_lease","其他 owner 的有效 lease","等 lease 到期并重新读 generation","否","intent_id+lease_generation+version","SameState","intent.lease_held"],
          ["expired_lease","过期 lease 与原 decision","generation 与版本 CAS 后查询 durable","仅查询证明确未尝试或有显式拒绝重试授权","intent_id+lease_generation+version","SameState","intent.dispatch_claimed"],
          ["accepted_audit_pending","Accepted 的未封存审计","仅修复 authority 审计和内部转换","否","durable_decision_id+attempt_id","AwaitingAuthority","finalizer.terminal_ref_invalid"],
          ["rejected_retry","已封存 Rejected","重新核对当前显式授权与 lease","仅显式授权产生新 attempt","durable_decision_id+new_attempt_id","AwaitingAuthority","transport.rejected"],
          ["uncertain","权威未知结果","隔离并等待已认证人工解析","否","durable_decision_id+attempt_id","ResolutionRequired","transport.uncertain"],
          ["not_delivered_before_business_commit","不投递 terminal 与独立 operator audit 已封存","重查原 decision 精确绑定及版本；仅恢复业务终态事务","否","intent_id+expected_version+event_id","NotDelivered","operator.not_delivered"],
          ["not_delivered_after_business_commit","NotDelivered 与不可变事件；确认可能丢失","查询原事件与关联 audit；返回已处置失败而非接受","否","intent_id+result_version+event_id","NotDelivered","operator.not_delivered"],
          ["payload_drift","原身份及不可变材料","重读并 CAS 隔离；保留冲突证据","否","intent_id","ResolutionRequired","intent.payload_conflict"],
          ["expected_version_conflict","获胜者提交事实","回滚本事务并重读 CAS 隔离","否","intent_id+expected_version","ResolutionRequired","intent.expected_version_conflict"],
          ["terminal_ref_invalid","原 durable 与业务事实","私有 authority 重新核验","否","IdentityRule::TerminalBinding","AwaitingAuthority/AwaitingFinalizer","finalizer.terminal_ref_invalid"],
          ["business_finalization_failure","原 durable terminal 与旧业务事实","整体回滚后重查；只重做最终化","否","intent_id+expected_version+event_id","AwaitingFinalizer","finalizer.transition_append_failed"],
          ["activation_rollback","历史 manifest 与已执行 journal","核对最新已执行 generation 与兼容目标","回滚本身不授权发送","unit_id+new_generation","RollbackTarget","activation.applied"],
          ["promotion_commit_ack_lost","新 generation journal 可能已提交","按 Unit/generation 查询而非再执行切换","否","unit_id+generation+event_id","ExistingExecutedState","activation.generation_conflict"],
          ["manifest_without_journal","新期望 manifest；尚无执行事实","比较 manifest 与 journal 并阻断就绪","否","unit_id+generation","Blocked","activation.manifest_mismatch"]
        ],
        error: 'rfc_fault_matrix_invalid'
      },
    }.freeze
    # v1 规范表的固定语义；只校验结构化单元格，不复制叙述或整份 RFC 快照。
    ROLLOUT_CONTRACTS = {
      "不投递终态合同（PROPOSED）" => {
        header: ["规则","适用范围","规范值","依据"],
        rows: [
          ["state","BusinessIntentState","NotDelivered；独立业务终态，不增加 durable 的十四态或 DeliveryResult 分支"],
          ["entry","AwaitingAuthority/ResolutionRequired","仅 Ready；后者最近进入隔离必须来自同 intent/decision 的 AwaitingAuthority+transport.uncertain；历史不得已有 AwaitingFinalizer/Completed"],
          ["verification","VerifiedTerminalRef","已认证操作员与生产 allowlist；私有 authority 重查精确 ManualConfirmedNotDelivered 绑定、外部证据哈希与独立 operator audit"],
          ["commit","ExpectedVersionCAS","同一业务事务 CAS 与追加 event；包含 terminal_ref_id、terminal_disposition、terminal_decision_id、binding SHA、operator audit 引用与 SHA"],
          ["storage_trust","SQLite/Application","SQL 验证边、原 decision、字段组与格式；应用验证身份认证、authority 真实性、hash 内容与事务包装"],
          ["cursor","NotDelivered","永不推进通知游标；不授权重发；不作为 Accepted 或 ProductionVerified 成功样本"],
          ["recovery","CommitAckLost","重查原 terminal 后只补本地终态事务；已提交时返回原 event；禁止再次外发"],
          ["gate","ResolvedButFailed","解除未解决 ResolutionRequired/Uncertain 阻断；failure 门禁及失败指标仍保留，不自动批准晋级"],
          ["retention","TerminalEvidence","关联原 intent、decision、transition、operator audit；满足最严格保留及清理资格才可清理，不因已处置立即删除"],
          ["rollback","AcceptedHistory","NotDelivered 无离开边；AwaitingFinalizer/Completed 及其隔离历史不得撤销为不投递"]
        ],
        error: 'rfc_not_delivered_contract_invalid'
      },
      "运行里程碑（PROPOSED）" => {
        header: ["标识","名称","前置条件","完成条件","本批状态","依据"],
        rows: [
          ["FoundationReady","Foundation Ready","TypedResultThenFinalizerReconcilerAndCompatibleSchemas","NoOwnerChange+EachAuthorityControlledAcceptedAndSameDecisionAlreadyDeliveredNoSecondSend+TestRestore+ParallelIsolation","NotAttained"],
          ["P0ProductionVerified","P0 Production Verified","FoundationReady+Q44ApprovedNonNullWaveUnits","EachApplicableP0UnitFreshSixGatesAndAuthorizedNaturalOrLowFrequencyEvidence+RequiredChannelReceipts","NotAttained"],
          ["ArchitectureReleaseCandidate","Architecture Release Candidate","FoundationReady+All52UnitsImplemented","42OwnerChangingAnd10ConformanceOnlyCodeContractsTestsComplete+NoImplicitActivation+DeletionGatesBeforeCleanup","NotAttained"],
          ["ProgramProductionVerified","Program Production Verified","ArchitectureReleaseCandidate+All52UnitsVerified","CompleteCatalog+AllUnitsComplete+NoAccidentalActivationUncertainBacklogDuplicateReceiptGap+TailCleanup+FreshEvidence","NotAttained"]
        ],
        error: 'rfc_runtime_milestones_invalid'
      },
      "运行退出验收（PROPOSED）" => {
        header: ["规则","适用范围","规范值","依据"],
        rows: [
          ["unit_inventory","CurrentCatalog","52 Units；42 owner-changing 与 10 conformance-only；不按 PushKind/count 推导 owner"],
          ["nullable_waves","Q44","只使用 WBS 当前非空批准波次；null 不代表遗漏、不自动赋予第十一波或生产授权；全部 52 Unit 仍在项目退出范围"],
          ["foundation_greybox","EachAuthoritativeRequiredChannel","受控 Accepted 与同一 decision 的 AlreadyDelivered/no-second-send；弱 COMPAT 或人工接受不得替代"],
          ["unit_greybox","EachUnit","自然 occurrence；低频仅经批准确定性灰盒；conformance-only 验证原 owner 而非虚构接管"],
          ["parallel_tests","DefaultParallelCI","默认并行无无法解释失败；进程全局状态测试隔离或显式强制串行并记录范围，禁止隐匿失败"],
          ["backup_restore","BusinessDBAndDurableDB","分别备份并记录各自 hash 与边界；在 Test 恢复并对账；不是跨库原子快照"],
          ["old_path_delete","PriorUnit","Accepted、same-decision replay、restart、fault、有效 session、Uncertain 全部门禁通过后，才在后续版本删除旧路径"],
          ["release_pipeline","ReleaseNAndNPlus1","N 接管当前 Unit；N+1 清理前一 Unit 并可晋级下一 Unit；最后单独完成 tail cleanup"],
          ["program_exit","All52Units","目录完整、所有 Unit 完成、无意外激活、未解决 Uncertain、陈旧 backlog、duplicate、receipt 缺口；清理结束并有 fresh evidence"],
          ["failure_retained","NotDelivered","已处置不等于发送成功；保留失败指标与 failure 门禁，不能冲抵成功回执缺口"],
          ["publication_boundary","ImplementationReady","仅文档发布资格；与四级 runtime milestone 正交，本批四级均未达到；后续 HTML/CI 发布不证明生产"],
          ["historical_estimate","Q41","36–69 工程人日与 7–10 交易周是目录冻结前暂估；现行机器 WBS 重新建立基线，保留完整范围"],
          ["priority","Q22Q26","先修假成功、过早状态与语义分裂；C0–C6 仅能力标签；Foundation→垂直 Unit→尾部清理"]
        ],
        error: 'rfc_runtime_exit_invalid'
      },
      "外部兼容（PROPOSED）" => {
        header: ["表面","保持项","内部边界","验收","破坏性变化","依据"],
        rows: [
          ["cli","Invocation+Arguments+ExitStatus+Output","BoolToTypedResultViaCompatibilityAdapter","ExistingInvocationGoldenArgsExitStdoutStderr+InvalidArgsAndModeMatrix","SeparateVersionedDecision+Unit+Acceptance"],
          ["config","Key+Default+Scope","PreserveExistingParsingDefaultsAndNamespace","ExistingKeyDefaultScopeGolden+MissingInvalidCrossScopeCases","SeparateVersionedDecision+Unit+Acceptance"],
          ["subscription","Subscription+Audience+RequiredChannels","PreserveRoutingAndCompletionPolicy","SameSubscriptionAudienceChannelSet+MissingRequiredChannelRefusal","SeparateVersionedDecision+Unit+Acceptance"],
          ["template","TemplateId+Version+RenderedBytes","InfrastructureMigrationNeverChangesWordingOrTemplate","SameFactsIdVersionExactFirstRenderedBytes+ReplayNoRerender","SeparateVersionedDecision+Unit+Acceptance"],
          ["authority","COMPATWeakEvidence","NeverTransportAcceptedOrVerifiedTerminalRefOrCursorAdvance","WeakOutcomeMatrixRejectsAuthorityUpgradeAndCursorMutation","SeparateVersionedDecision+Unit+Acceptance"]
        ],
        error: 'rfc_external_compatibility_invalid'
      },
      '调度版本与转换提交（PROPOSED）' => {
        header: %w[规则 适用范围 规范值 依据],
        rows: [
          ['storage_owner', 'ScheduleOccurrenceStateAndTransitionEvidence', 'BusinessDBSameTransactionIndependentOfPushIntentVersion'],
          ['initial_state', 'FirstUniqueOccurrenceInsert', 'Expected'],
          ['initial_version', 'FirstUniqueOccurrenceInsert', 'Zero'],
          ['create_conflict', 'ExistingScheduleOccurrenceId', 'ReadExistingNeverOverwriteOrReset'],
          ['request_guard', 'EveryLifecycleTransition', 'ExactIdFromStatusExpectedVersion'],
          ['fence_guard', 'EveryLifecycleTransition', 'CurrentUnitGenerationManifestOwnerAndExpectedGeneration'],
          ['lifecycle_guard', 'EveryLifecycleTransition', 'RegisteredEdgeReasonAuthorityWindowAndEvidence'],
          ['success_version', 'ExactlyOneRowCAS', 'CheckedExpectedVersionPlusOne'],
          ['overflow', 'ExpectedVersionAtU64Max', 'RefuseNoWritesNoEvents'],
          ['atomic_commit', 'SuccessfulTransition', 'StateVersionReasonAndTransitionEvidenceOneBusinessTransaction'],
          ['zero_rows', 'FailedCAS', 'NoStateOrVersionWriteNoEventNoPrepareProviderLLMSinkCursorOrder'],
          ['conflict_recovery', 'schedule.occurrence_conflict', 'RereadOccurrenceAndCurrentFenceReevaluateNeverBlindRetry'],
          ['identity_version', 'OccurrenceVersionAndRequestExpectedVersion', 'ExcludedFromScheduleOccurrenceId'],
          ['commit_ack_unknown', 'SameOccurrenceAndProposedResultVersion', 'RequeryOccurrenceAndVersionEventBeforeAnyNewRequest']
        ],
        error: 'rfc_schedule_version_invalid'
      },
      "调度身份（PROPOSED）" => {
        header: ["规则","函数","有序材料","排除材料","依据"],
        rows: [
          ["ScheduleOccurrence","SHA256CanonicalTuple","schema_version,namespace,unit_id,producer_id,schedule_or_trigger_id,calendar_id,business_date,occurrence_family,occurrence_key,completion_owner,source_contract_id","wall_clock_tick,phase_epic,activation_generation,version,expected_version,build,payload_sha256,rendered_sha256,evidence_sha256"],
        ],
        error: 'rfc_schedule_identity_invalid'
      },
      "调度生命周期（PROPOSED）" => {
        header: ["起点","终点","权威","窗口与版本条件","持久事实","禁止副作用","ReasonCode","依据"],
        rows: [
          ["Expected","Eligible","MarketSession","WindowOpen+CurrentVersion+ReadyGate","EligibilityEvent","ProviderOrSend","schedule.window_open"],
          ["Eligible","Prepared","PhysicalOwner","WindowOpen+CurrentFence+VersionCAS","FrozenDecisionOrIntentRef","DispatchBeforeCommit","intent.created"],
          ["Prepared","Closed","CompletionPolicy","BoundScheduleCloseProposal+VersionCAS","ScheduleClosureEvent","InferNotificationCursor","schedule.occurrence_closed"],
          ["Expected","Missed","MarketSession","WindowExpired+NoCatchUp+VersionCAS","MissedEvent","StalePrepareOrSend","schedule.window_expired"],
          ["Eligible","Missed","MarketSession","WindowExpired+NoCatchUp+VersionCAS","MissedEvent","StalePrepareOrSend","schedule.window_expired"],
          ["Expected","Deferred","MarketSession","NextEligibleSession+VersionCAS","DeferredEvent+NextEligibilityRef","PrepareOrSend","schedule.deferred"],
          ["Eligible","Deferred","MarketSession","NextEligibleSession+VersionCAS","DeferredEvent+NextEligibilityRef","PrepareOrSend","schedule.deferred"],
          ["Expected","BlockedOnInput","SourceContract","UnavailableEvidence+VersionCAS","InputBlockEvent","EmptyAsNoDataOrPollPermanentGap","input.source_unavailable"],
          ["Eligible","BlockedOnInput","SourceContract","UnavailableEvidence+VersionCAS","InputBlockEvent","EmptyAsNoDataOrPollPermanentGap","input.source_unavailable"],
          ["BlockedOnInput","Eligible","SourceContract+MarketSession","RecoveryEvent+WindowOpen+ReadyGate+VersionCAS","InputRecoveryEvent","InventProducerOrIdentity","input.source_recovered"],
          ["BlockedOnInput","Missed","MarketSession","WindowExpired+NoCatchUp+VersionCAS","MissedEvent","StalePrepareOrSend","schedule.window_expired"],
          ["BlockedOnInput","Deferred","MarketSession","NextEligibleSession+VersionCAS","DeferredEvent+NextEligibilityRef","PrepareOrSend","schedule.deferred"],
          ["Deferred","Eligible","MarketSession","NextEligibilityReached+ReadyGate+VersionCAS","DeferredRecoveryEvent","InventProducerOrIdentity","schedule.window_open"],
        ],
        error: 'rfc_schedule_lifecycle_invalid'
      },
      "调度恢复策略（PROPOSED）" => {
        header: ["规则","适用范围","规范值","依据"],
        rows: [
          ["ExpireWithoutCatchUp","NewOccurrence","ExpiredMeansMissedNoSend"],
          ["schema_version","ScheduleOccurrence","ScheduleOccurrence/v1"],
          ["SameBusinessDayBeforeDeadline","NewOccurrence","SameBusinessDateAndBeforeWindowEndOnly"],
          ["DeferToNextEligibleSession","NewOccurrence","PreserveIdentityAndLinkNextSession"],
          ["RecoverPersistedOnly","ExistingIntentOrDecision","OriginalIdentityAndBytesNoPrepareNoWindowOverride"],
          ["coalesce","Tick+StartupCatchUp+NormalDue","SameScheduleOccurrenceIdOnly"],
          ["non_trading_day","SessionBound","NoOccurrence"],
          ["non_trading_reason","schedule.not_trading_day","EvaluationOnlyNoOccurrenceNoNoDataOrDisabledIntent"],
          ["independent_trigger","CatalogSessionIndependentEventOrManual","AuthorityBusinessDateRequired"],
          ["INACTIVE","CatalogMetadata","NoTimerNoProducer"],
          ["STARVED","ExistingProducer","PreserveStateUntilProductAndInputApproval"],
          ["OPT-IN","ExistingProducer","PreserveStateUntilExplicitProductApproval"],
        ],
        error: 'rfc_schedule_recovery_invalid'
      },
      "运行就绪判定（PROPOSED）" => {
        header: ["状态","范围","权威","存活","部署就绪","退出处置","恢复事件","ReasonCode","依据"],
        rows: [
          ["Ready","EvaluatedScope","OperationalReadinessSnapshot","true","true","Continue","ReadyObserved","activation.ready"],
          ["CoreUnready","SharedPrerequisites","OperationalReadinessSnapshot","UntilControlledExit","false","StartupNonzeroOrStopNewAndRecoverIsolateThenNonzero","CoreDependenciesRestored","activation.core_unready"],
          ["ProducerUnready","AffectedProducers","OperationalReadinessSnapshot","true","false","IsolateAffectedContinueOthers","ProducerContractRestored","activation.producer_unready"],
          ["BlockedOnInput","KnownOccurrence","OperationalReadinessSnapshot","true","true","ContinueWithoutOccurrenceWork","InputEvidenceRestored","input.source_unavailable"],
        ],
        error: 'rfc_readiness_invalid'
      },
      "就绪查询与恢复合同（PROPOSED）" => {
        header: ["规则","适用范围","规范值","依据"],
        rows: [
          ["authority","Health+Readiness+CLI","SameOperationalReadinessSnapshot"],
          ["query_effects","AllQueries","NoProviderNoSinkNoTransition"],
          ["log_pager","Logs+OptionalPager","ProjectionOnlyNeverReadinessAuthority"],
          ["non_ready","CoreUnready+ProducerUnready+BlockedOnInput","StableReasonAffectedSetsQueryableRecoveryEvent"],
          ["missing_active_contract","Source+Schedule+Presentation+Policy","EscalateProducerUnready"],
          ["input.source_unready","RegisteredContractOccurrenceEvidenceUnavailable","BlockedOnInput"],
          ["activation.producer_unready","ActiveProducerContractMissing","ProducerUnready"],
          ["alert","ProducerUnready","IndependentOperationalAlert"],
          ["recovery","AllScopes","AppendPendingOrRecoveredEventThenNewSnapshot"],
        ],
        error: 'rfc_readiness_query_invalid'
      },
      "物理所有权与晋级合同（PROPOSED）" => {
        header: ["规则","适用范围","规范值","依据"],
        rows: [
          ["common_fence","LegacyAndNewSchedulerProducerDispatcherFinalizer","unit_id,generation,manifest_sha256,physical_owner"],
          ["authorization","EveryActor","CurrentGateAndFenceRequired"],
          ["Shadow","PhysicalOwner","None"],
          ["Active","NewOccurrence","ManifestOwnerOnly"],
          ["Draining","NewOccurrenceAndPrepare","ForbiddenPreserveAuthorityFinalizerReconcilerQuarantine"],
          ["Disabled","PersistedFacts","PreserveAndFenceOldOwnerAgainstResend"],
          ["daily_limit","NonEmergencyOwnerChangingPromotion","OneUnitPerBusinessDate"],
          ["no_quota","ShadowOrNoOwnerChangeDeployment","DoesNotConsumeDailyPromotion"],
          ["emergency_rollback","AnyTime","NewGenerationCASAppendJournalBlockLaterPromotionToday"],
          ["parallel_shadow","MultipleUnits","ExactlyOneOwnerPerOccurrence"],
          ["rollback_compatibility","LogicalOrFoundationCompatibleNMinusOne","PreserveAcceptedPendingFactsAndFences"],
          ["quota_transaction","ActivationDB","BEGIN IMMEDIATE"],
          ["quota_calendar","AuthorityBusinessDate","CalendarBoundUTCStartInclusiveEndExclusive"],
          ["quota_query","AllUnitsPromotionJournalOccurredAt","RejectAnyActivateOrRollbackInBusinessDateInterval"],
          ["quota_apply","SameImmediateTransaction","RevalidateGenerationThenAppendManifestAndJournalCommit"],
          ["quota_authority","MemoryLockOrLogs","NeverSufficient"],
        ],
        error: 'rfc_activation_operations_invalid'
      },
      "风险波次顺序（PROPOSED）" => {
        header: ["顺序","波次","晋级规则","依据"],
        rows: [
          ["1","CLI report typed BestEffort result","OneUnitPerBusinessDate"],
          ["2","09:05 chain","OneUnitPerBusinessDate"],
          ["3","15:30 chain","OneUnitPerBusinessDate"],
          ["4","AttributionDaily","OneUnitPerBusinessDate"],
          ["5","G5bAttribution","OneUnitPerBusinessDate"],
          ["6","15:05 snapshot occurrence","OneUnitPerBusinessDate"],
          ["7","CandidateBoard + CandidateInvalidated","OneUnitPerBusinessDate"],
          ["8","LimitBoards","OneUnitPerBusinessDate"],
          ["9","ReviewTask result semantics","OneUnitPerBusinessDate"],
          ["10","PaperReview-Starved conformance","OneUnitPerBusinessDate"],
        ],
        error: 'rfc_risk_waves_invalid'
      },
      "影子精确比较（PROPOSED）" => {
        header: ["比较项","输入约束","判等规则","差异处置","依据"],
        rows: [
          ["RunContext","SameInstance","ExactCapturedBusinessContext","BlockUnit:shadow.semantic_diff"],
          ["PreparedFacts","SameImmutableInstance","IncludingCapturedModelOutputs","BlockUnit:shadow.semantic_diff"],
          ["JobDecision","SharedFacts","ExactVariantAndAllFields","BlockUnit:shadow.semantic_diff"],
          ["SemanticProjection.sha256","SharedFacts","ExactSha256","BlockUnit:shadow.semantic_diff"],
          ["PreparedPush.rendered_sha256","SharedFacts","ExactSha256AndBytes","BlockUnit:shadow.semantic_diff"],
          ["ReasonCode","SharedFacts","ExactCode","BlockUnit:shadow.semantic_diff"],
          ["completion_proposal","SharedFacts","ExactScheduleAndCursorProposal","BlockUnit:shadow.semantic_diff"],
          ["exclusions","ComparisonOnly","attempt_id,latency,diagnostic_timestamp","NoOtherExclusions"],
        ],
        error: 'rfc_shadow_compare_invalid'
      },
      "影子副作用（PROPOSED）" => {
        header: ["副作用","许可","失败处置","依据"],
        rows: [
          ["provider_second_call","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["llm_recompute","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["business_db_write","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["durable_db_write","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["cursor_advance","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["candidate_watchlist_outcome","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["paper_order_fill","Forbidden","BlockUnit:shadow.side_effect_attempted"],
          ["transport_send","Forbidden","BlockUnit:shadow.side_effect_attempted"],
        ],
        error: 'rfc_shadow_effects_invalid'
      },
      "操作员请求与输出（PROPOSED）" => {
        header: ["方向","字段","类型","不变量","依据"],
        rows: [
          ["Request","command_id","Sha256","StableCommandIdentity"],
          ["Request","command","OperatorCommand","inspect/reconcile/resolve-uncertain/promote/rollback"],
          ["Request","target","TypedTargetRef","TargetTypeAndExactId"],
          ["Request","expected_version","u64","CurrentVersionCAS"],
          ["Request","expected_generation","u64","CurrentGenerationCAS"],
          ["Request","dry_run","bool","RequiredForEveryCommand"],
          ["Request","authenticated_operator_ref","AuthenticatedOperatorRef","HostOrServiceIdentityAndProductionAllowlist"],
          ["Request","reason","ReasonCode","StableNamespacedCode"],
          ["Request","evidence_refs","Vec<TypedEvidenceRef>","ProtectedURIAndSHA256AndTypeAndVersion"],
          ["Request","requested_at","UtcMicros","CapturedRequestTime"],
          ["Response","decision","OperatorDecision","Inspected/Planned/Applied/Refused"],
          ["Response","before_refs","Vec<TypedStateRef>","ExactBeforeSnapshot"],
          ["Response","after_refs","Vec<TypedStateRef>","AppliedOrExplicitlyProjected"],
          ["Response","affected_rows","u64","ZeroForDryRunRefusalOrInspect"],
          ["Response","mutation_journal_event_ref","Option<MutationEventRef>","AppliedMutationOnlyNullForInspectDryRunRefusal"],
          ["Response","operator_audit_event_ref","OperatorAuditEventRef","IndependentControlPlaneEnvelopeReference"],
          ["Response","refusal_reason","Option<ReasonCode>","RequiredWhenRefused"],
          ["Response","snapshot_sha256","Sha256","CanonicalResponseAndEvidenceBinding"],
        ],
        error: 'rfc_operator_wire_invalid'
      },
      "操作员命令（PROPOSED）" => {
        header: ["命令","目标","最小证据","允许行为","拒绝原因","演练","依据"],
        rows: [
          ["inspect","UnitOrIntentOrDecision","AuthenticatedIdentity+TargetRef","ReadOnlySnapshot","operator.unauthorized","SupportedNoWrites"],
          ["reconcile","IntentOrUnit","CurrentVersionFence+AuthorityRefs","DeterministicIdempotentRecoveryOnly","intent.expected_version_conflict","SupportedNoWrites"],
          ["resolve-uncertain","Decision","PriorInspect+ExactTerminalBinding+CurrentFenceVersion+ExternalEvidenceHash","AppendManualDispositionNeverRewriteReceipt","operator.resolution_conflict","SupportedNoWrites"],
          ["promote","Unit","CurrentManifest+SixFreshGates+WaveRank+DailyJournal+OnlineApproval","OneOwnerCASAppendJournal","activation.generation_conflict","SupportedNoWrites"],
          ["rollback","Unit","CompatibleRollbackTarget+CurrentFenceVersion+AuthenticatedRollbackPermission","NewGenerationCASAppendJournalPreserveAcceptedPending","activation.generation_conflict","SupportedNoWrites"],
        ],
        error: 'rfc_operator_commands_invalid'
      },
      "操作员权限（PROPOSED）" => {
        header: ["规则","认证与审批","规范值","依据"],
        rows: [
          ["SingleControl","AuthenticatedOnlineUserOrProductionAllowlistedOperator","V1BaselineOneMayApproveAndExecute"],
          ["DualControl","ExternalUnitOrOrganizationPolicy","DistinctAuthenticatedPreparerAndApproverCannotDowngrade"],
          ["emergency_rollback","AuthenticatedOperatorWithRollbackPermission","SingleOperatorAllowedAuditRequired"],
          ["Codex","EvidenceAndCommandPreparation","NeverProductionApproverOrExecutor"],
          ["dry_run_and_refusal","EveryCommand","NoDBNoJournalNoOwnerChangeNoProviderNoLLMNoSinkNoOrder"],
          ["unauthorized","MissingOrFreeTextIdentity","Refuse:operator.unauthorized"],
          ["invalid_evidence","MissingInvalidOrUnboundEvidence","Refuse:operator.evidence_invalid"],
          ["stale_version","ExpectedVersionConflict","Refuse:intent.expected_version_conflict"],
          ["stale_generation","ExpectedGenerationConflict","Refuse:activation.generation_conflict"],
          ["binding_conflict","TerminalOrOwnerBindingMismatch","Refuse:operator.resolution_conflict"],
          ["refusal_audit","OperatorAuditEnvelope","IndependentControlPlaneAuditSinkOnly"],
          ["audit_envelope","AuthenticatedIdentityOrUnauthenticatedMarker","CommandHashTimeReasonEvidenceHashSnapshotHash"],
          ["dry_run_refusal_storage","BusinessDurableActivationDBAndPromotionJournal","NoWrites"],
        ],
        error: 'rfc_operator_authorization_invalid'
      },
      "证据保留类别（PROPOSED）" => {
        header: ["类别","起算条件","最低策略","存储与清理","依据"],
        rows: [
          ["NonTerminal","UntilVerifiedTerminal","NeverAutoDelete","PreserveIncludingUncertainAndResolutionRequired"],
          ["MigrationEvidence","TerminalAndProductionVerified","AtLeast90DaysAfterBoth","CleanupEligibilityAllRequired"],
          ["DeliveryAuditRegulatory","ApplicableRegulatoryStart","StrictlyGreaterThanFiveYears","ExternalWORMOrObjectLockNeverRewrite"],
          ["ModelDecisionTrade","ApplicablePolicyStart","StrictestRegulatoryModelTradeSourcePolicy","NoUnifiedFiveYearMaximum"],
        ],
        error: 'rfc_retention_invalid'
      },
      "清理资格与安全（PROPOSED）" => {
        header: ["条件","规范值","依据"],
        rows: [
          ["terminal_binding","VerifiedExactLegalBindingRequired"],
          ["transition_journal_audit","AllLinkedIntegrityVerifiedRequired"],
          ["retention_expiry","StrictestApplicablePolicyExpiredRequired"],
          ["legal_hold","AbsentRequired"],
          ["disclosure","MinimumMetadataProtectedURIHashOnly"],
          ["backup_integrity","IndependentBackupsAndIntegrityEvidenceRequired"],
          ["nonterminal_uncertain_resolution","NeverAutoDelete"],
          ["worm_mutation","Forbidden"],
          ["secrets_and_unnecessary_content","NoKeysCookiesWebhookURLUnnecessaryBodyOrPositions"],
        ],
        error: 'rfc_cleanup_invalid'
      },
      "通用晋级门禁（PROPOSED）" => {
        header: ["门禁","输入","通过证据","失败原因","阻断晋级","依据"],
        rows: [
          ["unit","CurrentUnitBuildCatalogContracts","TypedDecisionsExactBindingsAndAllBranches","input.evidence_invalid","true"],
          ["failure","TestNamespaceFaultMatrix","RejectionUncertainIsolationNoFalseCompletion","transport.uncertain","true"],
          ["crash","SevenStepCommitBoundaries","OriginalIdentityBytesRecoveryNoLostIntent","intent.transition_conflict","true"],
          ["shadow","SharedContextFactsAndEffectCounters","ExactCompareZeroForbiddenEffects","shadow.semantic_diff","true"],
          ["dedup","SameDecisionReplayAndBusinessRevisions","NoSecondSendNoOverDedupExactReceipt","intent.payload_conflict","true"],
          ["rollback","PendingAcceptedUncertainAndOldOwner","NewGenerationJournalFenceNoResend","activation.owner_conflict","true"],
        ],
        error: 'rfc_rollout_gates_invalid'
      },
      "业务验收样本（PROPOSED）" => {
        header: ["样本","基线身份","必须证明","禁止结论","门禁","来源路径","章节定位","依据"],
        rows: [
          ["historical_backfill_2026-08-31","CURRENT_65_KIND:TomorrowWatch+PositionReview","OriginalBusinessDateStableDecisionExactReplay","SendDateEqualsBusinessDateOrLogMeansAccepted","unit,dedup,crash","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F01/§9"],
          ["n02_receipt_time","CURRENT_65_KIND:NewsFlashAggregated","ReceiptAcceptedAtSeparateFromSourceAnalyticsTime","WindowTimeMeansDeliveryTimeOrMissingMeansLoss","unit,shadow","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F01/§F10"],
          ["g5b_test_namespace","CURRENT_65_KIND:G5bAttribution","TestRecordsRejectedBeforeProductionProviderLLMSink","SymbolPrefixReplacesNamespaceOrDeletePollution","failure,unit,shadow","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F02"],
          ["news_ai_cross_batch","CURRENT_65_KIND:NewsToIdea;producer=news-ai-same-tick","BatchOnlyNoNewNotificationValidRevisionRemainsDistinct","CountReductionTargetOrAssessmentHashMeansContentRevision","dedup,shadow,crash","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F03"],
          ["paper_sell_254_2026-09-01","CURRENT_65_KIND:PaperSell","EachFillIntentTracePartialFailureRecoveryNoNewOrderSegmentLatency","254MeansDuplicateOrFileLatencyMeansAcceptedLatency","unit,failure,crash,dedup","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F04"],
          ["attribution_g5b_sink_fail","CURRENT_65_KIND:AttributionDaily+G5bAttribution","SavedResultsReuseNoLLMRecomputeNoEarlyCursor","AnalysisSavedMeansDelivered","failure,crash,shadow","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F05"],
          ["r03_blocked_input","CURRENT_65_KIND:IndustryChain;ReviewTask=R03","FixedContractGapVisibleNoPollingNoNewProducer","BlameUserSnapshotOrEmptyAsNoData","unit,failure","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F07"],
          ["r08_retryability","CURRENT_65_KIND:EventCalendar;ReviewTask=R08","NonretryableEvidencePreservedUntilCapabilityRecovery","StringMeansRetryableOrDropCFFEXRequirement","unit,failure","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F08"],
          ["no_data_disabled_uncertain","CURRENT_65_KIND_SCOPE:AllApplicableUnits","SeparateScheduleNotificationManualCounts","EmptyMeansNoDataOrDisabledMeansAcceptedOrBlindResend","unit,failure,dedup","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§9"],
          ["cross_db_conflict_rollback","CURRENT_65_KIND_SCOPE:AllApplicableUnits","CASConflictResolutionRequiredNewGenerationPreserveAccepted","OverwriteConflictOrUndoExternalAccepted","crash,rollback,dedup","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§9"],
        ],
        error: 'rfc_acceptance_samples_invalid'
      },
      "非基线回放样本（PROPOSED）" => {
        header: ["样本","状态","允许证明","禁止结论","门禁","来源路径","章节定位","依据"],
        rows: [
          ["paper_buy_29_2026-09-04","NON_BASELINE_REPLAY_ONLY","DesignReplayFilledVersusNotFilled","NoCatalogUnitNoBaselineCapabilityNoProducerActivationNoWaveChange","unit,crash,dedup","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F04"],
          ["watchdog_nonbaseline","NON_BASELINE_REPLAY_ONLY","DesignReplayLateStartupSlowReviewMissingRegistrationAlertFailure","NoCatalogUnitNoBaselineCapabilityNoProducerActivationNoWaveChange","unit,failure,crash","docs/push-system/comprehensive-reanalysis-2026-09-05.md","§F06"],
        ],
        error: 'rfc_nonbaseline_samples_invalid'
      },
      "故障环境与验收边界（PROPOSED）" => {
        header: ["环境","许可","禁止行为","依据"],
        rows: [
          ["Test","RejectionUncertainCrashReplayRollbackManualResolution","ProductionNamespaceAccess"],
          ["Production","ApprovedNormalTypedReceiptAndSameDecisionIdempotentReplay","DisconnectKillDatabaseOrderOrManufactureFault"],
        ],
        error: 'rfc_fault_environment_invalid'
      },
    }.freeze
    # 合法引用仍可能指向错误 Unit；样本绑定独立校验，不能只验证 ID 存在。
    # 样本来源路径/章节闭集由 ROLLOUT_CONTRACTS 固定；RfcInputs 同时校验该来源存在与 SHA。
    SAMPLE_BINDINGS = {
      'historical_backfill_2026-08-31' => [['unit', 'MU-review-r07'], ['unit', 'MU-review-r11']],
      'n02_receipt_time' => [['unit', 'MU-news-flash-aggregate']],
      'g5b_test_namespace' => [['unit', 'MU-g5b-attribution']],
      'news_ai_cross_batch' => [['producer', 'news-ai-same-tick'], ['unit', 'MU-news-ai']],
      'paper_sell_254_2026-09-01' => [['unit', 'MU-paper-sell']],
      'attribution_g5b_sink_fail' => [['unit', 'MU-attribution-daily'], ['unit', 'MU-g5b-attribution']],
      'r03_blocked_input' => [['unit', 'MU-review-r03-auto'], ['unit', 'MU-review-r03-manual']],
      'r08_retryability' => [['unit', 'MU-review-r08']],
      'no_data_disabled_uncertain' => [],
      'cross_db_conflict_rollback' => [],
      'paper_buy_29_2026-09-04' => [],
      'watchdog_nonbaseline' => []
    }.freeze
    CANONICAL_EXCEPTIONS = {
      'ScheduleOccurrence' => {'schedule_occurrence_id' => '派生且排除自身'},
      'OperationalReadinessSnapshot' => {'snapshot_id' => '派生且排除自身'},
      'PreparedFacts' => {'canonical_facts' => '外部原始字节', 'facts_sha256' => '派生且排除自身'},
      'SemanticProjection' => {'canonical_bytes' => '派生且排除自身', 'sha256' => '派生且排除自身'},
      'PreparedPush' => {'rendered_bytes' => '外部原始字节'},
      'VerifiedTerminalRef' => {'verified_at' => '派生且排除自身', 'binding_sha256' => '派生且排除自身'}
    }.freeze
    FIELD_RULE_REFERENCES = {
      ['ScheduleOccurrence', 'schedule_occurrence_id'] => ['ScheduleIdentity::v1', 'rfc_schedule_identity_invalid'],
      ['ScheduleOccurrence', 'version'] => ['ScheduleVersionRule::v1', 'rfc_schedule_version_invalid'],
      ['ScheduleOccurrenceTransitionRequest', 'expected_version'] => ['ScheduleVersionRule::v1', 'rfc_schedule_version_invalid'],
      ['PreparedPush', 'intent_id'] => ['IdentityRule::PreparedPushIntent', 'rfc_identity_contract_invalid'],
      ['VerifiedTerminalRef', 'binding_sha256'] => ['IdentityRule::TerminalBinding', 'rfc_identity_contract_invalid'],
      ['CompletionPolicy', 'already_terminal_policy'] => ['CompletionRule::AlreadyTerminal', 'rfc_completion_binding_invalid']
    }.freeze
    module_function

    def validate(root, strict: false)
      errors = content_errors(root)
      if strict && File.directory?(root)
        root = File.realpath(root)
        errors.concat(%w[rfc_status_provisional wbs_status_provisional])
        errors << 'rfc_html_missing' unless release_document(root, 'docs/push-system/push-system-implementation-rfc.html')
        errors << 'ci_rfc_gate_missing' unless ci_rfc_gate?(root)
      end
      errors.uniq
    end

    def release_document(root, path)
      read_document(root, path)
    rescue Invalid, SystemCallError, ArgumentError
      nil
    end

    def ci_rfc_gate?(root)
      bytes = release_document(root, '.github/workflows/ci.yml')
      return false unless bytes
      workflow = YAML.safe_load(bytes)
      return false unless workflow.is_a?(Hash) && workflow['jobs'].is_a?(Hash)
      # v1 不解析继承 defaults：任何 workflow/job defaults 均需另行扩展合同。
      # 这里只证明保守的本地执行形状，不等于远端 Actions 已运行或通过。
      return false if workflow.key?('defaults') || workflow.key?('env')
      workflow['jobs'].values.any? do |job|
        ci_gate_job?(job) && job['steps'].any? do |step|
          ci_gate_step?(step) && step['run'].is_a?(String) &&
            step['run'].strip == 'ruby scripts/architecture-docs/check.rb --check'
        end
      end
    rescue Psych::Exception, ArgumentError
      false
    end

    def ci_gate_job?(job)
      job.is_a?(Hash) && (job.keys - %w[name runs-on if continue-on-error steps]).empty? &&
        job['runs-on'] == 'ubuntu-latest' && job['steps'].is_a?(Array) && ci_gate_condition?(job)
    end

    def ci_gate_step?(step)
      step.is_a?(Hash) && (step.keys - %w[name id run if continue-on-error shell]).empty? &&
        (!step.key?('shell') || %w[bash sh].include?(step['shell'])) && ci_gate_condition?(step)
    end

    def ci_gate_condition?(scope)
      scope.is_a?(Hash) &&
        (!scope.key?('if') || scope['if'].equal?(true)) &&
        (!scope.key?('continue-on-error') || scope['continue-on-error'].equal?(false))
    end

    def content_errors(root)
      root = File.expand_path(root)
      return ['rfc_root_missing'] unless File.exist?(root)
      return ['rfc_root_invalid'] unless File.directory?(root)
      root = File.realpath(root)
      text = read_document(root, RFC_PATH).force_encoding(Encoding::UTF_8)
      return ['rfc_encoding_invalid'] unless text.valid_encoding?
      blocks = text.scan(/```json\n(.*?)\n```/m)
      return ['rfc_metadata_invalid'] unless blocks.length == 1
      metadata = JSON.parse(blocks.first.first)
      errors = metadata_errors(metadata)
      documents = {}
      DEPENDENCIES.each do |key, pair|
        path, sha = pair
        bytes = read_document(root, 'docs/push-system/' + path)
        errors << "rfc_dependency_sha_mismatch path=#{path}" unless Digest::SHA256.hexdigest(bytes) == sha
        documents[key] = bytes
      end
      return errors unless errors.empty?
      errors.concat(RfcInputs.validate(root))
      catalog = JSON.parse(documents['catalog_sha256'])
      evidence = JSON.parse(documents['evidence_manifest_sha256'])
      actual = {'kinds' => catalog['kinds'].length, 'producers' => catalog['producers'].length,
                'units' => catalog['migration_units'].length, 'evidence' => evidence['evidence'].length}
      errors << 'rfc_catalog_counts_invalid' unless actual.all? { |key, value| COUNTS[key] == value }
      statuses = catalog['kinds'].group_by { |kind| kind['status'] }.transform_values(&:length)
      errors << 'rfc_catalog_statuses_invalid' unless statuses == STATUSES
      errors.concat(contract_errors(text, catalog, evidence, documents['decisions_sha256']))
      errors.concat(sql_errors(root, text))
      errors.concat(Wbs.validate(root))
      errors.uniq
    rescue Invalid => error
      [error.message]
    rescue JSON::ParserError
      ['rfc_json_invalid']
    rescue SystemCallError
      ['rfc_io_error']
    rescue ArgumentError
      ['rfc_path_invalid']
    end

    def contract_errors(text, catalog, evidence, decisions)
      errors = []
      errors << 'rfc_placeholder_forbidden' if text.match?(/\b(?:TBD|TODO)\b|待补|待定/i)
      sections = {}
      text.scan(/^## ([^\n]+)\n(.*?)(?=^## |\z)/m).each do |name, body|
        errors << "rfc_section_duplicate name=#{name}" if sections.key?(name)
        sections[name] = body
      end
      required = (REQUIRED_SECTIONS + SEMANTIC_CONTRACTS.keys + PERSISTENCE_CONTRACTS.keys + ROLLOUT_CONTRACTS.keys +
                  ['裁决追踪（PROPOSED）', '业务持久化范围与 SQL 字节合同（PROPOSED）', '最终化事务与恢复边界（PROPOSED）', '规范 DDL 原始嵌入（PROPOSED）'] +
                  TYPE_FIELDS.keys.map { |name| "类型：#{name}（PROPOSED）" }).uniq
      required.each do |name|
        errors << "rfc_section_missing name=#{name}" unless sections.key?(name)
      end
      ids = {
        'Q' => decisions.scan(/^\| (\d+) \|/).flatten,
        'unit' => catalog['migration_units'].map { |entry| entry['id'] },
        'producer' => catalog['producers'].map { |entry| entry['id'] },
        'evidence' => evidence['evidence'].map { |entry| entry['id'] },
        'gate' => ROLLOUT_CONTRACTS.fetch('通用晋级门禁（PROPOSED）')[:rows].map(&:first),
        'milestone' => ROLLOUT_CONTRACTS.fetch('运行里程碑（PROPOSED）')[:rows].map(&:first),
        'acceptance' => SAMPLE_BINDINGS.keys + ROLLOUT_CONTRACTS.fetch('外部兼容（PROPOSED）')[:rows].map(&:first) + ['not_delivered'],
        'publication' => ['ImplementationReady']
      }
      references(text).each do |type, id|
        unless ids.key?(type) && ids[type].include?(id)
          errors << "rfc_reference_invalid type=#{type} id=#{id}"
        end
      end
      required.reject { |name| name == '元数据' }.each do |name|
        body = sections[name]
        errors << "rfc_section_evidence_missing name=#{name}" if body && references(body).empty?
      end
      TYPE_FIELDS.each do |name, fields|
        body = sections["类型：#{name}（PROPOSED）"]
        next unless body
        unless body.match?(/创建者：.+。消费者：.+。/)
          errors << "rfc_type_lifecycle_missing type=#{name}"
        end
        rows = table(body, %w[字段 类型 不变量 规范化], name, errors)
        errors << "rfc_type_fields_invalid type=#{name}" unless rows.map(&:first).sort == fields.keys.sort
        rows.each do |row|
          errors << "rfc_field_type_invalid type=#{name} field=#{row[0]}" unless fields[row[0]] == row[1]
          expected_mode = CANONICAL_EXCEPTIONS.fetch(name, {}).fetch(row[0], '纳入')
          unless row[3] == expected_mode
            errors << "rfc_canonical_rule_invalid type=#{name} field=#{row[0]}"
          end
          rule = FIELD_RULE_REFERENCES[[name, row[0]]]
          errors << "#{rule[1]} type=#{name} field=#{row[0]}" if rule && row[2] != rule[0]
        end
      end
      errors.concat(outcome_errors(sections))
      errors.concat(mapping_errors(sections, catalog))
      errors.concat(state_reason_errors(sections))
      errors.concat(semantic_contract_errors(sections))
      errors.concat(trace_errors(sections, decisions, required, ids))
      errors
    end

    # 选择来自逐字节冻结的 grill，而非另一份手写选择/摘要真相。
    # 约束摘要由人工评审；机器只约束结构、选择、真实规范落点和可解析引用。
    def trace_errors(sections, decisions, locators, ids)
      errors = []
      rows = section_table(sections, '裁决追踪（PROPOSED）',
                           %w[Q 冻结选择 约束摘要 规范落点 证据或验收引用], errors)
      expected = (1..55).map(&:to_s)
      errors << 'rfc_trace_coverage_invalid' unless rows.map(&:first).sort == expected.sort
      choices = decisions.scan(/^\| (\d+) \| ([ABC]) \|/).select { |pair| expected.include?(pair.first) }.to_h
      allowed_loci = locators - ['裁决追踪（PROPOSED）', '元数据', '规范 DDL 原始嵌入（PROPOSED）']
      rows.each do |row|
        errors << "rfc_trace_choice_invalid q=#{row[0]}" unless choices[row[0]] == row[1]
        unless allowed_loci.include?(row[3]) && sections.key?(row[3])
          errors << "rfc_trace_locus_invalid q=#{row[0]}"
        end
        refs = references(row[4])
        valid_refs = !refs.empty? && refs.all? do |type, id|
          type != 'Q' && ids.key?(type) && ids[type].include?(id)
        end
        residue = row[4].gsub(/\[([A-Za-z]+):([^\]\n]+)\]/, '').strip
        errors << "rfc_trace_refs_invalid q=#{row[0]}" unless valid_refs && residue.empty?
      end
      errors
    end

    def semantic_contract_errors(sections)
      errors = []
      SEMANTIC_CONTRACTS.merge(PERSISTENCE_CONTRACTS).merge(ROLLOUT_CONTRACTS).each do |name, profile|
        rows = section_table(sections, name, profile[:header], errors)
        errors << profile[:error] unless rows.map { |row| row[0...-1] }.sort == profile[:rows].sort
        if ['业务验收样本（PROPOSED）', '非基线回放样本（PROPOSED）'].include?(name)
          rows.each do |row|
            bindings = references(row.last).select { |type, _| %w[unit producer].include?(type) }
            unless bindings.sort == SAMPLE_BINDINGS.fetch(row.first, []).sort
              errors << "rfc_sample_bindings_invalid sample=#{row.first}"
            end
          end
        end
      end
      errors
    end

    def outcome_errors(sections)
      errors = []
      jobs = section_table(sections, '类型：JobDecision（PROPOSED）',
                           %w[分支 载荷 允许输入 业务提案 禁止行为 依据], errors)
      errors << 'rfc_job_variants_invalid' unless jobs.map(&:first).sort == JOB_PAYLOADS.keys.sort
      jobs.each do |row|
        errors << "rfc_job_payload_invalid variant=#{row[0]}" unless JOB_PAYLOADS[row[0]] == row[1]
      end
      delivery = section_table(sections, '类型：DeliveryResult（PROPOSED）',
                               %w[分支 载荷 权威类别 权威完成推进 条件 依据], errors)
      errors << 'rfc_delivery_variants_invalid' unless delivery.map(&:first).sort == DELIVERY.keys.sort
      delivery.each do |row|
        errors << "rfc_delivery_authority_invalid variant=#{row[0]}" unless DELIVERY[row[0]] == row[1, 3]
        if row[0] == 'AlreadyTerminal' && row[4] != 'CompletionRule::AlreadyTerminal'
          errors << 'rfc_completion_binding_invalid'
        end
      end
      completion = section_table(sections, '业务完成分支（PROPOSED）',
                                 %w[输入 允许策略 时段提案 游标提案 禁止行为 依据], errors)
      expected = JOB_PAYLOADS.keys + DELIVERY.keys
      errors << 'rfc_completion_branches_invalid' unless completion.map(&:first).sort == expected.sort
      completion.each do |row|
        unless %w[TransportAccepted AlreadyTerminal].include?(row[0]) || row[3] == 'None'
          errors << "rfc_completion_authority_invalid input=#{row[0]}"
        end
        if row[0] == 'AlreadyTerminal'
          expected_rule = ['CompletionRule::AlreadyTerminal', 'CompletionRule::AlreadyTerminal.schedule',
                           'CompletionRule::AlreadyTerminal.cursor', 'CompletionRule::AlreadyTerminal.forbidden']
          errors << 'rfc_completion_binding_invalid' unless row[1, 4] == expected_rule
        end
      end
      errors
    end

    def mapping_errors(sections, catalog)
      errors = []
      mapped = section_table(sections, 'monitor 到 durable 的映射（CURRENT）',
                             %w[monitor类型 durable类型 子类型 依据], errors).map { |row| row[0, 3] }
      errors << 'rfc_mapping_invalid' unless mapped.sort == MAPPINGS.sort
      all_kinds = catalog['kinds'].map { |kind| kind['kind'] }
      errors << 'rfc_mapping_catalog_invalid' unless (MAPPINGS.map(&:first) - all_kinds).empty?
      unmapped = section_table(sections, '未直接映射的 monitor 类型（CURRENT 状态；PROPOSED 处置）',
                               %w[monitor类型 状态 处置 依据], errors).map { |row| row[0, 3] }
      actions = {'ACTIVE' => 'adapt_or_conform', 'INACTIVE' => 'keep_inactive',
                 'STARVED' => 'retain_starved', 'OPT-IN' => 'retain_opt_in'}
      expected = catalog['kinds'].reject { |kind| MAPPINGS.any? { |row| row[0] == kind['kind'] } }.map do |kind|
        [kind['kind'], kind['status'], actions.fetch(kind['status'])]
      end
      errors << 'rfc_unmapped_invalid' unless unmapped.sort == expected.sort && expected.length == COUNTS['unmapped']
      errors
    end

    def state_reason_errors(sections)
      errors = []
      states = section_table(sections, 'durable 状态（CURRENT）与应用投影（PROPOSED）',
                             %w[状态 应用结果 传输处置终态 自动发送重试 业务最终化 依据], errors)
      errors << 'rfc_states_invalid' unless states.map(&:first).sort == STATE_PROJECTIONS.keys.sort
      states.each do |row|
        errors << "rfc_state_projection_invalid state=#{row[0]}" unless STATE_PROJECTIONS[row[0]] == row[1, 4]
      end
      reasons = section_table(sections, '类型：ReasonCode（PROPOSED）', %w[代码 条件 处理 依据], errors).map(&:first)
      errors << 'rfc_reason_duplicate' unless reasons.uniq == reasons
      namespaces = REASONS.map { |code| code.split('.').first }.uniq
      reasons.each do |code|
        unless code.match?(/\A[a-z]+\.[a-z][a-z0-9_]*\z/) && namespaces.include?(code.split('.').first)
          errors << "rfc_reason_namespace_invalid code=#{code}"
        end
      end
      errors << 'rfc_reason_coverage_invalid' unless (REASONS - reasons).empty?
      PERSISTENCE_CONTRACTS.merge(ROLLOUT_CONTRACTS).each_value do |profile|
        column = profile[:header].index('ReasonCode')
        next unless column
        profile[:rows].each do |row|
          row[column].split('/').each do |code|
            errors << "rfc_persistence_reason_missing code=#{code}" unless reasons.include?(code)
          end
        end
      end
      errors
    end

    def section_table(sections, name, header, errors)
      return [] unless sections[name]
      table(sections[name], header, name, errors)
    end

    def references(text)
      text.scan(/\[([A-Za-z]+):([^\]\n]+)\]/)
    end

    def table(body, header, name, errors)
      rows = body.lines.select { |line| line.start_with?('|') }.map do |line|
        line.strip.split('|', -1)[1...-1].map(&:strip)
      end
      unless rows.length >= 3 && rows[0] == header && rows[1].length == header.length &&
             rows[1].all? { |value| value.match?(/\A:?-{3,}:?\z/) }
        errors << "rfc_table_invalid name=#{name}"
        return []
      end
      values = rows.drop(2)
      values.select do |row|
        if row.length != header.length || row.any?(&:empty?)
          errors << "rfc_table_invalid name=#{name}"
          false
        else
          if header.last == '依据' && references(row.last).empty?
            errors << "rfc_row_evidence_missing name=#{name} row=#{row.first}"
          end
          true
        end
      end
    end

    # 只读字节门禁；不执行文档中的 SQL、SQLite 点命令或外部文件引用。
    # 真实 DDL 行为由临时 SQLite 测试承担，不能用两份自洽文本代替约束验证。
    def sql_errors(root, text)
      sql = read_document(root, SQL_PATH)
      begin_marker = '<!-- RFC-SQL-BEGIN -->'
      end_marker = '<!-- RFC-SQL-END -->'
      unless text.scan('RFC-SQL-BEGIN').length == 1 && text.scan('RFC-SQL-END').length == 1 &&
             text.scan(/^```sql[^\n]*$/i).length == 1
        return ['rfc_sql_region_invalid']
      end
      region = text.match(/#{Regexp.escape(begin_marker)}(.*?)#{Regexp.escape(end_marker)}/m)
      body = region && region[1].match(/\A\n```sql\n(.*)```\n\z/m)
      return ['rfc_sql_region_invalid'] unless body && !body[1].include?('```')
      errors = []
      errors << 'rfc_sql_bytes_mismatch' unless body[1].b == sql.b
      outside = text.sub(region[0], '')
      hashes = outside.scan(/^SQL SHA-256：([a-f0-9]{64})$/).flatten
      unless hashes.length == 1 && outside.scan('SQL SHA-256：').length == 1 && hashes[0] == Digest::SHA256.hexdigest(sql)
        errors << 'rfc_sql_hash_invalid'
      end
      errors
    end

    def metadata_errors(metadata)
      return ['rfc_metadata_invalid'] unless metadata.is_a?(Hash)
      errors = []
      fields = %w[schema_version status version source_baseline counts status_counts] + DEPENDENCIES.keys
      errors << 'rfc_metadata_fields_invalid' unless metadata.keys.sort == fields.sort
      errors << 'rfc_schema_invalid' unless metadata['schema_version'].eql?(1)
      errors << 'rfc_status_invalid' unless metadata['status'] == 'PROVISIONAL'
      errors << 'rfc_version_invalid' unless metadata['version'] == 'push-system-rfc-v1'
      errors << 'rfc_baseline_invalid' unless metadata['source_baseline'] == BASELINE
      DEPENDENCIES.each do |key, pair|
        errors << "rfc_metadata_hash_invalid field=#{key}" unless metadata[key] == pair.last
      end
      errors << 'rfc_counts_invalid' unless metadata['counts'].eql?(COUNTS)
      errors << 'rfc_status_counts_invalid' unless metadata['status_counts'].eql?(STATUSES)
      errors
    end

    def read_document(root, path)
      parts = path.split('/', -1)
      if path.include?("\0") || Pathname.new(path).absolute? || parts.any? { |part| ['', '.', '..'].include?(part) }
        raise Invalid, "rfc_path_invalid path=#{path}"
      end
      current = root
      parts.each do |part|
        current = File.join(current, part)
        raise Invalid, "rfc_path_invalid path=#{path}" if File.lstat(current).symlink?
      end
      real = File.realpath(current)
      raise Invalid, "rfc_path_invalid path=#{path}" unless real.start_with?(root + File::SEPARATOR)
      raise Invalid, "rfc_not_regular path=#{path}" unless File.file?(real)
      File.binread(real)
    rescue Errno::ENOENT
      raise Invalid, "rfc_document_missing path=#{path}"
    end
  end
end
