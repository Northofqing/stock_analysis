-- Chain v12: full Macro facts. All v1-v11 artifacts and existing facts stay immutable.
-- Replaces only the eight named Macro INSERT guards, not historical fences.
-- Transaction-final validation additionally requires each terminal result and its
-- query terminal / mandatory audit to be committed as one indivisible group.

CREATE TABLE chain_post_close_macro_query_terminals (
    intent_id TEXT NOT NULL REFERENCES chain_post_close_runs(intent_id),
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK(length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK(length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK(lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK(prior_head_version>=1),
    run_version INTEGER NOT NULL CHECK(run_version=prior_head_version+1),
    recorded_at INTEGER NOT NULL CHECK(recorded_at>=0),
    bytes BLOB NOT NULL CHECK(typeof(bytes)='blob' AND length(bytes)>0),
    byte_length INTEGER NOT NULL CHECK(byte_length=length(bytes)),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    phase TEXT NOT NULL CHECK(phase IN ('Gateway','WebDimension')),
    item_ordinal INTEGER NOT NULL,
    candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal>=1),
    plan_version INTEGER NOT NULL,
    plan_sha256 TEXT NOT NULL CHECK(length(plan_sha256)=64),
    request_plan_version INTEGER,
    request_sha256 TEXT,
    cause_kind TEXT NOT NULL CHECK(cause_kind IN ('DataResult','SharedControlRejected','HistoricalControlRejected','RequestRejected','LocalRouteUnavailable')),
    data_result_version INTEGER,
    control_result_version INTEGER,
    call_state TEXT NOT NULL CHECK(call_state IN ('Returned','NotCalled')),
    native_bytes BLOB NOT NULL CHECK(typeof(native_bytes)='blob' AND length(native_bytes)>0),
    native_length INTEGER NOT NULL CHECK(native_length=length(native_bytes)),
    native_sha256 TEXT NOT NULL CHECK(length(native_sha256)=64 AND native_sha256 NOT GLOB '*[^0-9a-f]*'),
    audit_id INTEGER UNIQUE REFERENCES data_acquisition_audit(id),
    audit_record_hash TEXT,
    audit_capability TEXT,
    audit_provider TEXT,
    audit_request_hash TEXT,
    previous_outcome TEXT,
    current_outcome TEXT,
    PRIMARY KEY(intent_id,phase,item_ordinal,candidate_ordinal),
    UNIQUE(intent_id,run_version),
    FOREIGN KEY(intent_id,plan_version) REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    FOREIGN KEY(intent_id,request_plan_version,request_sha256) REFERENCES chain_post_close_macro_request_plans(intent_id,run_version,request_sha256),
    FOREIGN KEY(intent_id,data_result_version) REFERENCES chain_post_close_macro_attempt_results(intent_id,run_version),
    FOREIGN KEY(intent_id,control_result_version) REFERENCES chain_post_close_macro_control_attempt_results(intent_id,run_version),
    FOREIGN KEY(audit_id) REFERENCES data_acquisition_audit_chain(acquisition_audit_id),
    CHECK((phase='Gateway' AND item_ordinal BETWEEN 1 AND 5 AND candidate_ordinal=1)
       OR (phase='WebDimension' AND item_ordinal BETWEEN 1 AND 6)),
    CHECK((cause_kind='DataResult' AND data_result_version IS NOT NULL AND control_result_version IS NULL AND request_plan_version IS NOT NULL AND request_sha256 IS NOT NULL AND call_state='Returned')
       OR (cause_kind='SharedControlRejected' AND data_result_version IS NULL AND control_result_version IS NOT NULL AND request_plan_version IS NOT NULL AND request_sha256 IS NOT NULL AND call_state='NotCalled' AND phase='Gateway' AND item_ordinal BETWEEN 1 AND 4)
       OR (cause_kind='HistoricalControlRejected' AND data_result_version IS NULL AND control_result_version IS NOT NULL AND request_plan_version IS NOT NULL AND request_sha256 IS NOT NULL AND call_state='NotCalled' AND phase='Gateway' AND item_ordinal BETWEEN 2 AND 4)
       OR (cause_kind='RequestRejected' AND data_result_version IS NULL AND control_result_version IS NULL AND request_plan_version IS NULL AND request_sha256 IS NULL AND call_state='NotCalled' AND phase='WebDimension')
       OR (cause_kind='LocalRouteUnavailable' AND data_result_version IS NULL AND control_result_version IS NULL AND request_plan_version IS NULL AND request_sha256 IS NULL AND call_state='NotCalled' AND phase='Gateway' AND item_ordinal=5)),
    CHECK((phase='Gateway' AND audit_id IS NOT NULL AND length(audit_record_hash)=64 AND length(audit_capability)>0 AND length(audit_provider)>0 AND length(audit_request_hash)=64 AND current_outcome IN ('available','verified_empty','partial','unavailable','invalid_request','audit_failure'))
       OR (phase='WebDimension' AND audit_id IS NULL AND audit_record_hash IS NULL AND audit_capability IS NULL AND audit_provider IS NULL AND audit_request_hash IS NULL AND previous_outcome IS NULL AND current_outcome IS NULL))
);

CREATE TABLE chain_post_close_macro_dimension_terminals (
    intent_id TEXT NOT NULL REFERENCES chain_post_close_runs(intent_id),
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK(length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK(length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK(lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK(prior_head_version>=1),
    run_version INTEGER NOT NULL CHECK(run_version=prior_head_version+1),
    recorded_at INTEGER NOT NULL CHECK(recorded_at>=0),
    bytes BLOB NOT NULL CHECK(typeof(bytes)='blob' AND length(bytes)>0),
    byte_length INTEGER NOT NULL CHECK(byte_length=length(bytes)),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    dimension INTEGER NOT NULL CHECK(dimension BETWEEN 1 AND 6),
    plan_version INTEGER NOT NULL,
    plan_sha256 TEXT NOT NULL CHECK(length(plan_sha256)=64),
    outcome TEXT NOT NULL CHECK(outcome IN ('SelectedResearchOnly','Exhausted','NoEligibleProviders')),
    last_query_terminal_version INTEGER,
    selected_candidate_ordinal INTEGER,
    pace_due INTEGER NOT NULL CHECK(pace_due-recorded_at=300000),
    PRIMARY KEY(intent_id,dimension),
    UNIQUE(intent_id,run_version),
    FOREIGN KEY(intent_id,plan_version) REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    FOREIGN KEY(intent_id,last_query_terminal_version) REFERENCES chain_post_close_macro_query_terminals(intent_id,run_version),
    CHECK((outcome='SelectedResearchOnly' AND last_query_terminal_version IS NOT NULL AND selected_candidate_ordinal>=1)
       OR (outcome='Exhausted' AND last_query_terminal_version IS NOT NULL AND selected_candidate_ordinal IS NULL)
       OR (outcome='NoEligibleProviders' AND last_query_terminal_version IS NULL AND selected_candidate_ordinal IS NULL))
);

CREATE TABLE chain_post_close_macro_finalize_begins (
    intent_id TEXT NOT NULL REFERENCES chain_post_close_runs(intent_id),
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK(length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK(length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK(lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK(prior_head_version>=1),
    run_version INTEGER NOT NULL CHECK(run_version=prior_head_version+1),
    recorded_at INTEGER NOT NULL CHECK(recorded_at>=0),
    bytes BLOB NOT NULL CHECK(typeof(bytes)='blob' AND length(bytes)>0),
    byte_length INTEGER NOT NULL CHECK(byte_length=length(bytes)),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    plan_version INTEGER NOT NULL,
    plan_sha256 TEXT NOT NULL CHECK(length(plan_sha256)=64),
    kind TEXT NOT NULL CHECK(kind IN ('Complete','BudgetExpired')),
    started_at INTEGER NOT NULL CHECK(started_at>=0),
    deadline_at INTEGER NOT NULL CHECK(deadline_at-started_at=15000000),
    facts_sha256 TEXT NOT NULL CHECK(length(facts_sha256)=64),
    output_bytes BLOB NOT NULL CHECK(typeof(output_bytes)='blob'),
    output_length INTEGER NOT NULL CHECK(output_length=length(output_bytes)),
    output_sha256 TEXT NOT NULL CHECK(length(output_sha256)=64 AND output_sha256 NOT GLOB '*[^0-9a-f]*'),
    expiry_opened_at INTEGER,
    expiry_elapsed_us INTEGER,
    PRIMARY KEY(intent_id),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,run_version,kind,output_sha256,lease_owner,lease_generation),
    FOREIGN KEY(intent_id,plan_version) REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    CHECK((kind='Complete' AND recorded_at<deadline_at
            AND expiry_opened_at IS NULL AND expiry_elapsed_us IS NULL)
       OR (kind='BudgetExpired' AND output_length=0
            AND ((recorded_at>=deadline_at
                    AND expiry_opened_at IS NULL AND expiry_elapsed_us IS NULL)
                OR (recorded_at<deadline_at
                    AND expiry_opened_at IS NOT NULL AND expiry_elapsed_us IS NOT NULL
                    AND typeof(expiry_opened_at)='integer' AND typeof(expiry_elapsed_us)='integer'
                    AND expiry_opened_at>=started_at AND expiry_opened_at<deadline_at
                    AND expiry_elapsed_us>=deadline_at-expiry_opened_at))))
);

CREATE TABLE chain_post_close_macro_stage_finals (
    intent_id TEXT NOT NULL REFERENCES chain_post_close_runs(intent_id),
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK(length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK(length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK(lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK(prior_head_version>=1),
    run_version INTEGER NOT NULL CHECK(run_version=prior_head_version+1),
    recorded_at INTEGER NOT NULL CHECK(recorded_at>=0),
    bytes BLOB NOT NULL CHECK(typeof(bytes)='blob' AND length(bytes)>0),
    byte_length INTEGER NOT NULL CHECK(byte_length=length(bytes)),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    finalize_begin_version INTEGER NOT NULL,
    plan_version INTEGER NOT NULL,
    plan_sha256 TEXT NOT NULL CHECK(length(plan_sha256)=64),
    kind TEXT NOT NULL CHECK(kind IN ('Complete','BudgetExpired')),
    started_at INTEGER NOT NULL CHECK(started_at>=0),
    deadline_at INTEGER NOT NULL CHECK(deadline_at-started_at=15000000),
    facts_sha256 TEXT NOT NULL CHECK(length(facts_sha256)=64),
    output_bytes BLOB NOT NULL CHECK(typeof(output_bytes)='blob'),
    output_length INTEGER NOT NULL CHECK(output_length=length(output_bytes)),
    output_sha256 TEXT NOT NULL CHECK(length(output_sha256)=64 AND output_sha256 NOT GLOB '*[^0-9a-f]*'),
    PRIMARY KEY(intent_id),
    UNIQUE(intent_id,run_version),
    FOREIGN KEY(intent_id,plan_version) REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    FOREIGN KEY(intent_id,finalize_begin_version,kind,output_sha256,lease_owner,lease_generation)
        REFERENCES chain_post_close_macro_finalize_begins(intent_id,run_version,kind,output_sha256,lease_owner,lease_generation),
    CHECK((kind='Complete' AND recorded_at<deadline_at)
       OR (kind='BudgetExpired' AND output_length=0))
);

DROP TRIGGER chain_post_close_macro_plans_guard;
DROP TRIGGER chain_post_close_macro_request_plans_guard;
DROP TRIGGER chain_post_close_macro_readiness_episode_plans_guard;
DROP TRIGGER chain_post_close_macro_control_attempt_begins_guard;
DROP TRIGGER chain_post_close_macro_control_attempt_results_guard;
DROP TRIGGER chain_post_close_macro_attempt_begins_guard;
DROP TRIGGER chain_post_close_macro_attempt_results_guard;
DROP TRIGGER chain_post_close_macro_source_finals_guard;

CREATE TRIGGER chain_post_close_macro_plans_guard BEFORE INSERT ON chain_post_close_macro_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_dragon_tiger_finals p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.parent_version AND p.final_sha256=NEW.parent_sha256 AND p.run_id=NEW.run_id AND p.run_context_sha256=NEW.run_context_sha256 AND p.input_sha256=NEW.input_sha256 AND p.applied_at<=NEW.started_at AND p.run_version<=NEW.prior_head_version AND p.lease_generation<=NEW.lease_generation AND (p.lease_generation<NEW.lease_generation OR p.lease_owner=NEW.lease_owner) AND p.final_outcome IN ('Available','VerifiedEmpty'))
BEGIN SELECT RAISE(ABORT,'chain v12 macro plans rejected'); END;

CREATE TRIGGER chain_post_close_macro_request_plans_guard BEFORE INSERT ON chain_post_close_macro_request_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.run_version<=NEW.prior_head_version AND p.run_id=NEW.run_id AND p.run_context_sha256=NEW.run_context_sha256 AND p.input_sha256=NEW.input_sha256 AND p.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
OR (NEW.profile='ExternalV1' AND NOT(NEW.phase='Gateway' AND NEW.item_ordinal BETWEEN 1 AND 4))
OR EXISTS(SELECT 1 FROM chain_post_close_macro_query_terminals t WHERE t.intent_id=NEW.intent_id AND t.phase=NEW.phase AND t.item_ordinal=NEW.item_ordinal AND t.candidate_ordinal=NEW.candidate_ordinal)
OR (NEW.phase='WebDimension' AND (
    EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension>=NEW.item_ordinal)
    OR 5<>(SELECT count(*) FROM (
        SELECT item_ordinal FROM chain_post_close_macro_query_terminals WHERE intent_id=NEW.intent_id AND phase='Gateway'
        UNION ALL SELECT item_ordinal FROM chain_post_close_macro_source_finals WHERE intent_id=NEW.intent_id AND phase='Gateway'))
    OR NEW.recorded_at<(SELECT recorded_at+200000 FROM (
        SELECT run_version,recorded_at FROM chain_post_close_macro_query_terminals WHERE intent_id=NEW.intent_id AND phase='Gateway'
        UNION ALL SELECT run_version,recorded_at FROM chain_post_close_macro_source_finals WHERE intent_id=NEW.intent_id AND phase='Gateway') ORDER BY run_version DESC LIMIT 1)
    OR (NEW.item_ordinal>1 AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension=NEW.item_ordinal-1 AND d.pace_due<=NEW.recorded_at))))
BEGIN SELECT RAISE(ABORT,'chain v12 macro request_plans rejected'); END;

CREATE TRIGGER chain_post_close_macro_readiness_episode_plans_guard BEFORE INSERT ON chain_post_close_macro_readiness_episode_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_request_plans q JOIN chain_post_close_macro_plans p ON p.intent_id=q.intent_id AND p.run_version=q.plan_version WHERE q.intent_id=NEW.intent_id AND q.run_version=NEW.request_plan_version AND p.run_version=NEW.plan_version AND q.profile='ExternalV1' AND q.phase=NEW.phase AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=NEW.candidate_ordinal AND q.endpoint=NEW.endpoint AND q.acquisition_authority=NEW.acquisition_authority AND q.run_version<=NEW.prior_head_version AND q.run_id=NEW.run_id AND q.run_context_sha256=NEW.run_context_sha256 AND q.input_sha256=NEW.input_sha256 AND q.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
OR 4<>(SELECT count(*) FROM chain_post_close_macro_request_plans q WHERE q.intent_id=NEW.intent_id AND q.plan_version=NEW.plan_version AND q.phase='Gateway' AND q.item_ordinal BETWEEN 1 AND 4 AND q.candidate_ordinal=1 AND q.profile='ExternalV1' AND q.endpoint=NEW.endpoint AND q.acquisition_authority=NEW.acquisition_authority)
BEGIN SELECT RAISE(ABORT,'chain v12 macro readiness_episode_plans rejected'); END;

CREATE TRIGGER chain_post_close_macro_control_attempt_begins_guard BEFORE INSERT ON chain_post_close_macro_control_attempt_begins
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_readiness_episode_plans e JOIN chain_post_close_macro_plans p ON p.intent_id=e.intent_id AND p.run_version=e.plan_version WHERE e.intent_id=NEW.intent_id AND e.episode_ordinal=NEW.episode_ordinal AND e.run_version=NEW.episode_plan_version AND e.run_version<=NEW.prior_head_version AND e.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
OR (NEW.control_ordinal=2 AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_results h WHERE h.intent_id=NEW.intent_id AND h.episode_ordinal=NEW.episode_ordinal AND h.control_ordinal=1 AND h.kind='Health' AND h.run_version=NEW.health_result_version AND h.outcome='Ready' AND h.run_version<=NEW.prior_head_version AND h.recorded_at<=NEW.recorded_at))
BEGIN SELECT RAISE(ABORT,'chain v12 macro control_attempt_begins rejected'); END;

CREATE TRIGGER chain_post_close_macro_control_attempt_results_guard BEFORE INSERT ON chain_post_close_macro_control_attempt_results
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_begins b JOIN chain_post_close_macro_plans p ON p.intent_id=b.intent_id WHERE b.intent_id=NEW.intent_id AND b.episode_ordinal=NEW.episode_ordinal AND b.control_ordinal=NEW.control_ordinal AND b.kind=NEW.kind AND b.run_version=NEW.begin_version AND b.request_sha256=NEW.request_sha256 AND b.lease_owner=NEW.lease_owner AND b.lease_generation=NEW.lease_generation AND b.run_version<=NEW.prior_head_version AND b.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v12 macro control_attempt_results rejected'); END;

CREATE TRIGGER chain_post_close_macro_attempt_begins_guard BEFORE INSERT ON chain_post_close_macro_attempt_begins
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_request_plans q JOIN chain_post_close_macro_plans p ON p.intent_id=q.intent_id AND p.run_version=q.plan_version WHERE q.intent_id=NEW.intent_id AND q.run_version=NEW.request_plan_version AND q.request_sha256=NEW.request_sha256 AND q.phase=NEW.phase AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=NEW.candidate_ordinal AND q.run_version<=NEW.prior_head_version AND q.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
OR (NEW.attempt_ordinal=1 AND NEW.previous_result_version IS NOT NULL)
OR (NEW.attempt_ordinal>1 AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_results prior WHERE prior.intent_id=NEW.intent_id AND prior.phase=NEW.phase AND prior.item_ordinal=NEW.item_ordinal AND prior.candidate_ordinal=NEW.candidate_ordinal AND prior.attempt_ordinal=NEW.attempt_ordinal-1 AND prior.run_version=NEW.previous_result_version AND prior.continuation='Retry' AND prior.retry_not_before<=NEW.recorded_at AND prior.run_version<=NEW.prior_head_version))
OR NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_request_plans q
    WHERE q.intent_id=NEW.intent_id
      AND q.run_version=NEW.request_plan_version
      AND q.phase=NEW.phase
      AND q.item_ordinal=NEW.item_ordinal
      AND q.candidate_ordinal=NEW.candidate_ordinal
      AND ((q.profile='LocalBridgeV1' AND q.acquisition_authority IS NULL
            AND NEW.readiness_result_version IS NULL)
        OR (q.profile='ExternalV1' AND NEW.readiness_result_version IS NOT NULL
            AND EXISTS(
                SELECT 1 FROM chain_post_close_macro_readiness_episode_plans e
                JOIN chain_post_close_macro_control_attempt_results c
                  ON c.intent_id=e.intent_id AND c.episode_ordinal=e.episode_ordinal
                 AND c.control_ordinal=2 AND c.kind='Capabilities'
                 AND c.outcome='Ready' AND c.run_version=NEW.readiness_result_version
                JOIN chain_post_close_macro_control_attempt_begins b
                  ON b.intent_id=c.intent_id AND b.episode_ordinal=c.episode_ordinal
                 AND b.control_ordinal=c.control_ordinal AND b.run_version=c.begin_version
                JOIN chain_post_close_macro_control_attempt_results h
                  ON h.intent_id=c.intent_id AND h.episode_ordinal=c.episode_ordinal
                 AND h.control_ordinal=1 AND h.kind='Health' AND h.outcome='Ready'
                 AND h.run_version=b.health_result_version
                WHERE e.intent_id=NEW.intent_id
                  AND e.plan_version=q.plan_version
                  AND e.required_operation=17
                  AND e.endpoint=q.endpoint
                  AND e.acquisition_authority=q.acquisition_authority
                  AND h.recorded_at<=c.recorded_at
                  AND c.recorded_at<=NEW.recorded_at
                  AND c.run_version<=NEW.prior_head_version
            )))
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_source_finals f WHERE f.intent_id=NEW.intent_id AND f.phase=NEW.phase AND f.item_ordinal=NEW.item_ordinal)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_query_terminals t WHERE t.intent_id=NEW.intent_id AND t.phase=NEW.phase AND t.item_ordinal=NEW.item_ordinal AND t.candidate_ordinal=NEW.candidate_ordinal)
OR (NEW.phase='WebDimension' AND EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension>=NEW.item_ordinal))
BEGIN SELECT RAISE(ABORT,'chain v12 macro attempt_begins rejected'); END;

CREATE TRIGGER chain_post_close_macro_attempt_results_guard BEFORE INSERT ON chain_post_close_macro_attempt_results
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_begins b JOIN chain_post_close_macro_plans p ON p.intent_id=b.intent_id WHERE b.intent_id=NEW.intent_id AND b.phase=NEW.phase AND b.item_ordinal=NEW.item_ordinal AND b.candidate_ordinal=NEW.candidate_ordinal AND b.attempt_ordinal=NEW.attempt_ordinal AND b.run_version=NEW.begin_version AND b.request_sha256=NEW.request_sha256 AND b.lease_owner=NEW.lease_owner AND b.lease_generation=NEW.lease_generation AND b.run_version<=NEW.prior_head_version AND b.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v12 macro attempt_results rejected'); END;

CREATE TRIGGER chain_post_close_macro_source_finals_guard BEFORE INSERT ON chain_post_close_macro_source_finals
BEGIN SELECT RAISE(ABORT,'chain v12 legacy source final is read-only'); END;

CREATE TRIGGER chain_post_close_macro_query_terminals_guard BEFORE INSERT ON chain_post_close_macro_query_terminals
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.sha256=NEW.plan_sha256 AND p.run_version<=NEW.prior_head_version AND p.recorded_at<=NEW.recorded_at)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND NEW.recorded_at<p.deadline_at)
OR (NEW.request_plan_version IS NOT NULL AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_request_plans q WHERE q.intent_id=NEW.intent_id AND q.phase=NEW.phase AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=NEW.candidate_ordinal AND q.plan_version=NEW.plan_version AND q.run_version=NEW.request_plan_version AND q.request_sha256=NEW.request_sha256 AND q.run_version<=NEW.prior_head_version))
OR (NEW.cause_kind='DataResult' AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_attempt_results r
    JOIN chain_post_close_macro_attempt_begins b ON b.intent_id=r.intent_id AND b.run_version=r.begin_version
    WHERE r.intent_id=NEW.intent_id AND r.phase=NEW.phase AND r.item_ordinal=NEW.item_ordinal AND r.candidate_ordinal=NEW.candidate_ordinal AND r.run_version=NEW.data_result_version AND r.continuation='Terminal' AND r.request_sha256=NEW.request_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.recorded_at=NEW.recorded_at AND r.run_version<=NEW.prior_head_version AND b.request_plan_version=NEW.request_plan_version))
