use super::{
    derive_occurrence_id, BusinessDate, OccurrenceFamily, OccurrenceIdentityMaterial,
    OccurrenceKey,
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
