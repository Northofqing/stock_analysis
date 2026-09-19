-- Chain v11: immutable Macro plan, request, readiness, data and source-final facts.
-- v11 is an undeployed development layout and is installed only by the attested v10-to-v11 migration.

CREATE TABLE chain_post_close_macro_plans (
    intent_id TEXT PRIMARY KEY REFERENCES chain_post_close_runs(intent_id),
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
    parent_version INTEGER NOT NULL,
    parent_sha256 TEXT NOT NULL CHECK(length(parent_sha256)=64),
    started_at INTEGER NOT NULL CHECK(started_at>=0),
    deadline_at INTEGER NOT NULL CHECK(deadline_at-started_at=15000000),
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    CHECK(recorded_at>=started_at AND recorded_at<deadline_at),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,run_version,request_sha256),
    FOREIGN KEY(intent_id,parent_version,parent_sha256)
        REFERENCES chain_post_close_dragon_tiger_finals(intent_id,run_version,final_sha256)
);

CREATE TABLE chain_post_close_macro_request_plans (
    intent_id TEXT NOT NULL,
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
    item_ordinal INTEGER NOT NULL CHECK(item_ordinal>=1),
    candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal>=1),
    plan_version INTEGER NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    profile TEXT NOT NULL CHECK(profile IN ('LocalBridgeV1','ExternalV1')),
    acquisition_authority TEXT,
    endpoint TEXT NOT NULL CHECK(length(endpoint) BETWEEN 1 AND 2048),
    PRIMARY KEY(intent_id,phase,item_ordinal,candidate_ordinal),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,run_version,request_sha256),
    FOREIGN KEY(intent_id,plan_version)
        REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    CHECK((phase='Gateway' AND item_ordinal BETWEEN 1 AND 5 AND candidate_ordinal=1)
       OR (phase='WebDimension' AND item_ordinal BETWEEN 1 AND 6)),
    CHECK((profile='LocalBridgeV1' AND acquisition_authority IS NULL)
       OR (profile='ExternalV1' AND acquisition_authority IS NOT NULL
           AND length(acquisition_authority) BETWEEN 11 AND 512))
);

CREATE TABLE chain_post_close_macro_readiness_episode_plans (
    intent_id TEXT NOT NULL,
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
    episode_ordinal INTEGER NOT NULL CHECK(episode_ordinal>=1),
    plan_version INTEGER NOT NULL,
    request_plan_version INTEGER NOT NULL,
    phase TEXT NOT NULL CHECK(phase='Gateway'),
    item_ordinal INTEGER NOT NULL CHECK(item_ordinal BETWEEN 1 AND 4),
    candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal=1),
    required_operation INTEGER NOT NULL CHECK(required_operation=17),
    endpoint TEXT NOT NULL CHECK(endpoint LIKE 'https://%'),
    acquisition_authority TEXT NOT NULL CHECK(length(acquisition_authority) BETWEEN 11 AND 512),
    health_request_sha256 TEXT NOT NULL CHECK(length(health_request_sha256)=64),
    capabilities_request_sha256 TEXT NOT NULL CHECK(length(capabilities_request_sha256)=64),
    PRIMARY KEY(intent_id,episode_ordinal),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,episode_ordinal,run_version),
    FOREIGN KEY(intent_id,plan_version)
        REFERENCES chain_post_close_macro_plans(intent_id,run_version),
    FOREIGN KEY(intent_id,request_plan_version)
        REFERENCES chain_post_close_macro_request_plans(intent_id,run_version),
    CHECK(health_request_sha256<>capabilities_request_sha256)
);