OR (NEW.cause_kind='SharedControlRejected' AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_control_attempt_results c
    JOIN chain_post_close_macro_readiness_episode_plans e ON e.intent_id=c.intent_id AND e.episode_ordinal=c.episode_ordinal
    JOIN chain_post_close_macro_request_plans q ON q.intent_id=e.intent_id AND q.plan_version=e.plan_version AND q.endpoint=e.endpoint AND q.acquisition_authority=e.acquisition_authority AND q.profile='ExternalV1'
    WHERE c.intent_id=NEW.intent_id AND c.run_version=NEW.control_result_version AND c.outcome='Rejected' AND c.lease_owner=NEW.lease_owner AND c.lease_generation=NEW.lease_generation AND c.recorded_at=NEW.recorded_at AND c.run_version<=NEW.prior_head_version AND q.run_version=NEW.request_plan_version AND q.phase=NEW.phase AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=NEW.candidate_ordinal AND e.required_operation=17))
OR (NEW.cause_kind='HistoricalControlRejected' AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_plans p
    JOIN chain_post_close_macro_readiness_episode_plans e ON e.intent_id=p.intent_id AND e.plan_version=p.run_version
    JOIN chain_post_close_macro_control_attempt_results c ON c.intent_id=e.intent_id AND c.episode_ordinal=e.episode_ordinal
    JOIN chain_post_close_macro_source_finals f ON f.intent_id=c.intent_id AND f.cause_kind='ExternalControlResult' AND f.cause_version=c.run_version
    JOIN chain_post_close_macro_request_plans q ON q.intent_id=e.intent_id AND q.plan_version=p.run_version
    WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND json_extract(CAST(p.bytes AS TEXT),'$.version')=2
      AND e.episode_ordinal=1 AND e.phase='Gateway' AND e.item_ordinal=1 AND e.required_operation=17
      AND c.run_version=NEW.control_result_version AND c.outcome='Rejected'
      AND f.phase='Gateway' AND f.item_ordinal=1 AND f.run_version<=NEW.prior_head_version
      AND f.lease_owner=c.lease_owner AND f.lease_generation=c.lease_generation AND f.recorded_at=c.recorded_at
      AND f.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at
      AND q.run_version=NEW.request_plan_version AND q.phase='Gateway' AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=1
      AND q.profile='ExternalV1' AND q.endpoint=e.endpoint AND q.acquisition_authority=e.acquisition_authority
      AND q.lease_owner=NEW.lease_owner AND q.lease_generation=NEW.lease_generation AND q.recorded_at=NEW.recorded_at))
