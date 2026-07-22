use super::*;

#[derive(serde::Serialize)]
struct DefaultContractEvent {
    event_type: &'static str,
    value: u8,
}

impl DomainEvent for DefaultContractEvent {
    fn event_type(&self) -> &'static str {
        self.event_type
    }

    fn source(&self) -> &'static str {
        "gate_d_regression"
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::json!({"value": self.value})
    }
}

#[test]
fn domain_defaults_and_all_envelope_identity_guards_are_exercised() {
    let now = chrono::Local::now();
    let event = DefaultContractEvent {
        event_type: "gate.d.default",
        value: 7,
    };
    let envelope = EventEnvelope::from_event(&event, "id-1".into(), "trace-1".into(), now)
        .expect("default entity and validation hooks must permit a valid event");
    assert_eq!(envelope.entity_key, None);
    assert_eq!(envelope.payload["value"], 7);

    assert!(matches!(
        EventEnvelope::from_event(&event, " ".into(), "trace".into(), now),
        Err(EnvelopeError::BlankId)
    ));
    assert!(matches!(
        EventEnvelope::from_event(&event, "id".into(), " ".into(), now),
        Err(EnvelopeError::BlankTraceId)
    ));
    let blank_type = DefaultContractEvent {
        event_type: " ",
        value: 0,
    };
    assert!(matches!(
        EventEnvelope::from_event(&blank_type, "id".into(), "trace".into(), now),
        Err(EnvelopeError::BlankEventType)
    ));
}

#[test]
fn push_delivery_trait_payload_matches_the_serialized_contract() {
    let event = PushDeliveryEvent::new(
        "gate_d".into(),
        Some("TEST_CODE_000001".into()),
        "Pushed".into(),
        "test".into(),
        12,
        3,
    );
    let payload = DomainEvent::payload(&event);
    assert_eq!(payload["kind"], "gate_d");
    assert!(payload.get("code").is_none());
    assert_eq!(payload["audit_schema_version"], 2);
    assert_eq!(payload["identity_hash"].as_str().unwrap().len(), 64);
}