CREATE TABLE chain_post_close_macro_control_attempt_begins (
    intent_id TEXT NOT NULL,
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
    episode_ordinal INTEGER NOT NULL,
    control_ordinal INTEGER NOT NULL CHECK(control_ordinal IN (1,2)),
    kind TEXT NOT NULL CHECK((control_ordinal=1 AND kind='Health') OR (control_ordinal=2 AND kind='Capabilities')),
    episode_plan_version INTEGER NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    health_result_version INTEGER,
    PRIMARY KEY(intent_id,episode_ordinal,control_ordinal),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,episode_ordinal,control_ordinal,run_version,request_sha256,lease_owner,lease_generation),
    FOREIGN KEY(intent_id,episode_ordinal)
        REFERENCES chain_post_close_macro_readiness_episode_plans(intent_id,episode_ordinal),
    FOREIGN KEY(intent_id,episode_ordinal,episode_plan_version)
        REFERENCES chain_post_close_macro_readiness_episode_plans(intent_id,episode_ordinal,run_version),
    FOREIGN KEY(intent_id,episode_ordinal,health_result_version)
        REFERENCES chain_post_close_macro_control_attempt_results(intent_id,episode_ordinal,run_version),
    CHECK((control_ordinal=1 AND health_result_version IS NULL)
       OR (control_ordinal=2 AND health_result_version IS NOT NULL))
);

CREATE TABLE chain_post_close_macro_control_attempt_results (
    intent_id TEXT NOT NULL,
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
    episode_ordinal INTEGER NOT NULL,
    control_ordinal INTEGER NOT NULL CHECK(control_ordinal IN (1,2)),
    kind TEXT NOT NULL CHECK((control_ordinal=1 AND kind='Health') OR (control_ordinal=2 AND kind='Capabilities')),
    begin_version INTEGER NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    outcome TEXT NOT NULL CHECK(outcome IN ('Ready','Rejected')),
    PRIMARY KEY(intent_id,episode_ordinal,control_ordinal),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,episode_ordinal,run_version),
    FOREIGN KEY(intent_id,episode_ordinal,control_ordinal,begin_version,request_sha256,lease_owner,lease_generation)
        REFERENCES chain_post_close_macro_control_attempt_begins(intent_id,episode_ordinal,control_ordinal,run_version,request_sha256,lease_owner,lease_generation)
);

CREATE TABLE chain_post_close_macro_attempt_begins (
    intent_id TEXT NOT NULL,
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
    item_ordinal INTEGER NOT NULL CHECK(item_ordinal>=1),
    candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal>=1),
    attempt_ordinal INTEGER NOT NULL CHECK(attempt_ordinal>=1),
    request_plan_version INTEGER NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    readiness_result_version INTEGER,
    previous_result_version INTEGER,
    PRIMARY KEY(intent_id,phase,item_ordinal,candidate_ordinal,attempt_ordinal),
    UNIQUE(intent_id,run_version),
    UNIQUE(intent_id,phase,item_ordinal,candidate_ordinal,attempt_ordinal,run_version,request_sha256,lease_owner,lease_generation),
    FOREIGN KEY(intent_id,request_plan_version,request_sha256)
        REFERENCES chain_post_close_macro_request_plans(intent_id,run_version,request_sha256),
    FOREIGN KEY(intent_id,previous_result_version)
        REFERENCES chain_post_close_macro_attempt_results(intent_id,run_version),
    FOREIGN KEY(intent_id,readiness_result_version)
        REFERENCES chain_post_close_macro_control_attempt_results(intent_id,run_version),
    CHECK((phase='Gateway' AND item_ordinal BETWEEN 1 AND 5 AND candidate_ordinal=1)
       OR (phase='WebDimension' AND item_ordinal BETWEEN 1 AND 6))
);

