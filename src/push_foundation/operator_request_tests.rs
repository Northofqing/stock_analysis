use serde_json::{json, Value};

use super::operator_request::{
    OperatorCommand, OperatorRequestError, OperatorTarget, UnverifiedOperatorRequest,
};
use crate::monitor::push_job::{Namespace, ReasonCode};

const UNIT_COMMAND_ID: &str = "f43e17c300e257c88c6e96a99ca935091b85e0fc910655e9df98891d760dbd1b";

fn unit_wire_value() -> Value {
    json!({
        "command": "inspect",
        "target": {
            "kind": "unit",
            "id": "MU-p01",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        "expected_version": 0,
        "expected_generation": 0,
        "dry_run": true,
        "authenticated_operator_ref": "TEST_CODE-operator-session",
        "reason": "operator.evidence_invalid",
        "evidence_refs": [],
        "requested_at": 0,
        "command_id": UNIT_COMMAND_ID
    })
}

fn intent_wire_value() -> Value {
    json!({
        "command": "reconcile",
        "target": {
            "kind": "intent",
            "id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        "expected_version": 3,
        "expected_generation": 7,
        "dry_run": false,
        "authenticated_operator_ref": "TEST_CODE-operator-session",
        "reason": "transport.uncertain",
        "evidence_refs": [
            {
                "kind": "external-disposition",
                "version": "1",
                "protected_uri": "vault://TEST_CODE/证据/one",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            {
                "kind": "external-disposition",
                "version": "1",
                "protected_uri": "vault://TEST_CODE/two",
                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            }
        ],
        "requested_at": 1783000000000000_i64,
        "command_id": "f3d8af8ac91486b64b9de91c1cd81e5fdae4a2bf697146d1badd24fe2554e28e"
    })
}

fn parse_value(value: &Value) -> Result<UnverifiedOperatorRequest, OperatorRequestError> {
    UnverifiedOperatorRequest::parse(
        &serde_json::to_vec(value).expect("TEST_CODE serialize request fixture"),
    )
}

#[test]
fn parses_independent_unit_golden_and_exposes_unverified_material() {
    let wire = br#"{
        "command": "inspect",
        "target": {
            "kind": "unit",
            "id": "MU-p01",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        "expected_version": 0,
        "expected_generation": 0,
        "dry_run": true,
        "authenticated_operator_ref": "TEST_CODE-operator-session",
        "reason": "operator.evidence_invalid",
        "evidence_refs": [],
        "requested_at": 0,
        "command_id": "f43e17c300e257c88c6e96a99ca935091b85e0fc910655e9df98891d760dbd1b"
    }"#;

    let request = UnverifiedOperatorRequest::parse(wire).expect("TEST_CODE valid request");

    assert_eq!(request.command(), OperatorCommand::Inspect);
    assert!(matches!(request.target(), OperatorTarget::Unit { .. }));
    assert_eq!(request.expected_version(), 0);
    assert_eq!(request.expected_generation(), 0);
    assert!(request.dry_run());
    assert_eq!(request.claimed_operator_ref(), "TEST_CODE-operator-session");
    assert!(request.evidence_refs().is_empty());
    assert_eq!(request.requested_at().get(), 0);
    assert_eq!(
        request.request_digest().as_str(),
        "f43e17c300e257c88c6e96a99ca935091b85e0fc910655e9df98891d760dbd1b"
    );
    assert_eq!(
        request.canonical_bytes(),
        b"PushOperatorRequest/v1\0{\"authenticated_operator_ref\":\"TEST_CODE-operator-session\",\"command\":\"inspect\",\"dry_run\":true,\"evidence_refs\":[],\"expected_generation\":0,\"expected_version\":0,\"reason\":\"operator.evidence_invalid\",\"requested_at\":0,\"target\":{\"id\":\"MU-p01\",\"kind\":\"unit\",\"namespace\":{\"kind\":\"Test\",\"run_id\":\"TEST_CODE-operator-wire\"}}}"
    );
}

#[test]
fn parses_independent_intent_and_decision_goldens() {
    let intent_wire = r#"{
        "command": "reconcile",
        "target": {
            "kind": "intent",
            "id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        "expected_version": 3,
        "expected_generation": 7,
        "dry_run": false,
        "authenticated_operator_ref": "TEST_CODE-operator-session",
        "reason": "transport.uncertain",
        "evidence_refs": [
            {
                "kind": "external-disposition",
                "version": "1",
                "protected_uri": "vault://TEST_CODE/证据/one",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            {
                "kind": "external-disposition",
                "version": "1",
                "protected_uri": "vault://TEST_CODE/two",
                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            }
        ],
        "requested_at": 1783000000000000,
        "command_id": "f3d8af8ac91486b64b9de91c1cd81e5fdae4a2bf697146d1badd24fe2554e28e"
    }"#;
    let intent = UnverifiedOperatorRequest::parse(intent_wire.as_bytes())
        .expect("TEST_CODE valid intent golden");
    let expected_intent = concat!(
        "PushOperatorRequest/v1",
        "\0",
        r#"{"authenticated_operator_ref":"TEST_CODE-operator-session","command":"reconcile","dry_run":false,"evidence_refs":[{"kind":"external-disposition","protected_uri":"vault://TEST_CODE/证据/one","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":"1"},{"kind":"external-disposition","protected_uri":"vault://TEST_CODE/two","sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","version":"1"}],"expected_generation":7,"expected_version":3,"reason":"transport.uncertain","requested_at":1783000000000000,"target":{"id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","kind":"intent","namespace":{"kind":"Test","run_id":"TEST_CODE-operator-wire"}}}"#
    );
    assert_eq!(intent.canonical_bytes(), expected_intent.as_bytes());
    assert_eq!(intent.canonical_bytes().len(), 739);
    assert_eq!(
        intent.request_digest().as_str(),
        "f3d8af8ac91486b64b9de91c1cd81e5fdae4a2bf697146d1badd24fe2554e28e"
    );
    assert_eq!(intent.reason(), ReasonCode::TransportUncertain);
    assert_eq!(intent.evidence_refs().len(), 2);
    assert_eq!(
        intent.evidence_refs()[0].protected_uri().as_str(),
        "vault://TEST_CODE/证据/one"
    );

    let decision_wire = r#"{
        "command": "resolve-uncertain",
        "target": {
            "kind": "decision",
            "id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        "expected_version": 18446744073709551615,
        "expected_generation": 18446744073709551615,
        "dry_run": false,
        "authenticated_operator_ref": "TEST_CODE/操作员\"A\\B\tC",
        "reason": "operator.not_delivered",
        "evidence_refs": [{
            "kind": "external-disposition",
            "version": "1",
            "protected_uri": "vault://TEST_CODE/证据/one",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }],
        "requested_at": 9223372036854775807,
        "command_id": "ff89fa2bc88b0a6c223243edaeb5fbb9a76cd0843b3d7ba5626addf82524f8c1"
    }"#;
    let decision = UnverifiedOperatorRequest::parse(decision_wire.as_bytes())
        .expect("TEST_CODE valid decision golden");
    let expected_decision = concat!(
        "PushOperatorRequest/v1",
        "\0",
        r#"{"authenticated_operator_ref":"TEST_CODE/操作员\"A\\B\tC","command":"resolve-uncertain","dry_run":false,"evidence_refs":[{"kind":"external-disposition","protected_uri":"vault://TEST_CODE/证据/one","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":"1"}],"expected_generation":18446744073709551615,"expected_version":18446744073709551615,"reason":"operator.not_delivered","requested_at":9223372036854775807,"target":{"id":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","kind":"decision","namespace":{"kind":"Test","run_id":"TEST_CODE-operator-wire"}}}"#
    );
    assert_eq!(decision.canonical_bytes(), expected_decision.as_bytes());
    assert_eq!(decision.canonical_bytes().len(), 633);
    assert_eq!(
        decision.request_digest().as_str(),
        "ff89fa2bc88b0a6c223243edaeb5fbb9a76cd0843b3d7ba5626addf82524f8c1"
    );
    assert_eq!(decision.expected_version(), u64::MAX);
    assert_eq!(decision.expected_generation(), u64::MAX);
    assert_eq!(decision.requested_at().get(), i64::MAX);
}

#[test]
fn object_order_and_whitespace_do_not_change_request_identity() {
    let reordered = format!(
        r#"{{ "command_id":"{UNIT_COMMAND_ID}", "requested_at":0,
        "evidence_refs":[], "reason":"operator.evidence_invalid",
        "authenticated_operator_ref":"TEST_CODE-operator-session", "dry_run":true,
        "expected_generation":0, "expected_version":0,
        "target":{{"namespace":{{"run_id":"TEST_CODE-operator-wire","kind":"Test"}},"id":"MU-p01","kind":"unit"}},
        "command":"inspect" }}"#
    );

    let request = UnverifiedOperatorRequest::parse(reordered.as_bytes())
        .expect("TEST_CODE reordered request");

    assert_eq!(request.request_digest().as_str(), UNIT_COMMAND_ID);
}

#[test]
fn parses_independent_production_null_namespace_golden_as_unverified() {
    let wire = br#"{
        "command":"inspect",
        "target":{"kind":"unit","id":"MU-p01","namespace":{"kind":"Production","run_id":null}},
        "expected_version":0,
        "expected_generation":0,
        "dry_run":true,
        "authenticated_operator_ref":"TEST_CODE-operator-session",
        "reason":"operator.evidence_invalid",
        "evidence_refs":[],
        "requested_at":0,
        "command_id":"4f80bb38823491535dcf9df0646c647df47aa1f95b1f2c4df7b83d077f5bbc99"
    }"#;
    let request =
        UnverifiedOperatorRequest::parse(wire).expect("TEST_CODE Production namespace claim");

    assert!(matches!(
        request.target().namespace(),
        Namespace::Production
    ));
    assert_eq!(request.canonical_bytes().len(), 322);
    assert_eq!(
        request.canonical_bytes(),
        b"PushOperatorRequest/v1\0{\"authenticated_operator_ref\":\"TEST_CODE-operator-session\",\"command\":\"inspect\",\"dry_run\":true,\"evidence_refs\":[],\"expected_generation\":0,\"expected_version\":0,\"reason\":\"operator.evidence_invalid\",\"requested_at\":0,\"target\":{\"id\":\"MU-p01\",\"kind\":\"unit\",\"namespace\":{\"kind\":\"Production\",\"run_id\":null}}}"
    );
    assert_eq!(
        request.request_digest().as_str(),
        "4f80bb38823491535dcf9df0646c647df47aa1f95b1f2c4df7b83d077f5bbc99"
    );
}

#[test]
fn every_material_field_changes_identity_but_command_id_is_not_self_material() {
    let mut mutations = Vec::new();

    let mut changed = unit_wire_value();
    changed["command"] = json!("promote");
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["target"]["id"] = json!("MU-p02");
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["target"]["namespace"]["run_id"] = json!("TEST_CODE-other-run");
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["target"]["namespace"] = json!({"kind": "Production", "run_id": null});
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["expected_version"] = json!(1);
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["expected_generation"] = json!(1);
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["dry_run"] = json!(false);
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["authenticated_operator_ref"] = json!("TEST_CODE-other-operator");
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["reason"] = json!("operator.not_delivered");
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["evidence_refs"] = json!([{
        "kind": "external-disposition",
        "version": "1",
        "protected_uri": "vault://TEST_CODE/evidence",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }]);
    mutations.push(changed);

    let mut changed = unit_wire_value();
    changed["requested_at"] = json!(1);
    mutations.push(changed);

    for changed in mutations {
        assert_eq!(
            parse_value(&changed),
            Err(OperatorRequestError::CommandIdMismatch)
        );
    }

    let mut changed_id_only = unit_wire_value();
    changed_id_only["command_id"] =
        json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(
        parse_value(&changed_id_only),
        Err(OperatorRequestError::CommandIdMismatch)
    );
}

#[test]
fn evidence_array_order_is_identity_material() {
    let wire = r#"{
        "command_id":"f3d8af8ac91486b64b9de91c1cd81e5fdae4a2bf697146d1badd24fe2554e28e",
        "command":"reconcile",
        "target":{"kind":"intent","id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","namespace":{"kind":"Test","run_id":"TEST_CODE-operator-wire"}},
        "expected_version":3,"expected_generation":7,"dry_run":false,
        "authenticated_operator_ref":"TEST_CODE-operator-session","reason":"transport.uncertain",
        "evidence_refs":[
            {"kind":"external-disposition","version":"1","protected_uri":"vault://TEST_CODE/two","sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"},
            {"kind":"external-disposition","version":"1","protected_uri":"vault://TEST_CODE/证据/one","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        ],
        "requested_at":1783000000000000
    }"#;

    assert_eq!(
        UnverifiedOperatorRequest::parse(wire.as_bytes()),
        Err(OperatorRequestError::CommandIdMismatch)
    );
}

#[test]
fn every_evidence_field_is_request_identity_material() {
    for (pointer, value) in [
        ("/evidence_refs/0/kind", json!("external-disposition-v2")),
        ("/evidence_refs/0/version", json!("2")),
        (
            "/evidence_refs/0/protected_uri",
            json!("vault://TEST_CODE/other"),
        ),
        (
            "/evidence_refs/0/sha256",
            json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ),
    ] {
        let mut changed = intent_wire_value();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            parse_value(&changed),
            Err(OperatorRequestError::CommandIdMismatch)
        );
    }
}

#[test]
fn rejects_every_missing_top_level_field_and_unknown_or_trailing_data() {
    let fields = [
        "command_id",
        "command",
        "target",
        "expected_version",
        "expected_generation",
        "dry_run",
        "authenticated_operator_ref",
        "reason",
        "evidence_refs",
        "requested_at",
    ];
    for field in fields {
        let mut missing = unit_wire_value();
        missing
            .as_object_mut()
            .expect("TEST_CODE request object")
            .remove(field);
        assert_eq!(
            parse_value(&missing),
            Err(OperatorRequestError::WireStructure)
        );
    }

    let mut extra = unit_wire_value();
    extra["extra"] = json!(true);
    assert_eq!(
        parse_value(&extra),
        Err(OperatorRequestError::WireStructure)
    );

    let duplicated = format!(
        r#"{{"command":"inspect","command":"inspect","target":{{"kind":"unit","id":"MU-p01","namespace":{{"kind":"Test","run_id":"TEST_CODE-operator-wire"}}}},"expected_version":0,"expected_generation":0,"dry_run":true,"authenticated_operator_ref":"TEST_CODE-operator-session","reason":"operator.evidence_invalid","evidence_refs":[],"requested_at":0,"command_id":"{UNIT_COMMAND_ID}"}}"#
    );
    assert_eq!(
        UnverifiedOperatorRequest::parse(duplicated.as_bytes()),
        Err(OperatorRequestError::WireStructure)
    );

    let trailing = format!(
        "{} {{}}",
        serde_json::to_string(&unit_wire_value()).unwrap()
    );
    assert_eq!(
        UnverifiedOperatorRequest::parse(trailing.as_bytes()),
        Err(OperatorRequestError::WireStructure)
    );
    assert_eq!(
        UnverifiedOperatorRequest::parse(b"{\"command\":\"\xff\"}"),
        Err(OperatorRequestError::WireStructure)
    );
}

#[test]
fn rejects_missing_extra_and_duplicate_nested_fields() {
    for field in ["kind", "id", "namespace"] {
        let mut missing = unit_wire_value();
        missing["target"]
            .as_object_mut()
            .expect("TEST_CODE target object")
            .remove(field);
        assert_eq!(
            parse_value(&missing),
            Err(OperatorRequestError::WireStructure)
        );
    }
    for field in ["kind", "run_id"] {
        let mut missing = unit_wire_value();
        missing["target"]["namespace"]
            .as_object_mut()
            .expect("TEST_CODE namespace object")
            .remove(field);
        assert_eq!(
            parse_value(&missing),
            Err(OperatorRequestError::WireStructure)
        );
    }

    let evidence = json!({
        "kind": "external-disposition",
        "version": "1",
        "protected_uri": "vault://TEST_CODE/evidence",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    for field in ["kind", "version", "protected_uri", "sha256"] {
        let mut missing = unit_wire_value();
        let mut item = evidence.clone();
        item.as_object_mut().unwrap().remove(field);
        missing["evidence_refs"] = json!([item]);
        assert_eq!(
            parse_value(&missing),
            Err(OperatorRequestError::WireStructure)
        );
    }

    for pointer in ["/target", "/target/namespace"] {
        let mut extra = unit_wire_value();
        extra.pointer_mut(pointer).unwrap()["extra"] = json!(true);
        assert_eq!(
            parse_value(&extra),
            Err(OperatorRequestError::WireStructure)
        );
    }
    let mut extra_evidence = unit_wire_value();
    let mut item = evidence;
    item["extra"] = json!(true);
    extra_evidence["evidence_refs"] = json!([item]);
    assert_eq!(
        parse_value(&extra_evidence),
        Err(OperatorRequestError::WireStructure)
    );

    let nested_duplicates = [
        r#"{"kind":"unit","id":"MU-p01","id":"MU-p01","namespace":{"kind":"Test","run_id":"TEST_CODE-operator-wire"}}"#,
        r#"{"kind":"unit","id":"MU-p01","namespace":{"kind":"Test","kind":"Test","run_id":"TEST_CODE-operator-wire"}}"#,
    ];
    for target in nested_duplicates {
        let wire = format!(
            r#"{{"command":"inspect","target":{target},"expected_version":0,"expected_generation":0,"dry_run":true,"authenticated_operator_ref":"TEST_CODE-operator-session","reason":"operator.evidence_invalid","evidence_refs":[],"requested_at":0,"command_id":"{UNIT_COMMAND_ID}"}}"#
        );
        assert_eq!(
            UnverifiedOperatorRequest::parse(wire.as_bytes()),
            Err(OperatorRequestError::WireStructure)
        );
    }
    let duplicate_evidence_field = format!(
        r#"{{"command":"inspect","target":{{"kind":"unit","id":"MU-p01","namespace":{{"kind":"Test","run_id":"TEST_CODE-operator-wire"}}}},"expected_version":0,"expected_generation":0,"dry_run":true,"authenticated_operator_ref":"TEST_CODE-operator-session","reason":"operator.evidence_invalid","evidence_refs":[{{"kind":"external","kind":"external","version":"1","protected_uri":"vault://TEST_CODE/evidence","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}],"requested_at":0,"command_id":"{UNIT_COMMAND_ID}"}}"#
    );
    assert_eq!(
        UnverifiedOperatorRequest::parse(duplicate_evidence_field.as_bytes()),
        Err(OperatorRequestError::WireStructure)
    );
}

#[test]
fn rejects_array_encoding_for_structs_and_object_encoding_for_command() {
    let top_level_sequence = json!([
        UNIT_COMMAND_ID,
        "inspect",
        {
            "kind": "unit",
            "id": "MU-p01",
            "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
        },
        0,
        0,
        true,
        "TEST_CODE-operator-session",
        "operator.evidence_invalid",
        [],
        0
    ]);
    assert_eq!(
        parse_value(&top_level_sequence),
        Err(OperatorRequestError::WireStructure)
    );

    let mut evidence_sequence = unit_wire_value();
    evidence_sequence["evidence_refs"] = json!([[
        "external",
        "1",
        "vault://TEST_CODE/evidence",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ]]);
    assert_eq!(
        parse_value(&evidence_sequence),
        Err(OperatorRequestError::WireStructure)
    );

    let mut target_sequence = unit_wire_value();
    target_sequence["target"] = json!([
        "unit",
        "MU-p01",
        {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
    ]);
    assert_eq!(
        parse_value(&target_sequence),
        Err(OperatorRequestError::WireStructure)
    );

    let mut namespace_sequence = unit_wire_value();
    namespace_sequence["target"]["namespace"] = json!(["Test", "TEST_CODE-operator-wire"]);
    assert_eq!(
        parse_value(&namespace_sequence),
        Err(OperatorRequestError::WireStructure)
    );

    let mut production_namespace_sequence = unit_wire_value();
    production_namespace_sequence["target"]["namespace"] = json!(["Production", null]);
    assert_eq!(
        parse_value(&production_namespace_sequence),
        Err(OperatorRequestError::WireStructure)
    );

    let mut object_command = unit_wire_value();
    object_command["command"] = json!({"inspect": null});
    assert_eq!(
        parse_value(&object_command),
        Err(OperatorRequestError::WireStructure)
    );
}

#[test]
fn rejects_wrong_types_invalid_numbers_and_invalid_closed_values() {
    let wrong_types = [
        ("/command", json!(7)),
        ("/target", json!("unit")),
        ("/expected_version", json!("0")),
        ("/expected_generation", json!(false)),
        ("/dry_run", json!(0)),
        ("/authenticated_operator_ref", json!(null)),
        ("/reason", json!(["operator.evidence_invalid"])),
        ("/evidence_refs", json!({})),
        ("/requested_at", json!(true)),
        ("/command_id", json!(null)),
    ];
    for (pointer, value) in wrong_types {
        let mut wrong = unit_wire_value();
        *wrong.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            parse_value(&wrong),
            Err(OperatorRequestError::WireStructure)
        );
    }

    for (pointer, number) in [
        ("/expected_version", "-1"),
        ("/expected_version", "1.5"),
        ("/expected_version", "18446744073709551616"),
        ("/expected_generation", "-1"),
        ("/expected_generation", "1.5"),
        ("/expected_generation", "18446744073709551616"),
        ("/requested_at", "-1"),
        ("/requested_at", "1.5"),
        ("/requested_at", "9223372036854775808"),
    ] {
        let mut wire = serde_json::to_string(&unit_wire_value()).unwrap();
        let old = match pointer {
            "/expected_version" => "\"expected_version\":0",
            "/expected_generation" => "\"expected_generation\":0",
            "/requested_at" => "\"requested_at\":0",
            _ => unreachable!(),
        };
        let field = &old[..old.len() - 1];
        wire = wire.replacen(old, &format!("{field}{number}"), 1);
        let expected = if pointer == "/requested_at" && number == "-1" {
            OperatorRequestError::InvalidField("requested_at")
        } else {
            OperatorRequestError::WireStructure
        };
        assert_eq!(
            UnverifiedOperatorRequest::parse(wire.as_bytes()),
            Err(expected)
        );
    }

    for (pointer, value, expected) in [
        (
            "/command",
            json!("delete"),
            OperatorRequestError::InvalidField("command"),
        ),
        (
            "/target/kind",
            json!("occurrence"),
            OperatorRequestError::InvalidField("target.kind"),
        ),
        (
            "/reason",
            json!("operator.unknown"),
            OperatorRequestError::InvalidField("reason"),
        ),
        (
            "/target/id",
            json!("ABCDEF"),
            OperatorRequestError::CommandIdMismatch,
        ),
        (
            "/command_id",
            json!("ABCDEF"),
            OperatorRequestError::InvalidField("command_id"),
        ),
    ] {
        let mut wrong = unit_wire_value();
        *wrong.pointer_mut(pointer).unwrap() = value;
        assert_eq!(parse_value(&wrong), Err(expected));
    }

    let mut intent_bad_digest = unit_wire_value();
    intent_bad_digest["target"] = json!({
        "kind": "intent",
        "id": "ABCDEF",
        "namespace": {"kind": "Test", "run_id": "TEST_CODE-operator-wire"}
    });
    assert_eq!(
        parse_value(&intent_bad_digest),
        Err(OperatorRequestError::InvalidField("target.id"))
    );
}

#[test]
fn enforces_namespace_shape_and_command_target_matrix() {
    for namespace in [
        json!({"kind": "Production"}),
        json!({"kind": "Production", "run_id": "TEST_CODE-run"}),
        json!({"kind": "Test"}),
        json!({"kind": "Test", "run_id": null}),
    ] {
        let mut wrong = unit_wire_value();
        wrong["target"]["namespace"] = namespace;
        assert_eq!(
            parse_value(&wrong),
            Err(OperatorRequestError::WireStructure)
        );
    }

    let mut unknown_namespace = unit_wire_value();
    unknown_namespace["target"]["namespace"] = json!({"kind": "Unknown", "run_id": null});
    assert_eq!(
        parse_value(&unknown_namespace),
        Err(OperatorRequestError::InvalidField("target.namespace.kind"))
    );

    let digest = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    for (command, kind, id) in [
        ("reconcile", "decision", digest),
        ("resolve-uncertain", "unit", "MU-p01"),
        ("resolve-uncertain", "intent", digest),
        ("promote", "intent", digest),
        ("promote", "decision", digest),
        ("rollback", "intent", digest),
        ("rollback", "decision", digest),
    ] {
        let mut wrong = unit_wire_value();
        wrong["command"] = json!(command);
        wrong["target"]["kind"] = json!(kind);
        wrong["target"]["id"] = json!(id);
        assert_eq!(
            parse_value(&wrong),
            Err(OperatorRequestError::TargetCommandMismatch)
        );
    }

    for (command, kind, id, command_id) in [
        (
            "inspect",
            "intent",
            digest,
            "1a5356fd2280ed8ea43187144b2fa9e5f9f9b6a5613c85212c466bdd7283aae7",
        ),
        (
            "inspect",
            "decision",
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "423892959d8885615f6707a01eec52ebf49a3ab7d355a2cb08f7c0b82f92b6a6",
        ),
        (
            "reconcile",
            "unit",
            "MU-p01",
            "b64d2fd60470de9a11b31292a30744e4a35f7b3e6ae7256867940a3e08ad458e",
        ),
        (
            "promote",
            "unit",
            "MU-p01",
            "8476aa417e2bdf475db32ef346be7f19bdc7100183fe98230ef1804f48726f16",
        ),
        (
            "rollback",
            "unit",
            "MU-p01",
            "f2ee3ebab08c4047187fb598c1d49a08f8ae2ccfd0e20dacf663eae5a962f64b",
        ),
    ] {
        let mut accepted_shape = unit_wire_value();
        accepted_shape["command"] = json!(command);
        accepted_shape["target"]["kind"] = json!(kind);
        accepted_shape["target"]["id"] = json!(id);
        accepted_shape["command_id"] = json!(command_id);
        parse_value(&accepted_shape).expect("TEST_CODE allowed command-target pair");
    }
}

#[test]
fn rejects_text_digest_evidence_and_input_limits() {
    for pointer in [
        "/authenticated_operator_ref",
        "/target/id",
        "/target/namespace/run_id",
    ] {
        for invalid in ["", " padded", "padded ", "nul\0text"] {
            let mut wrong = unit_wire_value();
            *wrong.pointer_mut(pointer).unwrap() = json!(invalid);
            assert!(matches!(
                parse_value(&wrong),
                Err(OperatorRequestError::InvalidField(_))
            ));
        }
        let mut wrong = unit_wire_value();
        *wrong.pointer_mut(pointer).unwrap() = json!("x".repeat(513));
        assert!(matches!(
            parse_value(&wrong),
            Err(OperatorRequestError::InvalidField(_))
        ));
    }

    for field in ["kind", "version", "protected_uri"] {
        for invalid in ["", " padded", "padded ", "nul\0text"] {
            let mut wrong = unit_wire_value();
            wrong["evidence_refs"] = json!([{
                "kind": "external",
                "version": "1",
                "protected_uri": "vault://TEST_CODE/evidence",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }]);
            wrong["evidence_refs"][0][field] = json!(invalid);
            assert!(matches!(
                parse_value(&wrong),
                Err(OperatorRequestError::InvalidField(_))
            ));
        }
        let mut too_long = unit_wire_value();
        too_long["evidence_refs"] = json!([{
            "kind": "external",
            "version": "1",
            "protected_uri": "vault://TEST_CODE/evidence",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }]);
        too_long["evidence_refs"][0][field] = json!("界".repeat(171));
        assert!(matches!(
            parse_value(&too_long),
            Err(OperatorRequestError::InvalidField(_))
        ));
    }

    for invalid_sha in [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let mut wrong = unit_wire_value();
        wrong["evidence_refs"] = json!([{
            "kind": "external",
            "version": "1",
            "protected_uri": "vault://TEST_CODE/evidence",
            "sha256": invalid_sha
        }]);
        assert_eq!(
            parse_value(&wrong),
            Err(OperatorRequestError::InvalidField("evidence_refs.sha256"))
        );
    }

    let mut duplicate = unit_wire_value();
    let evidence = json!({
        "kind": "external",
        "version": "1",
        "protected_uri": "vault://TEST_CODE/evidence",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    duplicate["evidence_refs"] = json!([evidence.clone(), evidence]);
    assert_eq!(
        parse_value(&duplicate),
        Err(OperatorRequestError::DuplicateEvidence)
    );

    let mut too_many = unit_wire_value();
    too_many["evidence_refs"] = Value::Array(
        (0..65)
            .map(|index| {
                json!({
                    "kind": "external",
                    "version": "1",
                    "protected_uri": format!("vault://TEST_CODE/{index}"),
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                })
            })
            .collect(),
    );
    assert_eq!(
        parse_value(&too_many),
        Err(OperatorRequestError::TooManyEvidenceRefs)
    );

    let mut exact_input_limit = serde_json::to_vec(&unit_wire_value()).unwrap();
    exact_input_limit.resize(64 * 1024, b' ');
    UnverifiedOperatorRequest::parse(&exact_input_limit)
        .expect("TEST_CODE exactly 64 KiB padded JSON");

    assert_eq!(
        UnverifiedOperatorRequest::parse(&[exact_input_limit, vec![b' '; 1]].concat()),
        Err(OperatorRequestError::InputTooLarge)
    );
}

#[test]
fn accepts_exact_evidence_and_utf8_text_limits() {
    let mut exact_evidence = unit_wire_value();
    exact_evidence["evidence_refs"] = Value::Array(
        (0..64)
            .map(|index| {
                json!({
                    "kind": "external",
                    "version": "1",
                    "protected_uri": format!("vault://TEST_CODE/{index}"),
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                })
            })
            .collect(),
    );
    exact_evidence["command_id"] =
        json!("f0e75fe888a6b0377ac5ad693cfd43103aba216ef10ee9d70b05b844f49098d6");
    assert_eq!(
        parse_value(&exact_evidence)
            .expect("TEST_CODE exactly 64 evidence refs")
            .evidence_refs()
            .len(),
        64
    );

    let exact_text = "界".repeat(170) + "aa";
    assert_eq!(exact_text.len(), 512);
    let mut cases = Vec::new();

    let mut value = unit_wire_value();
    value["authenticated_operator_ref"] = json!(exact_text.clone());
    value["command_id"] = json!("d4fe50b0eed70ef41b0d73b144195005559d1a55b4138c2b20f48100be9ba81a");
    cases.push(value);

    let mut value = unit_wire_value();
    value["target"]["namespace"]["run_id"] = json!(exact_text.clone());
    value["command_id"] = json!("e83b4392608bdb7caa1950f57c69f14529b2f9f2c1e0139d53418093884d25a8");
    cases.push(value);

    let mut value = unit_wire_value();
    value["target"]["id"] = json!(exact_text.clone());
    value["command_id"] = json!("860d41787deb653b4f8679e89f0af8bdce941d05e05a58f67ae31d581420cc85");
    cases.push(value);

    for (field, command_id) in [
        (
            "kind",
            "a753eebde602ea97d5f58549f129f298a23e3b118db12b99d7843cd0991bcfd5",
        ),
        (
            "version",
            "4a5c07eccf15ff3ddda9968e570d83800e8392a8328cf6d6719728714ef8febe",
        ),
        (
            "protected_uri",
            "17fd31bd65b7eda370e9358c6010b7139f3330642c6a2f6e43924af48c0e1194",
        ),
    ] {
        let mut value = unit_wire_value();
        value["evidence_refs"] = json!([{
            "kind": "external",
            "version": "1",
            "protected_uri": "vault://TEST_CODE/evidence",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }]);
        value["evidence_refs"][0][field] = json!(exact_text.clone());
        value["command_id"] = json!(command_id);
        cases.push(value);
    }

    for value in cases {
        parse_value(&value).expect("TEST_CODE exactly 512-byte UTF-8 text");
    }
}

#[test]
fn debug_and_errors_do_not_disclose_operator_uri_or_target_sentinels() {
    let intent_wire = r#"{
        "command":"reconcile",
        "target":{"kind":"unit","id":"TEST_CODE-SECRET-UNIT/PATH","namespace":{"kind":"Test","run_id":"TEST_CODE-SECRET-RUN"}},
        "expected_version":0,"expected_generation":0,"dry_run":true,
        "authenticated_operator_ref":"TEST_CODE-SECRET-OPERATOR","reason":"operator.evidence_invalid",
        "evidence_refs":[{"kind":"TEST_CODE-SECRET-KIND","version":"TEST_CODE-SECRET-VERSION","protected_uri":"vault://TEST_CODE-SECRET/PATH","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],
        "requested_at":0,
        "command_id":"de844a6e2d3ad7cd0d0be85e955f8a2f486c86885fdc7b866c7318c29eaa398d"
    }"#;
    let request = UnverifiedOperatorRequest::parse(intent_wire.as_bytes())
        .expect("TEST_CODE valid redaction fixture");

    for rendered in [
        format!("{request:?}"),
        format!("{:?}", request.target()),
        format!("{:?}", request.evidence_refs()[0]),
    ] {
        assert!(!rendered.contains("TEST_CODE-SECRET"), "{rendered}");
        assert!(!rendered.contains("vault://"), "{rendered}");
    }

    let assert_redacted = |wire: &[u8], expected: OperatorRequestError| {
        let error = UnverifiedOperatorRequest::parse(wire).unwrap_err();
        assert_eq!(error, expected);
        for rendered in [format!("{error}"), format!("{error:?}")] {
            for sensitive in [
                "TEST_CODE-SECRET",
                "vault://",
                "/PATH",
                "\"authenticated_operator_ref\"",
            ] {
                assert!(!rendered.contains(sensitive), "{rendered}");
            }
        }
    };

    let malformed = intent_wire.replace("\"expected_version\":0", "\"expected_version\":null");
    assert_redacted(malformed.as_bytes(), OperatorRequestError::WireStructure);

    let fixture: Value = serde_json::from_str(intent_wire).expect("TEST_CODE secret fixture JSON");

    let mut invalid_field = fixture.clone();
    invalid_field["reason"] = json!("TEST_CODE-SECRET-REASON");
    assert_redacted(
        &serde_json::to_vec(&invalid_field).unwrap(),
        OperatorRequestError::InvalidField("reason"),
    );

    let mut target_mismatch = fixture.clone();
    target_mismatch["command"] = json!("resolve-uncertain");
    assert_redacted(
        &serde_json::to_vec(&target_mismatch).unwrap(),
        OperatorRequestError::TargetCommandMismatch,
    );

    let mut duplicate_evidence = fixture.clone();
    let evidence = fixture["evidence_refs"][0].clone();
    duplicate_evidence["evidence_refs"] = json!([evidence.clone(), evidence]);
    assert_redacted(
        &serde_json::to_vec(&duplicate_evidence).unwrap(),
        OperatorRequestError::DuplicateEvidence,
    );

    let mut identity_mismatch = fixture.clone();
    identity_mismatch["requested_at"] = json!(1);
    assert_redacted(
        &serde_json::to_vec(&identity_mismatch).unwrap(),
        OperatorRequestError::CommandIdMismatch,
    );

    let mut too_many_evidence = fixture.clone();
    too_many_evidence["evidence_refs"] = Value::Array(
        (0..65)
            .map(|_| fixture["evidence_refs"][0].clone())
            .collect(),
    );
    assert_redacted(
        &serde_json::to_vec(&too_many_evidence).unwrap(),
        OperatorRequestError::TooManyEvidenceRefs,
    );

    let mut too_large = intent_wire.as_bytes().to_vec();
    too_large.resize(64 * 1024 + 1, b' ');
    assert_redacted(&too_large, OperatorRequestError::InputTooLarge);
}

#[test]
fn parsed_namespace_is_a_claim_not_authority() {
    let request = parse_value(&unit_wire_value()).expect("TEST_CODE valid unverified request");
    assert!(matches!(
        request.target().namespace(),
        Namespace::Test { run_id } if run_id.as_str() == "TEST_CODE-operator-wire"
    ));
    assert_eq!(request.claimed_operator_ref(), "TEST_CODE-operator-session");
}
