use super::*;
use crate::protocol::types::XdXrInfo;

fn make_xdxr(year: u32, month: u32, day: u32, category: u32) -> XdXrInfo {
    XdXrInfo {
        year,
        month,
        day,
        category,
        name: String::new(),
        fenhong: Some(1.0),
        songzhuangu: Some(0.0),
        peigu: Some(0.0),
        peigujia: Some(0.0),
        suogu: None,
        panqianliutong: None,
        panhouliutong: None,
        qianzongguben: None,
        houzongguben: None,
        fenshu: None,
        xingquanjia: None,
    }
}

#[test]
fn test_auto_detect_tier_empty() {
    let xdxr: Vec<XdXrInfo> = Vec::new();
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Mid);
}

#[test]
fn test_auto_detect_tier_low() {
    // 上市 5 年
    let xdxr = vec![make_xdxr(2021, 7, 1, 1)];
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Low);
}

#[test]
fn test_auto_detect_tier_mid() {
    // 上市 15 年
    let xdxr = vec![make_xdxr(2011, 7, 1, 1)];
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Mid);
}

#[test]
fn test_auto_detect_tier_high() {
    // 上市 25 年
    let xdxr = vec![make_xdxr(2001, 7, 1, 1)];
    assert_eq!(
        FqService::auto_detect_tier(&xdxr, 2026),
        FqContextTier::High
    );
}

#[test]
fn test_auto_detect_tier_boundary_10() {
    // 恰好 10 年 → Low
    let xdxr = vec![make_xdxr(2016, 7, 1, 1)];
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Low);
}

#[test]
fn test_auto_detect_tier_boundary_11() {
    // 11 年 → Mid
    let xdxr = vec![make_xdxr(2015, 7, 1, 1)];
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Mid);
}

#[test]
fn test_auto_detect_tier_boundary_20() {
    // 恰好 20 年 → Mid
    let xdxr = vec![make_xdxr(2006, 7, 1, 1)];
    assert_eq!(FqService::auto_detect_tier(&xdxr, 2026), FqContextTier::Mid);
}

#[test]
fn test_auto_detect_tier_boundary_21() {
    // 21 年 → High
    let xdxr = vec![make_xdxr(2005, 7, 1, 1)];
    assert_eq!(
        FqService::auto_detect_tier(&xdxr, 2026),
        FqContextTier::High
    );
}

#[test]
fn test_auto_detect_tier_uses_first_record() {
    // 多条记录，使用第一条 (最早)
    let xdxr = vec![
        make_xdxr(2003, 11, 18, 5), // 最早 (category=5)
        make_xdxr(2004, 6, 1, 1),   // 分红
        make_xdxr(2025, 7, 1, 1),   // 最近
    ];
    assert_eq!(
        FqService::auto_detect_tier(&xdxr, 2026),
        FqContextTier::High
    );
}

#[test]
fn test_fq_type_from_u8() {
    assert_eq!(FqService::fq_type_from_u8(0), FqType::None);
    assert_eq!(FqService::fq_type_from_u8(1), FqType::Qfq);
    assert_eq!(FqService::fq_type_from_u8(2), FqType::Hfq);
    assert_eq!(FqService::fq_type_from_u8(3), FqType::Qfq); // 默认前复权
}
