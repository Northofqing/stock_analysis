use super::{
    derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id, AudienceId,
    BusinessDate, CalendarId, CompletionOwnerId, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, RunId,
    ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest, SourceContractId,
    SourceContractVersion, SubjectId, UnitId, UtcMicros,
};

fn occurrence_material() -> OccurrenceIdentityMaterial {
    OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-06").expect("valid date"),
        OccurrenceFamily::try_new("daily".to_owned()).expect("valid family"),
        OccurrenceKey::try_new("close".to_owned()).expect("valid key"),
    )
}

#[test]
fn w01_occurrence_golden_hash_is_stable() {
    let id = derive_occurrence_id(&occurrence_material());
    assert_eq!(
        id.as_str(),
        "5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a"
    );
}

#[test]
fn w01_identity_value_types_reject_invalid_input() {
    assert!(UnitId::try_new(String::new()).is_err());
    assert!(ProducerId::try_new(" value".to_owned()).is_err());
    assert!(AudienceId::try_new("value\0hidden".to_owned()).is_err());
    assert!(BusinessDate::parse("2026-9-6").is_err());
    assert!(Sha256Digest::parse("payload", "ABC").is_err());
    assert!(UtcMicros::try_new(-1).is_err());
}

#[test]
fn w01_test_namespace_and_subject_are_validated() {
    let one = Namespace::test(RunId::try_new("run-1".to_owned()).expect("valid run"));
    let two = Namespace::test(RunId::try_new("run-2".to_owned()).expect("valid run"));
    assert_ne!(one, two);
    assert_ne!(one, Namespace::Production);

    let entity = SubjectId::entity("000001.SZ".to_owned()).expect("valid subject");
    assert!(matches!(entity, SubjectId::Entity(ref value) if value.as_str() == "000001.SZ"));
    assert!(SubjectId::entity(" bad-subject".to_owned()).is_err());

    assert_eq!(
        SourceContractVersion::try_new("source-v1".to_owned())
            .expect("valid version")
            .as_str(),
        "source-v1"
    );
}

#[test]
fn w01_outer_identities_bind_source_contract_without_changing_raw_occurrence() {
    let occurrence = occurrence_material();
    let raw_id = derive_occurrence_id(&occurrence);
    let schedule = |source: &str| {
        derive_schedule_occurrence_id(&ScheduleOccurrenceIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-close".to_owned()).expect("valid unit"),
            ProducerId::try_new("close-scheduled".to_owned()).expect("valid producer"),
            ScheduleOrTriggerId::try_new("schedule-close".to_owned()).expect("valid schedule"),
            CalendarId::try_new("a-share-calendar".to_owned()).expect("valid calendar"),
            occurrence.clone(),
            CompletionOwnerId::try_new("owner-close".to_owned()).expect("valid owner"),
            SourceContractId::try_new(source.to_owned()).expect("valid source"),
        ))
    };
    let intent = |source: &str| {
        derive_intent_id(&IntentIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-close".to_owned()).expect("valid unit"),
            CompletionOwnerId::try_new("owner-close".to_owned()).expect("valid owner"),
            SourceContractId::try_new(source.to_owned()).expect("valid source"),
            raw_id.clone(),
            SubjectId::Global,
            AudienceId::try_new("portfolio-owner".to_owned()).expect("valid audience"),
        ))
    };

    assert_ne!(schedule("close-v1"), schedule("close-v2"));
    assert_ne!(intent("close-v1"), intent("close-v2"));
    assert_eq!(raw_id, derive_occurrence_id(&occurrence));
}