OR (NEW.call_state='NotCalled' AND EXISTS(
    SELECT 1 FROM chain_post_close_macro_attempt_begins b WHERE b.intent_id=NEW.intent_id AND b.phase=NEW.phase AND b.item_ordinal=NEW.item_ordinal AND b.candidate_ordinal=NEW.candidate_ordinal))
OR (NEW.cause_kind='LocalRouteUnavailable' AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version
    AND ((json_extract(CAST(p.bytes AS TEXT),'$.version')=3 AND json_extract(CAST(p.bytes AS TEXT),'$.local_route.state')='ObservedUnavailable')
      OR (json_extract(CAST(p.bytes AS TEXT),'$.version')=2 AND EXISTS(
        SELECT 1 FROM json_each(CAST(p.bytes AS TEXT),'$.decisions') d WHERE json_extract(d.value,'$.availability_source')='explicit-registry-local-semantic-search-disconnected')))))
OR (NEW.phase='Gateway' AND EXISTS(SELECT 1 FROM chain_post_close_macro_source_finals f WHERE f.intent_id=NEW.intent_id AND f.phase=NEW.phase AND f.item_ordinal=NEW.item_ordinal))
OR (NEW.audit_id IS NOT NULL AND EXISTS(SELECT 1 FROM chain_post_close_macro_source_finals f WHERE f.audit_id=NEW.audit_id))
OR (NEW.phase='Gateway' AND (
    NEW.audit_capability<>CASE NEW.item_ordinal WHEN 1 THEN 'GlobalNews-Eastmoney' WHEN 2 THEN 'GlobalNews-CLS' WHEN 3 THEN 'GlobalNews-Jin10' WHEN 4 THEN 'GlobalNews-ThePaper' WHEN 5 THEN 'EconomicCalendar-Jin10' END
    OR NEW.audit_provider<>CASE NEW.item_ordinal WHEN 1 THEN 'Eastmoney' WHEN 2 THEN 'Cailianpress' WHEN 3 THEN 'Jin10' WHEN 4 THEN 'ThePaper' WHEN 5 THEN 'Jin10' END
    OR NOT EXISTS(SELECT 1 FROM data_acquisition_audit a JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id=a.id
        WHERE a.id=NEW.audit_id AND a.schema_version=1 AND a.capability=NEW.audit_capability AND a.provider=NEW.audit_provider AND a.request_hash=NEW.audit_request_hash
        AND length(a.source)>0 AND length(a.observed_at)>0 AND a.outcome=NEW.current_outcome AND a.request_count=1 AND c.record_hash=NEW.audit_record_hash
        AND NEW.previous_outcome IS (SELECT prior.outcome FROM data_acquisition_audit prior WHERE prior.id<a.id AND prior.capability=a.capability AND prior.provider=a.provider ORDER BY prior.id DESC LIMIT 1)
        AND ((a.outcome='available' AND a.accepted_count>=1 AND a.rejected_count=0 AND a.reason_code='accepted' AND a.retryable=0)
          OR (a.outcome='verified_empty' AND a.accepted_count=0 AND a.rejected_count=0 AND a.reason_code='verified_empty' AND a.retryable=0)
          OR (a.outcome NOT IN ('available','verified_empty') AND a.accepted_count=0 AND a.rejected_count=1 AND length(a.reason_code)>0)))))