CREATE TABLE chain_post_close_macro_attempt_results (
    intent_id TEXT NOT NULL,
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
    item_ordinal INTEGER NOT NULL CHECK(item_ordinal>=1),
    candidate_ordinal INTEGER NOT NULL CHECK(candidate_ordinal>=1),
    attempt_ordinal INTEGER NOT NULL CHECK(attempt_ordinal>=1),
    begin_version INTEGER NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256)=64),
    continuation TEXT NOT NULL CHECK(continuation IN ('Retry','Terminal')),
    retry_not_before INTEGER,
    PRIMARY KEY(intent_id,phase,item_ordinal,candidate_ordinal,attempt_ordinal),
    UNIQUE(intent_id,run_version),
    FOREIGN KEY(intent_id,phase,item_ordinal,candidate_ordinal,attempt_ordinal,begin_version,request_sha256,lease_owner,lease_generation)
        REFERENCES chain_post_close_macro_attempt_begins(intent_id,phase,item_ordinal,candidate_ordinal,attempt_ordinal,run_version,request_sha256,lease_owner,lease_generation),
    CHECK((phase='Gateway' AND item_ordinal BETWEEN 1 AND 5 AND candidate_ordinal=1)
       OR (phase='WebDimension' AND item_ordinal BETWEEN 1 AND 6)),
    CHECK((continuation='Retry' AND retry_not_before IS NOT NULL AND retry_not_before>=recorded_at)
       OR (continuation='Terminal' AND retry_not_before IS NULL))
);

CREATE TABLE chain_post_close_macro_source_finals (
    intent_id TEXT NOT NULL,
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
    item_ordinal INTEGER NOT NULL CHECK(item_ordinal>=1),
    cause_kind TEXT NOT NULL CHECK(cause_kind IN ('DataResult','ExternalControlResult')),
    cause_version INTEGER NOT NULL CHECK(cause_version=prior_head_version),
    native_bytes BLOB NOT NULL CHECK(typeof(native_bytes)='blob' AND length(native_bytes)>0),
    native_length INTEGER NOT NULL CHECK(native_length=length(native_bytes)),
    native_sha256 TEXT NOT NULL CHECK(length(native_sha256)=64 AND native_sha256 NOT GLOB '*[^0-9a-f]*'),
    audit_id INTEGER NOT NULL UNIQUE REFERENCES data_acquisition_audit(id),
    audit_record_hash TEXT NOT NULL CHECK(length(audit_record_hash)=64),
    previous_outcome TEXT,
    current_outcome TEXT NOT NULL CHECK(current_outcome IN ('available','verified_empty','partial','unavailable','invalid_request','audit_failure')),
    PRIMARY KEY(intent_id,phase,item_ordinal),
    UNIQUE(intent_id,run_version),
    CHECK((phase='Gateway' AND item_ordinal BETWEEN 1 AND 5)
       OR (phase='WebDimension' AND item_ordinal BETWEEN 1 AND 6)),
    FOREIGN KEY(audit_id) REFERENCES data_acquisition_audit_chain(acquisition_audit_id)
);

-- Every Macro fact is immutable, is admitted only at the current fenced run head,
-- and rejects a run-version already owned by any of the eight Macro fact tables.
CREATE TRIGGER chain_post_close_macro_plans_guard BEFORE INSERT ON chain_post_close_macro_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_dragon_tiger_finals p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.parent_version AND p.final_sha256=NEW.parent_sha256 AND p.run_id=NEW.run_id AND p.run_context_sha256=NEW.run_context_sha256 AND p.input_sha256=NEW.input_sha256 AND p.applied_at<=NEW.started_at AND p.run_version<=NEW.prior_head_version AND p.lease_generation<=NEW.lease_generation AND (p.lease_generation<NEW.lease_generation OR p.lease_owner=NEW.lease_owner) AND p.final_outcome IN ('Available','VerifiedEmpty'))
BEGIN SELECT RAISE(ABORT,'chain v11 macro insert rejected'); END;

CREATE TRIGGER chain_post_close_macro_request_plans_guard BEFORE INSERT ON chain_post_close_macro_request_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_plans p WHERE p.intent_id=NEW.intent_id AND p.run_version=NEW.plan_version AND NEW.prior_head_version=p.run_version AND p.run_id=NEW.run_id AND p.run_context_sha256=NEW.run_context_sha256 AND p.input_sha256=NEW.input_sha256 AND p.lease_owner=NEW.lease_owner AND p.lease_generation=NEW.lease_generation AND p.recorded_at=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v11 macro request plan rejected'); END;

