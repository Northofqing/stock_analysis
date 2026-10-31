use super::*;

#[test]
fn security_type_covers_every_documented_market_family() {
    let cases = [
        (1, "600000", 1),
        (1, "688001", 1),
        (1, "900901", 2),
        (1, "519001", 5),
        (1, "500001", 3),
        (1, "510001", 3),
        (1, "580001", 3),
        (1, "110001", 4),
        (1, "130001", 4),
        (1, "000001", 0),
        (0, "399001", 0),
        (0, "000001", 1),
        (0, "300001", 1),
        (0, "200001", 2),
        (0, "150001", 3),
        (0, "160001", 3),
        (0, "100001", 4),
        (0, "120001", 4),
        (0, "130001", 4),
        (2, "920001", 1),
        (1, "700001", 1),
    ];
    for (market, code, expected) in cases {
        assert_eq!(get_security_type(market, code), expected, "{market}:{code}");
    }
}

#[test]
fn coefficient_matches_each_security_type() {
    let cases = [
        (1, "000001", 0.01),
        (1, "600001", 0.01),
        (1, "900901", 0.001),
        (1, "510001", 0.001),
        (1, "110001", 0.0001),
        (1, "519001", 0.00001),
    ];
    for (market, code, expected) in cases {
        assert_eq!(get_security_coefficient(market, code), expected);
    }
}