BEGIN SELECT RAISE(ABORT,'chain v12 macro query_terminals rejected'); END;

CREATE TRIGGER chain_post_close_macro_query_terminals_update BEFORE UPDATE ON chain_post_close_macro_query_terminals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_query_terminals_delete BEFORE DELETE ON chain_post_close_macro_query_terminals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;

CREATE TRIGGER chain_post_close_macro_dimension_terminals_guard BEFORE INSERT ON chain_post_close_macro_dimension_terminals
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.sha256=NEW.plan_sha256 AND p.run_version<=NEW.prior_head_version AND p.recorded_at<=NEW.recorded_at)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND NEW.recorded_at<p.deadline_at)
OR (NEW.dimension>1 AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension=NEW.dimension-1 AND d.pace_due<=NEW.recorded_at))
OR (NEW.last_query_terminal_version IS NOT NULL AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_query_terminals t WHERE t.intent_id=NEW.intent_id AND t.phase='WebDimension' AND t.item_ordinal=NEW.dimension AND t.run_version=NEW.last_query_terminal_version AND t.plan_version=NEW.plan_version AND t.run_version<=NEW.prior_head_version AND t.recorded_at<=NEW.recorded_at
    AND (NEW.outcome<>'SelectedResearchOnly' OR t.candidate_ordinal=NEW.selected_candidate_ordinal)))
