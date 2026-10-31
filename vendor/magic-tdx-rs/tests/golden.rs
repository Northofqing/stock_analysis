use serde_json::Value;
use std::fs;
#[test]
fn security_count_fixture_is_stable() {
    let bytes = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/protocol/security_count_one.bin"
    ))
    .unwrap();
    assert_eq!(&bytes[..2], [1, 0]);
    assert_eq!(
        magic_tdx_rs::protocol::parsers::parse_security_count(&bytes).unwrap(),
        1
    );
}

#[test]
fn fixture_manifest_is_complete() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
    let manifest: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(format!("{root}manifest.json")).unwrap()).unwrap();
    assert!(!manifest.is_empty());
    for entry in manifest {
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .expect("fixture path");
        let operation = entry
            .get("operation")
            .and_then(Value::as_str)
            .expect("fixture operation");
        assert!(!operation.is_empty());
        assert!(!fs::read(format!("{root}{path}")).unwrap().is_empty());
    }
}
