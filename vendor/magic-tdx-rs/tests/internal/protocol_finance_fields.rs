use super::*;

#[test]
fn test_extract_indicators_empty() {
    let fields = vec![0.0f32; 584];
    let result = extract_indicators(&fields);
    assert_eq!(result.len(), INDICATORS.len());
    assert!((result["eps"] - 0.0).abs() < 1e-10);
}

#[test]
fn test_extract_indicators_values() {
    let mut fields = vec![0.0f32; 584];
    fields[0] = 1.5; // idx 1 = eps
    fields[5] = 12.5; // idx 6 = roe_diluted
    let result = extract_indicators(&fields);
    assert!((result["eps"] - 1.5).abs() < 1e-10);
    assert!((result["roe_diluted"] - 12.5).abs() < 1e-10);
}

#[test]
fn test_validate_fields_len() {
    assert!(validate_fields_len(584));
    assert!(!validate_fields_len(100));
}