OR EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_begins b WHERE b.intent_id=NEW.intent_id AND b.phase='WebDimension' AND b.item_ordinal=NEW.dimension AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_results r WHERE r.intent_id=b.intent_id AND r.begin_version=b.run_version))
BEGIN SELECT RAISE(ABORT,'chain v12 macro dimension_terminals rejected'); END;

CREATE TRIGGER chain_post_close_macro_dimension_terminals_update BEFORE UPDATE ON chain_post_close_macro_dimension_terminals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_dimension_terminals_delete BEFORE DELETE ON chain_post_close_macro_dimension_terminals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;

CREATE TRIGGER chain_post_close_macro_finalize_begins_guard BEFORE INSERT ON chain_post_close_macro_finalize_begins
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.sha256=NEW.plan_sha256 AND p.run_version<=NEW.prior_head_version AND p.recorded_at<=NEW.recorded_at)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_begins b WHERE b.intent_id=NEW.intent_id AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_attempt_results r WHERE r.intent_id=b.intent_id AND r.phase=b.phase AND r.item_ordinal=b.item_ordinal AND r.candidate_ordinal=b.candidate_ordinal AND r.attempt_ordinal=b.attempt_ordinal AND r.begin_version=b.run_version AND r.request_sha256=b.request_sha256 AND r.lease_owner=b.lease_owner AND r.lease_generation=b.lease_generation))
OR EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_begins b WHERE b.intent_id=NEW.intent_id AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_control_attempt_results r WHERE r.intent_id=b.intent_id AND r.episode_ordinal=b.episode_ordinal AND r.control_ordinal=b.control_ordinal AND r.begin_version=b.run_version AND r.request_sha256=b.request_sha256 AND r.lease_owner=b.lease_owner AND r.lease_generation=b.lease_generation))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.started_at=NEW.started_at AND p.deadline_at=NEW.deadline_at)
OR (NEW.kind='Complete' AND (
    5<>(SELECT count(*) FROM (
        SELECT item_ordinal FROM chain_post_close_macro_query_terminals WHERE intent_id=NEW.intent_id AND phase='Gateway'
        UNION ALL SELECT item_ordinal FROM chain_post_close_macro_source_finals WHERE intent_id=NEW.intent_id AND phase='Gateway'))
    OR 6<>(SELECT count(*) FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id)
    OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension=6 AND d.pace_due<=NEW.recorded_at)))