CREATE TRIGGER chain_post_close_macro_readiness_episode_plans_guard BEFORE INSERT ON chain_post_close_macro_readiness_episode_plans
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_request_plans q JOIN chain_post_close_macro_plans p ON p.intent_id=q.intent_id AND p.run_version=q.plan_version WHERE q.intent_id=NEW.intent_id AND q.run_version=NEW.request_plan_version AND p.run_version=NEW.plan_version AND q.profile='ExternalV1' AND q.phase=NEW.phase AND q.item_ordinal=NEW.item_ordinal AND q.candidate_ordinal=NEW.candidate_ordinal AND q.endpoint=NEW.endpoint AND q.acquisition_authority=NEW.acquisition_authority AND NEW.prior_head_version=q.run_version AND q.run_id=NEW.run_id AND q.run_context_sha256=NEW.run_context_sha256 AND q.input_sha256=NEW.input_sha256 AND q.lease_owner=NEW.lease_owner AND q.lease_generation=NEW.lease_generation AND q.recorded_at=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v11 macro readiness plan rejected'); END;

CREATE TRIGGER chain_post_close_macro_control_attempt_begins_guard BEFORE INSERT ON chain_post_close_macro_control_attempt_begins
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_readiness_episode_plans e JOIN chain_post_close_macro_plans p ON p.intent_id=e.intent_id AND p.run_version=e.plan_version WHERE e.intent_id=NEW.intent_id AND e.episode_ordinal=NEW.episode_ordinal AND e.run_version=NEW.episode_plan_version AND e.run_version<=NEW.prior_head_version AND e.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
OR (NEW.control_ordinal=2 AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_results h WHERE h.intent_id=NEW.intent_id AND h.episode_ordinal=NEW.episode_ordinal AND h.control_ordinal=1 AND h.kind='Health' AND h.run_version=NEW.health_result_version AND h.outcome='Ready' AND h.run_version<=NEW.prior_head_version AND h.recorded_at<=NEW.recorded_at))
BEGIN SELECT RAISE(ABORT,'chain v11 macro control begin rejected'); END;

CREATE TRIGGER chain_post_close_macro_control_attempt_results_guard BEFORE INSERT ON chain_post_close_macro_control_attempt_results
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_begins b JOIN chain_post_close_macro_plans p ON p.intent_id=b.intent_id WHERE b.intent_id=NEW.intent_id AND b.episode_ordinal=NEW.episode_ordinal AND b.control_ordinal=NEW.control_ordinal AND b.kind=NEW.kind AND b.run_version=NEW.begin_version AND b.request_sha256=NEW.request_sha256 AND b.lease_owner=NEW.lease_owner AND b.lease_generation=NEW.lease_generation AND b.run_version<=NEW.prior_head_version AND b.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v11 macro control result rejected'); END;

CREATE TRIGGER chain_post_close_macro_attempt_begins_guard BEFORE INSERT ON chain_post_close_macro_attempt_begins
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
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
                  AND e.request_plan_version=q.run_version
                  AND e.phase=NEW.phase
                  AND e.item_ordinal=NEW.item_ordinal
                  AND e.candidate_ordinal=NEW.candidate_ordinal
                  AND e.required_operation=17
                  AND e.endpoint=q.endpoint
                  AND e.acquisition_authority=q.acquisition_authority
                  AND h.recorded_at<=c.recorded_at
                  AND c.recorded_at<=NEW.recorded_at
                  AND c.run_version<=NEW.prior_head_version
            )))
)
OR EXISTS(SELECT 1 FROM chain_post_close_macro_source_finals f WHERE f.intent_id=NEW.intent_id AND f.phase=NEW.phase AND f.item_ordinal=NEW.item_ordinal)
BEGIN SELECT RAISE(ABORT,'chain v11 macro data begin rejected'); END;

