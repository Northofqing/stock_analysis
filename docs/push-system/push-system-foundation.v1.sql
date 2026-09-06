-- PROPOSED：仅用于新建临时数据库验证；不是生产迁移器。
-- 每个业务连接必须再次启用外键与递归 trigger；时间均为非负 UTC 微秒。
PRAGMA foreign_keys=ON;
PRAGMA recursive_triggers=ON;
BEGIN IMMEDIATE;

CREATE TABLE IF NOT EXISTS push_foundation_schema (
  version INTEGER PRIMARY KEY CHECK(version=1),
  description TEXT NOT NULL CHECK(description='push-foundation-v1')
);
INSERT INTO push_foundation_schema(version,description)
SELECT 1,'push-foundation-v1' WHERE NOT EXISTS (SELECT 1 FROM push_foundation_schema);

CREATE TABLE IF NOT EXISTS push_intents (
  intent_id TEXT NOT NULL CHECK(length(intent_id) BETWEEN 1 AND 512) PRIMARY KEY,
  namespace TEXT NOT NULL CHECK(length(namespace) BETWEEN 1 AND 512),
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  occurrence_family TEXT NOT NULL CHECK(length(occurrence_family) BETWEEN 1 AND 512),
  occurrence_key TEXT NOT NULL CHECK(length(occurrence_key) BETWEEN 1 AND 512),
  completion_owner TEXT NOT NULL CHECK(length(completion_owner) BETWEEN 1 AND 512),
  source_contract_id TEXT NOT NULL CHECK(length(source_contract_id) BETWEEN 1 AND 512),
  subject TEXT NOT NULL CHECK(length(subject) BETWEEN 1 AND 512),
  audience TEXT NOT NULL CHECK(length(audience) BETWEEN 1 AND 512),
  durable_decision_id TEXT NOT NULL CHECK(length(durable_decision_id) BETWEEN 1 AND 512),
  business_date TEXT NOT NULL CHECK(length(business_date)=10 AND business_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]' AND date(business_date,'+0 days') IS business_date),
  prepared_push_bytes BLOB NOT NULL CHECK(typeof(prepared_push_bytes)='blob' AND length(prepared_push_bytes)>0),
  rendered_bytes BLOB NOT NULL CHECK(typeof(rendered_bytes)='blob' AND length(rendered_bytes)>0),
  payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256)=64 AND payload_sha256 NOT GLOB '*[^0-9a-f]*'),
  rendered_sha256 TEXT NOT NULL CHECK(length(rendered_sha256)=64 AND rendered_sha256 NOT GLOB '*[^0-9a-f]*'),
  evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  template_sha256 TEXT NOT NULL CHECK(length(template_sha256)=64 AND template_sha256 NOT GLOB '*[^0-9a-f]*'),
  source_contract_sha256 TEXT NOT NULL CHECK(length(source_contract_sha256)=64 AND source_contract_sha256 NOT GLOB '*[^0-9a-f]*'),
  state TEXT NOT NULL CHECK(state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NoData','Disabled','ResolutionRequired')),
  previous_state TEXT CHECK(previous_state IS NULL OR previous_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NoData','Disabled','ResolutionRequired')),
  reason TEXT NOT NULL CHECK(length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  lease_owner TEXT CHECK(lease_owner IS NULL OR length(lease_owner) BETWEEN 1 AND 512),
  lease_until INTEGER CHECK(lease_until IS NULL OR (typeof(lease_until)='integer' AND lease_until>=0)),
  lease_generation INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lease_generation)='integer' AND lease_generation>=0),
  version INTEGER NOT NULL DEFAULT 0 CHECK(typeof(version)='integer' AND version>=0),
  created_at INTEGER NOT NULL CHECK(typeof(created_at)='integer' AND created_at>=0),
  updated_at INTEGER NOT NULL CHECK(typeof(updated_at)='integer' AND updated_at>=0),
  CHECK(updated_at>=created_at),
  CHECK((lease_owner IS NULL)=(lease_until IS NULL)),
  UNIQUE(namespace,unit_id,completion_owner,source_contract_id,business_date,occurrence_family,occurrence_key,subject,audience),
  UNIQUE(namespace,durable_decision_id)
);
CREATE INDEX IF NOT EXISTS push_intents_recovery ON push_intents(state,lease_until,unit_id);
CREATE TRIGGER IF NOT EXISTS push_intents_insert_guard
BEFORE INSERT ON push_intents
WHEN EXISTS(SELECT 1 FROM push_intents WHERE intent_id=NEW.intent_id)
  OR NEW.version<>0 OR NEW.previous_state IS NOT NULL OR NEW.state NOT IN ('PendingDispatch','NoData','Disabled') OR NEW.lease_generation<>0 OR NEW.lease_owner IS NOT NULL
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.insert_conflict');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_immutable
BEFORE UPDATE ON push_intents
WHEN NEW.intent_id IS NOT OLD.intent_id OR
  NEW.namespace IS NOT OLD.namespace OR
  NEW.unit_id IS NOT OLD.unit_id OR
  NEW.occurrence_family IS NOT OLD.occurrence_family OR
  NEW.occurrence_key IS NOT OLD.occurrence_key OR
  NEW.completion_owner IS NOT OLD.completion_owner OR
  NEW.source_contract_id IS NOT OLD.source_contract_id OR
  NEW.subject IS NOT OLD.subject OR
  NEW.audience IS NOT OLD.audience OR
  NEW.durable_decision_id IS NOT OLD.durable_decision_id OR
  NEW.business_date IS NOT OLD.business_date OR
  NEW.created_at IS NOT OLD.created_at OR
  NEW.prepared_push_bytes IS NOT OLD.prepared_push_bytes OR
  NEW.rendered_bytes IS NOT OLD.rendered_bytes OR
  NEW.payload_sha256 IS NOT OLD.payload_sha256 OR
  NEW.rendered_sha256 IS NOT OLD.rendered_sha256 OR
  NEW.evidence_sha256 IS NOT OLD.evidence_sha256 OR
  NEW.template_sha256 IS NOT OLD.template_sha256 OR
  NEW.source_contract_sha256 IS NOT OLD.source_contract_sha256
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.immutable');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_delete
BEFORE DELETE ON push_intents
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.delete_forbidden');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_cas
BEFORE UPDATE ON push_intents
WHEN NEW.version<>OLD.version+1 OR NEW.previous_state IS NOT OLD.state OR NEW.updated_at<OLD.updated_at
  OR NEW.lease_generation<OLD.lease_generation OR NEW.lease_generation>OLD.lease_generation+1
  OR (NEW.lease_owner IS NOT OLD.lease_owner AND NEW.lease_owner IS NOT NULL AND NEW.lease_generation<>OLD.lease_generation+1)
  OR (NEW.lease_owner IS NOT OLD.lease_owner AND NEW.lease_owner IS NOT NULL AND OLD.lease_owner IS NOT NULL AND NEW.updated_at<OLD.lease_until)
  OR NOT (
    (NEW.state=OLD.state AND OLD.state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','ResolutionRequired'))
    OR (OLD.state='PendingDispatch' AND NEW.state IN ('AwaitingAuthority','NoData','Disabled','ResolutionRequired'))
    OR (OLD.state='AwaitingAuthority' AND NEW.state IN ('AwaitingFinalizer','ResolutionRequired'))
    OR (OLD.state='AwaitingFinalizer' AND NEW.state IN ('Completed','ResolutionRequired'))
    OR (OLD.state='ResolutionRequired' AND NEW.state='AwaitingFinalizer')
    OR (OLD.state IN ('Completed','NoData','Disabled') AND NEW.state='ResolutionRequired')
  )
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.cas_or_edge_invalid');
END;

CREATE TABLE IF NOT EXISTS push_intent_transitions (
  event_id TEXT NOT NULL CHECK(length(event_id) BETWEEN 1 AND 512) PRIMARY KEY,
  intent_id TEXT NOT NULL CHECK(length(intent_id) BETWEEN 1 AND 512) REFERENCES push_intents(intent_id),
  from_state TEXT NOT NULL CHECK(from_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NoData','Disabled','ResolutionRequired')),
  to_state TEXT NOT NULL CHECK(to_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NoData','Disabled','ResolutionRequired')),
  expected_version INTEGER NOT NULL CHECK(typeof(expected_version)='integer' AND expected_version>=0),
  result_version INTEGER NOT NULL CHECK(typeof(result_version)='integer' AND result_version=expected_version+1),
  previous_sha256 TEXT CHECK(previous_sha256 IS NULL OR (length(previous_sha256)=64 AND previous_sha256 NOT GLOB '*[^0-9a-f]*')),
  canonical_sha256 TEXT NOT NULL CHECK(length(canonical_sha256)=64 AND canonical_sha256 NOT GLOB '*[^0-9a-f]*'),
  actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 512),
  reason TEXT NOT NULL CHECK(length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  terminal_ref_id TEXT CHECK(terminal_ref_id IS NULL OR length(terminal_ref_id) BETWEEN 1 AND 512),
  terminal_binding_sha256 TEXT CHECK(terminal_binding_sha256 IS NULL OR (length(terminal_binding_sha256)=64 AND terminal_binding_sha256 NOT GLOB '*[^0-9a-f]*')),
  occurred_at INTEGER NOT NULL CHECK(typeof(occurred_at)='integer' AND occurred_at>=0),
  CHECK((terminal_ref_id IS NULL)=(terminal_binding_sha256 IS NULL)),
  CHECK((to_state='Completed')=(terminal_ref_id IS NOT NULL)),
  CHECK((result_version=1)=(previous_sha256 IS NULL)),
  UNIQUE(intent_id,result_version)
);
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_binding
BEFORE INSERT ON push_intent_transitions
WHEN EXISTS(SELECT 1 FROM push_intent_transitions WHERE event_id=NEW.event_id)
  OR NOT EXISTS(SELECT 1 FROM push_intents i WHERE i.intent_id=NEW.intent_id AND i.state=NEW.to_state AND i.previous_state=NEW.from_state AND i.version=NEW.result_version AND i.updated_at<=NEW.occurred_at)
  OR (NEW.result_version>1 AND NOT EXISTS(SELECT 1 FROM push_intent_transitions p WHERE p.intent_id=NEW.intent_id AND p.result_version=NEW.expected_version AND p.to_state=NEW.from_state AND p.canonical_sha256=NEW.previous_sha256))
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.transition_binding_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_update
BEFORE UPDATE ON push_intent_transitions
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_delete
BEFORE DELETE ON push_intent_transitions
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.append_only');
END;

CREATE TABLE IF NOT EXISTS push_activation_manifests (
  manifest_sha256 TEXT NOT NULL CHECK(length(manifest_sha256)=64 AND manifest_sha256 NOT GLOB '*[^0-9a-f]*') PRIMARY KEY,
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=1),
  previous_manifest_sha256 TEXT CHECK(previous_manifest_sha256 IS NULL OR (length(previous_manifest_sha256)=64 AND previous_manifest_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  desired_state TEXT NOT NULL CHECK(desired_state IN ('Disabled','Shadow','Active','Draining')),
  physical_owner TEXT NOT NULL CHECK(length(physical_owner) BETWEEN 1 AND 512),
  build_commit TEXT NOT NULL CHECK(length(build_commit)=40 AND build_commit NOT GLOB '*[^0-9a-f]*'),
  build_sha256 TEXT NOT NULL CHECK(length(build_sha256)=64 AND build_sha256 NOT GLOB '*[^0-9a-f]*'),
  catalog_sha256 TEXT NOT NULL CHECK(length(catalog_sha256)=64 AND catalog_sha256 NOT GLOB '*[^0-9a-f]*'),
  business_schema_sha256 TEXT NOT NULL CHECK(length(business_schema_sha256)=64 AND business_schema_sha256 NOT GLOB '*[^0-9a-f]*'),
  durable_schema_sha256 TEXT NOT NULL CHECK(length(durable_schema_sha256)=64 AND durable_schema_sha256 NOT GLOB '*[^0-9a-f]*'),
  template_sha256 TEXT NOT NULL CHECK(length(template_sha256)=64 AND template_sha256 NOT GLOB '*[^0-9a-f]*'),
  source_contract_sha256 TEXT NOT NULL CHECK(length(source_contract_sha256)=64 AND source_contract_sha256 NOT GLOB '*[^0-9a-f]*'),
  evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  approved_by TEXT NOT NULL CHECK(length(approved_by) BETWEEN 1 AND 512),
  approved_at INTEGER NOT NULL CHECK(typeof(approved_at)='integer' AND approved_at>=0),
  window_start INTEGER NOT NULL CHECK(typeof(window_start)='integer' AND window_start>=0),
  window_end INTEGER NOT NULL CHECK(typeof(window_end)='integer' AND window_end>=0),
  rollback_target_sha256 TEXT CHECK(rollback_target_sha256 IS NULL OR (length(rollback_target_sha256)=64 AND rollback_target_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  created_at INTEGER NOT NULL CHECK(typeof(created_at)='integer' AND created_at>=0),
  CHECK(window_end>window_start AND approved_at<=created_at),
  CHECK((generation=1)=(previous_manifest_sha256 IS NULL)),
  UNIQUE(unit_id,generation)
);
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_chain
BEFORE INSERT ON push_activation_manifests
WHEN EXISTS(SELECT 1 FROM push_activation_manifests WHERE manifest_sha256=NEW.manifest_sha256)
  OR NEW.generation<>COALESCE((SELECT MAX(generation)+1 FROM push_activation_manifests WHERE unit_id=NEW.unit_id),1)
  OR (NEW.generation=1 AND (NEW.desired_state<>'Disabled' OR NEW.rollback_target_sha256 IS NOT NULL))
  OR (NEW.generation>1 AND NOT EXISTS(SELECT 1 FROM push_activation_manifests p WHERE p.unit_id=NEW.unit_id AND p.generation=NEW.generation-1 AND p.manifest_sha256=NEW.previous_manifest_sha256))
  OR (NEW.generation>1 AND NEW.rollback_target_sha256 IS NULL AND NOT EXISTS(
    SELECT 1 FROM push_activation_manifests p WHERE p.manifest_sha256=NEW.previous_manifest_sha256 AND (
      (p.desired_state='Disabled' AND NEW.desired_state='Shadow') OR (p.desired_state='Shadow' AND NEW.desired_state='Active')
      OR (p.desired_state='Active' AND NEW.desired_state='Draining') OR (p.desired_state='Draining' AND NEW.desired_state='Disabled'))))
  OR (NEW.rollback_target_sha256 IS NOT NULL AND NOT EXISTS(SELECT 1 FROM push_activation_manifests r WHERE r.manifest_sha256=NEW.rollback_target_sha256 AND r.unit_id=NEW.unit_id AND r.generation<NEW.generation AND r.desired_state=NEW.desired_state AND r.physical_owner=NEW.physical_owner))
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.generation_or_edge_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_update
BEFORE UPDATE ON push_activation_manifests
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.history_immutable');
END;
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_delete
BEFORE DELETE ON push_activation_manifests
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.history_immutable');
END;

CREATE TABLE IF NOT EXISTS push_promotion_journal (
  event_id TEXT NOT NULL CHECK(length(event_id) BETWEEN 1 AND 512) PRIMARY KEY,
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=1),
  from_manifest_sha256 TEXT CHECK(from_manifest_sha256 IS NULL OR (length(from_manifest_sha256)=64 AND from_manifest_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  to_manifest_sha256 TEXT NOT NULL CHECK(length(to_manifest_sha256)=64 AND to_manifest_sha256 NOT GLOB '*[^0-9a-f]*') REFERENCES push_activation_manifests(manifest_sha256),
  actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 512),
  action TEXT NOT NULL CHECK(action IN ('Initialize','EnterShadow','Activate','Drain','Disable','Rollback')),
  reason TEXT NOT NULL CHECK(length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  window_start INTEGER NOT NULL CHECK(typeof(window_start)='integer' AND window_start>=0),
  window_end INTEGER NOT NULL CHECK(typeof(window_end)='integer' AND window_end>=0),
  evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  rollback_target_sha256 TEXT CHECK(rollback_target_sha256 IS NULL OR (length(rollback_target_sha256)=64 AND rollback_target_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  previous_sha256 TEXT CHECK(previous_sha256 IS NULL OR (length(previous_sha256)=64 AND previous_sha256 NOT GLOB '*[^0-9a-f]*')),
  canonical_sha256 TEXT NOT NULL CHECK(length(canonical_sha256)=64 AND canonical_sha256 NOT GLOB '*[^0-9a-f]*'),
  occurred_at INTEGER NOT NULL CHECK(typeof(occurred_at)='integer' AND occurred_at>=0),
  CHECK(window_end>window_start AND occurred_at>=window_start AND occurred_at<window_end),
  CHECK((generation=1)=(from_manifest_sha256 IS NULL)),
  CHECK((generation=1)=(previous_sha256 IS NULL)),
  CHECK((action='Rollback')=(rollback_target_sha256 IS NOT NULL)),
  UNIQUE(unit_id,generation)
);
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_binding
BEFORE INSERT ON push_promotion_journal
WHEN EXISTS(SELECT 1 FROM push_promotion_journal WHERE event_id=NEW.event_id)
  OR NEW.generation<>COALESCE((SELECT MAX(generation)+1 FROM push_promotion_journal WHERE unit_id=NEW.unit_id),1)
  OR NOT EXISTS(SELECT 1 FROM push_activation_manifests m WHERE m.manifest_sha256=NEW.to_manifest_sha256 AND m.unit_id=NEW.unit_id AND m.generation=NEW.generation AND m.previous_manifest_sha256 IS NEW.from_manifest_sha256 AND m.rollback_target_sha256 IS NEW.rollback_target_sha256 AND m.window_start=NEW.window_start AND m.window_end=NEW.window_end AND m.evidence_sha256=NEW.evidence_sha256 AND m.approved_by=NEW.actor AND m.approved_at<=NEW.occurred_at AND (
    (NEW.action='Initialize' AND m.generation=1 AND m.desired_state='Disabled')
    OR (NEW.action='EnterShadow' AND m.generation>1 AND m.desired_state='Shadow' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Activate' AND m.desired_state='Active' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Drain' AND m.desired_state='Draining' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Disable' AND m.generation>1 AND m.desired_state='Disabled' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Rollback' AND m.rollback_target_sha256 IS NOT NULL)))
  OR (NEW.generation>1 AND NOT EXISTS(SELECT 1 FROM push_promotion_journal p WHERE p.unit_id=NEW.unit_id AND p.generation=NEW.generation-1 AND p.to_manifest_sha256=NEW.from_manifest_sha256 AND p.canonical_sha256=NEW.previous_sha256))
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.journal_binding_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_update
BEFORE UPDATE ON push_promotion_journal
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_delete
BEFORE DELETE ON push_promotion_journal
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_schema_update
BEFORE UPDATE ON push_foundation_schema
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_schema_delete
BEFORE DELETE ON push_foundation_schema
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;

COMMIT;