BEGIN SELECT RAISE(ABORT,'chain v12 macro finalize_begins rejected'); END;

CREATE TRIGGER chain_post_close_macro_finalize_begins_update BEFORE UPDATE ON chain_post_close_macro_finalize_begins BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_finalize_begins_delete BEFORE DELETE ON chain_post_close_macro_finalize_begins BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;

CREATE TRIGGER chain_post_close_macro_stage_finals_guard BEFORE INSERT ON chain_post_close_macro_stage_finals
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=12 AND NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version>12))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_runs r WHERE r.intent_id=NEW.intent_id AND r.run_id=NEW.run_id AND r.run_context_sha256=NEW.run_context_sha256 AND r.input_sha256=NEW.input_sha256 AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.head_version=NEW.run_version AND r.updated_at=NEW.recorded_at AND r.lease_until>NEW.recorded_at)
OR EXISTS(
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_request_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_readiness_episode_plans
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_control_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_source_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_query_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_dimension_terminals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_finalize_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_macro_stage_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_stage_finals f WHERE f.intent_id=NEW.intent_id)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.sha256=NEW.plan_sha256 AND p.run_version<=NEW.prior_head_version AND p.recorded_at<=NEW.recorded_at)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_begins b WHERE b.intent_id=NEW.intent_id AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_attempt_results r WHERE r.intent_id=b.intent_id AND r.phase=b.phase AND r.item_ordinal=b.item_ordinal AND r.candidate_ordinal=b.candidate_ordinal AND r.attempt_ordinal=b.attempt_ordinal AND r.begin_version=b.run_version AND r.request_sha256=b.request_sha256 AND r.lease_owner=b.lease_owner AND r.lease_generation=b.lease_generation))
OR EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_begins b WHERE b.intent_id=NEW.intent_id AND NOT EXISTS(
    SELECT 1 FROM chain_post_close_macro_control_attempt_results r WHERE r.intent_id=b.intent_id AND r.episode_ordinal=b.episode_ordinal AND r.control_ordinal=b.control_ordinal AND r.begin_version=b.run_version AND r.request_sha256=b.request_sha256 AND r.lease_owner=b.lease_owner AND r.lease_generation=b.lease_generation))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND p.started_at=NEW.started_at AND p.deadline_at=NEW.deadline_at)