CREATE TRIGGER chain_post_close_macro_attempt_results_guard BEFORE INSERT ON chain_post_close_macro_attempt_results
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR NOT EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_begins b JOIN chain_post_close_macro_plans p ON p.intent_id=b.intent_id WHERE b.intent_id=NEW.intent_id AND b.phase=NEW.phase AND b.item_ordinal=NEW.item_ordinal AND b.candidate_ordinal=NEW.candidate_ordinal AND b.attempt_ordinal=NEW.attempt_ordinal AND b.run_version=NEW.begin_version AND b.request_sha256=NEW.request_sha256 AND b.lease_owner=NEW.lease_owner AND b.lease_generation=NEW.lease_generation AND b.run_version<=NEW.prior_head_version AND b.recorded_at<=NEW.recorded_at AND NEW.recorded_at<p.deadline_at)
BEGIN SELECT RAISE(ABORT,'chain v11 macro data result rejected'); END;

CREATE TRIGGER chain_post_close_macro_source_finals_guard BEFORE INSERT ON chain_post_close_macro_source_finals
WHEN NOT EXISTS(SELECT 1 FROM chain_post_close_layouts WHERE layout_version=11)
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
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
OR (NEW.cause_kind='DataResult' AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_attempt_results r WHERE r.intent_id=NEW.intent_id AND r.phase=NEW.phase AND r.item_ordinal=NEW.item_ordinal AND r.run_version=NEW.cause_version AND r.continuation='Terminal' AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.recorded_at=NEW.recorded_at))
OR (NEW.cause_kind='ExternalControlResult' AND NOT EXISTS(SELECT 1 FROM chain_post_close_macro_control_attempt_results r JOIN chain_post_close_macro_readiness_episode_plans e ON e.intent_id=r.intent_id AND e.episode_ordinal=r.episode_ordinal WHERE r.intent_id=NEW.intent_id AND e.phase=NEW.phase AND e.item_ordinal=NEW.item_ordinal AND r.run_version=NEW.cause_version AND r.outcome='Rejected' AND r.lease_owner=NEW.lease_owner AND r.lease_generation=NEW.lease_generation AND r.recorded_at=NEW.recorded_at))
OR NOT EXISTS(
    SELECT 1 FROM data_acquisition_audit audit
    JOIN data_acquisition_audit_chain chain ON chain.acquisition_audit_id=audit.id
    WHERE audit.id=NEW.audit_id
      AND audit.schema_version=1
      AND audit.capability='GlobalNews-Eastmoney'
      AND audit.provider='Eastmoney'
      AND length(audit.source)>0
      AND audit.request_hash='fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c'
      AND length(audit.observed_at)>0
      AND audit.outcome=NEW.current_outcome
      AND audit.request_count=1
      AND chain.record_hash=NEW.audit_record_hash
      AND NEW.previous_outcome IS (
          SELECT previous.outcome FROM data_acquisition_audit previous
          WHERE previous.id<audit.id
            AND previous.capability=audit.capability
            AND previous.provider=audit.provider
          ORDER BY previous.id DESC LIMIT 1
      )
      AND ((audit.outcome='available' AND audit.accepted_count>=1 AND audit.rejected_count=0 AND audit.reason_code='accepted' AND audit.retryable=0)
        OR (audit.outcome='verified_empty' AND audit.accepted_count=0 AND audit.rejected_count=0 AND audit.reason_code='verified_empty' AND audit.retryable=0)
        OR (audit.outcome NOT IN ('available','verified_empty') AND audit.accepted_count=0 AND audit.rejected_count=1 AND length(audit.reason_code)>0))
)
BEGIN SELECT RAISE(ABORT,'chain v11 macro source final rejected'); END;

