# frozen_string_literal: true

require 'json'
require 'digest'
require 'pathname'
require_relative 'rfc_inputs'

module ArchitectureDocs
  module RfcSpec
    RFC_PATH = 'docs/push-system/push-system-implementation-rfc.md'
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
    REQUIRED_SECTIONS = ['Metadata', 'Scope and authority', 'Navigation and canonical rules (PROPOSED)',
      'Type: JobDecision (PROPOSED)', 'Type: DeliveryResult (PROPOSED)', 'Completion branches (PROPOSED)',
      'Monitor to durable mapping (CURRENT)', 'Unmapped monitor kinds (CURRENT status; PROPOSED treatment)',
      'Durable states (CURRENT); application projection (PROPOSED)', 'Type: ReasonCode (PROPOSED)',
      'Adapter conformance (PROPOSED)', 'Task2 verification boundary'].freeze
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
      ["DailyReport","DailyReport","requested FactorIC/SectorTier/CapitalVerify or None"]
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
      'Reserved' => %w[Blocked no lease_fenced_first_attempt no],
      'AttemptInFlight' => %w[Blocked no never_until_reconciled no],
      'AcceptedAuditPending' => %w[Blocked no never after_authority_sealed],
      'AcceptedTaskTransitionPending' => %w[Blocked no never after_authority_sealed],
      'Delivered' => %w[TransportAccepted/AlreadyTerminal yes never accepted_binding_only],
      'RejectedAuditPending' => %w[Blocked no never_until_reconciled no],
      'RejectedTaskTransitionPending' => %w[Blocked no never_until_reconciled no],
      'RejectedDurable' => %w[TransportRejected/AlreadyTerminal yes explicit_authorization_only rejection_proposal_no_cursor],
      'UncertainAuditPending' => %w[Blocked no never no],
      'UncertainTaskTransitionPending' => %w[Blocked no never no],
      'UncertainManualReview' => %w[TransportUncertain/AlreadyTerminal yes never quarantine_no_cursor],
      'ManualRejectedAuditPending' => %w[Blocked no never no],
      'ManualRejectedTaskTransitionPending' => %w[Blocked no never no],
      'ManualResolvedRejected' => %w[AlreadyTerminal yes never manual_not_delivered_no_cursor]
    }.freeze
    REASONS = %w[
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
      operator.unauthorized
      operator.evidence_invalid
      operator.resolution_conflict
    ].freeze
    module_function

    def validate(root, strict: false)
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
      errors << 'rfc_status_provisional' if strict
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
      required = REQUIRED_SECTIONS + TYPE_FIELDS.keys.map { |name| "Type: #{name} (PROPOSED)" }
      required.each do |name|
        errors << "rfc_section_missing name=#{name}" unless sections.key?(name)
      end
      ids = {
        'Q' => decisions.scan(/^\| (\d+) \|/).flatten,
        'unit' => catalog['migration_units'].map { |entry| entry['id'] },
        'producer' => catalog['producers'].map { |entry| entry['id'] },
        'evidence' => evidence['evidence'].map { |entry| entry['id'] }
      }
      references(text).each do |type, id|
        unless ids.key?(type) && ids[type].include?(id)
          errors << "rfc_reference_invalid type=#{type} id=#{id}"
        end
      end
      required.reject { |name| name == 'Metadata' }.each do |name|
        body = sections[name]
        errors << "rfc_section_evidence_missing name=#{name}" if body && references(body).empty?
      end
      TYPE_FIELDS.each do |name, fields|
        body = sections["Type: #{name} (PROPOSED)"]
        next unless body
        unless body.match?(/Creator: .+\. Consumer: .+\./)
          errors << "rfc_type_lifecycle_missing type=#{name}"
        end
        rows = table(body, %w[field type invariant canonical], name, errors)
        errors << "rfc_type_fields_invalid type=#{name}" unless rows.map(&:first).sort == fields.keys.sort
        rows.each do |row|
          errors << "rfc_field_type_invalid type=#{name} field=#{row[0]}" unless fields[row[0]] == row[1]
          unless ['include', 'external exact bytes', 'derived self-excluded'].include?(row[3])
            errors << "rfc_canonical_rule_invalid type=#{name} field=#{row[0]}"
          end
        end
      end
      errors.concat(outcome_errors(sections))
      errors.concat(mapping_errors(sections, catalog))
      errors.concat(state_reason_errors(sections))
      errors
    end

    def outcome_errors(sections)
      errors = []
      jobs = section_table(sections, 'Type: JobDecision (PROPOSED)',
                           %w[variant payload allowed_input proposal forbidden refs], errors)
      errors << 'rfc_job_variants_invalid' unless jobs.map(&:first).sort == JOB_PAYLOADS.keys.sort
      jobs.each do |row|
        errors << "rfc_job_payload_invalid variant=#{row[0]}" unless JOB_PAYLOADS[row[0]] == row[1]
      end
      delivery = section_table(sections, 'Type: DeliveryResult (PROPOSED)',
                               %w[variant payload authority authoritative_completion condition refs], errors)
      errors << 'rfc_delivery_variants_invalid' unless delivery.map(&:first).sort == DELIVERY.keys.sort
      delivery.each do |row|
        errors << "rfc_delivery_authority_invalid variant=#{row[0]}" unless DELIVERY[row[0]] == row[1, 3]
      end
      completion = section_table(sections, 'Completion branches (PROPOSED)',
                                 %w[input allowed_policy schedule_proposal cursor_proposal forbidden refs], errors)
      expected = JOB_PAYLOADS.keys + DELIVERY.keys
      errors << 'rfc_completion_branches_invalid' unless completion.map(&:first).sort == expected.sort
      completion.each do |row|
        unless %w[TransportAccepted AlreadyTerminal].include?(row[0]) || row[3] == 'None'
          errors << "rfc_completion_authority_invalid input=#{row[0]}"
        end
      end
      errors
    end

    def mapping_errors(sections, catalog)
      errors = []
      mapped = section_table(sections, 'Monitor to durable mapping (CURRENT)',
                             %w[monitor_kind durable_kind sub_kind refs], errors).map { |row| row[0, 3] }
      errors << 'rfc_mapping_invalid' unless mapped.sort == MAPPINGS.sort
      all_kinds = catalog['kinds'].map { |kind| kind['kind'] }
      errors << 'rfc_mapping_catalog_invalid' unless (MAPPINGS.map(&:first) - all_kinds).empty?
      unmapped = section_table(sections, 'Unmapped monitor kinds (CURRENT status; PROPOSED treatment)',
                               %w[monitor_kind status treatment refs], errors).map { |row| row[0, 3] }
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
      states = section_table(sections, 'Durable states (CURRENT); application projection (PROPOSED)',
                             %w[state application_result terminal automatic_send_retry business_finalizer refs], errors)
      errors << 'rfc_states_invalid' unless states.map(&:first).sort == STATE_PROJECTIONS.keys.sort
      states.each do |row|
        errors << "rfc_state_projection_invalid state=#{row[0]}" unless STATE_PROJECTIONS[row[0]] == row[1, 4]
      end
      reasons = section_table(sections, 'Type: ReasonCode (PROPOSED)', %w[code condition handling refs], errors).map(&:first)
      errors << 'rfc_reason_duplicate' unless reasons.uniq == reasons
      namespaces = REASONS.map { |code| code.split('.').first }.uniq
      reasons.each do |code|
        unless code.match?(/\A[a-z]+\.[a-z][a-z0-9_]*\z/) && namespaces.include?(code.split('.').first)
          errors << "rfc_reason_namespace_invalid code=#{code}"
        end
      end
      errors << 'rfc_reason_coverage_invalid' unless (REASONS - reasons).empty?
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
          if header.last == 'refs' && references(row.last).empty?
            errors << "rfc_row_evidence_missing name=#{name} row=#{row.first}"
          end
          true
        end
      end
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