OR (NEW.kind='Complete' AND (
    5<>(SELECT count(*) FROM (
        SELECT item_ordinal FROM chain_post_close_macro_query_terminals WHERE intent_id=NEW.intent_id AND phase='Gateway'
        UNION ALL SELECT item_ordinal FROM chain_post_close_macro_source_finals WHERE intent_id=NEW.intent_id AND phase='Gateway'))
    OR 6<>(SELECT count(*) FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id)
    OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_dimension_terminals d WHERE d.intent_id=NEW.intent_id AND d.dimension=6 AND d.pace_due<=NEW.recorded_at)))
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_finalize_begins b WHERE b.intent_id=NEW.intent_id AND b.run_version=NEW.finalize_begin_version AND b.run_version=NEW.prior_head_version AND b.plan_version=NEW.plan_version AND b.plan_sha256=NEW.plan_sha256 AND b.kind=NEW.kind AND b.started_at=NEW.started_at AND b.deadline_at=NEW.deadline_at AND b.facts_sha256=NEW.facts_sha256 AND b.output_bytes=NEW.output_bytes AND b.output_length=NEW.output_length AND b.output_sha256=NEW.output_sha256 AND b.lease_owner=NEW.lease_owner AND b.lease_generation=NEW.lease_generation AND b.recorded_at<=NEW.recorded_at)
BEGIN SELECT RAISE(ABORT,'chain v12 macro stage_finals rejected'); END;

CREATE TRIGGER chain_post_close_macro_stage_finals_update BEFORE UPDATE ON chain_post_close_macro_stage_finals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_stage_finals_delete BEFORE DELETE ON chain_post_close_macro_stage_finals BEGIN SELECT RAISE(ABORT,'chain v12 macro immutable'); END;