CREATE TRIGGER chain_post_close_macro_plans_update BEFORE UPDATE ON chain_post_close_macro_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_plans_delete BEFORE DELETE ON chain_post_close_macro_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_request_plans_update BEFORE UPDATE ON chain_post_close_macro_request_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_request_plans_delete BEFORE DELETE ON chain_post_close_macro_request_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_readiness_episode_plans_update BEFORE UPDATE ON chain_post_close_macro_readiness_episode_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_readiness_episode_plans_delete BEFORE DELETE ON chain_post_close_macro_readiness_episode_plans BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_control_attempt_begins_update BEFORE UPDATE ON chain_post_close_macro_control_attempt_begins BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_control_attempt_begins_delete BEFORE DELETE ON chain_post_close_macro_control_attempt_begins BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_control_attempt_results_update BEFORE UPDATE ON chain_post_close_macro_control_attempt_results BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_control_attempt_results_delete BEFORE DELETE ON chain_post_close_macro_control_attempt_results BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_attempt_begins_update BEFORE UPDATE ON chain_post_close_macro_attempt_begins BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_attempt_begins_delete BEFORE DELETE ON chain_post_close_macro_attempt_begins BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_attempt_results_update BEFORE UPDATE ON chain_post_close_macro_attempt_results BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_attempt_results_delete BEFORE DELETE ON chain_post_close_macro_attempt_results BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_source_finals_update BEFORE UPDATE ON chain_post_close_macro_source_finals BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;
CREATE TRIGGER chain_post_close_macro_source_finals_delete BEFORE DELETE ON chain_post_close_macro_source_finals BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END;

CREATE TRIGGER chain_post_close_stage_begins_macro_fence BEFORE INSERT ON chain_post_close_stage_begins
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_stage_results_macro_fence BEFORE INSERT ON chain_post_close_stage_results
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_cache_writes_macro_fence BEFORE INSERT ON chain_post_close_concept_cache_writes
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_cluster_configurations_macro_fence BEFORE INSERT ON chain_post_close_cluster_configurations
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_cluster_materials_macro_fence BEFORE INSERT ON chain_post_close_cluster_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_chain_daily_applications_macro_fence BEFORE INSERT ON chain_post_close_chain_daily_applications
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_attempt_begins_macro_fence BEFORE INSERT ON chain_post_close_board_attempt_begins
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_attempt_results_macro_fence BEFORE INSERT ON chain_post_close_board_attempt_results
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_kind_finals_macro_fence BEFORE INSERT ON chain_post_close_board_kind_finals
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_directory_materials_macro_fence BEFORE INSERT ON chain_post_close_board_directory_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_selections_macro_fence BEFORE INSERT ON chain_post_close_board_selections
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_status_materials_macro_fence BEFORE INSERT ON chain_post_close_board_status_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_board_error_materials_macro_fence BEFORE INSERT ON chain_post_close_board_error_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_occurrences_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_occurrences
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_attempt_begins_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_attempt_begins
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_attempt_results_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_attempt_results
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_status_materials_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_status_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_error_materials_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_error_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_concept_rpc_finals_macro_fence BEFORE INSERT ON chain_post_close_concept_rpc_finals
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_materials_macro_fence BEFORE INSERT ON chain_post_close_position_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_materials_macro_fence BEFORE INSERT ON chain_post_close_position_concept_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_occurrences_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_occurrences
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_begins_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_attempt_begins
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_results_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_attempt_results
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_status_materials_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_status_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_error_materials_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_error_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_rpc_finals_macro_fence BEFORE INSERT ON chain_post_close_position_concept_rpc_finals
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_position_concept_cache_writes_macro_fence BEFORE INSERT ON chain_post_close_position_concept_cache_writes
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_occurrences_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_occurrences
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_attempt_begins_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_attempt_begins
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_attempt_results_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_attempt_results
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_status_materials_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_status_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_error_materials_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_error_materials
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_finals_macro_fence BEFORE INSERT ON chain_post_close_dragon_tiger_finals
WHEN EXISTS(SELECT 1 FROM chain_post_close_macro_plans WHERE intent_id=NEW.intent_id)
BEGIN SELECT RAISE(ABORT,'chain v11 macro parent sealed'); END;
