//! Tests for BaostockProvider URL/format helpers.
//!
//! 覆盖 Task 5 (BaostockProvider 骨架):
//! - build_login_url / build_logout_url — URL 构造
//! - build_kline_query_body — form body 包含所有关键字段 (code/fields/adjustflag/sessionid)
//! - parse_baostock_response — key=value 行解析 (含 Missing 返回 None)
//!
//! Task 13 (TCP 协议重写, C1 critical fix):
//! - test_build_login_msg_format — 验证 TCP 消息帧 (header / body / crc32 / \n 结尾)
//! - test_parse_login_response_success — 解析真实登录响应 (session_id 提取)
//! - test_parse_kline_response_decompresses — msg_type="96" zlib 解压 + CSV record 解析

#![allow(
    deprecated,
    reason = "these compatibility tests intentionally verify the retained pre-TCP helper contract"
)]

use stock_analysis::data_provider::baostock_provider::{
    build_kline_query_body, build_kline_request_body, build_login_msg, build_login_url,
    build_logout_body, build_logout_url, parse_baostock_response, parse_baostock_response_kline,
    parse_baostock_tcp_response, read_tcp_response, BaostockTcpMessage,
};

#[test]
fn parse_kline_body_format() {
    // Baostock 响应格式 (实测):
    // code,date,open,high,low,close,volume,amount,pctChg
    let body = "code,date,open,high,low,close,volume,amount,pctChg\n\
                sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50,0.37\n\
                sh.600000,2024-01-16,13.55,13.70,13.50,13.65,15000,20000.00,0.74\n";
    let klines =
        stock_analysis::data_provider::baostock_provider::parse_kline_body(body, "600000").unwrap();
    assert_eq!(klines.len(), 2);
    assert_eq!(klines[0].open, 13.55);
    assert_eq!(klines[0].close, 13.65);
    assert_eq!(klines[1].volume, 12345.0);
    assert_eq!(klines[1].amount, 16789.50);
    assert_eq!(
        klines[0].date,
        chrono::NaiveDate::from_ymd_opt(2024, 1, 16).unwrap()
    );
}

#[test]
fn parse_kline_body_rejects_malformed_required_field() {
    let body = "date,open,high,low,close,volume,amount,pctChg\n\
                2024-01-15,not-a-price,13.60,13.45,13.55,12345,16789.50,0.37\n";
    let error = stock_analysis::data_provider::baostock_provider::parse_kline_body(body, "600000")
        .unwrap_err();
    assert!(error.to_string().contains("open='not-a-price'"));
}

#[test]
fn test_build_login_url() {
    assert_eq!(build_login_url(), "http://baostock.com/baostock/Login");
}

#[test]
fn test_build_logout_url() {
    assert_eq!(build_logout_url(), "http://baostock.com/baostock/Logout");
}

#[test]
fn build_kline_query_body_format() {
    let body = build_kline_query_body(
        "sh.600000",
        "date,open,high,low,close",
        "20240101",
        "20241231",
        "session_xxx",
    );
    assert!(body.contains("QueryHistoryKLinePlus"));
    assert!(body.contains("code=sh.600000"));
    assert!(body.contains("adjustflag=2")); // 前复权
    assert!(body.contains("sessionid=session_xxx"));
}

#[test]
fn parse_baostock_response_extracts_field() {
    let body = "sessionId=ABC123\nErrorCode=0\nErrorMsg=success\n";
    assert_eq!(
        parse_baostock_response(body, "sessionId").unwrap(),
        Some("ABC123".to_string())
    );
    assert_eq!(
        parse_baostock_response(body, "ErrorCode").unwrap(),
        Some("0".to_string())
    );
    assert_eq!(parse_baostock_response(body, "Missing").unwrap(), None);
}

// ============================================================================
// Task 13: TCP 协议测试 (C1 critical fix)
// ============================================================================

/// Task 13: 验证 TCP 消息帧格式 — header `VERSION\x01TYPE\x01BODYLEN_10` + body + `\x01CRC32_DEC`.
/// 实测 baostock (Python baostock 源码): version="00.9.20", msg_type="00" (login).
/// 关键点: body 包含 "login\x01anonymous\x01888888\x010",
/// body_len 按 chars (非 bytes) 计数, 0-padded 10 位,
/// CRC32 是 header+body 的 zlib-crc32, 输出 **decimal 字符串** (不是 hex).
/// 客户端发出帧末尾不带 `\n`; 服务端响应末尾追加 `<![CDATA[]]>\n` (13 bytes).
#[test]
fn test_build_login_msg_format() {
    let body = "login\x01anonymous\x01888888\x010".to_string();
    let msg = build_login_msg(&body);

    // 1. header 段: 必须含 VERSION="00.9.20" + msg_type="00"
    assert!(
        msg.contains("00.9.20"),
        "msg must contain version '00.9.20', got first 80 bytes: {:?}",
        &msg[..80.min(msg.len())]
    );
    assert!(
        msg.contains("\x0100\x01"),
        "msg must contain '\\x0100\\x01' separator + msg_type='00', got: {:?}",
        &msg[..80.min(msg.len())]
    );

    // 2. body 必须存在
    assert!(
        msg.contains("login\x01anonymous\x01888888\x010"),
        "msg must contain login body"
    );

    // 3. 末尾不能是 \n (客户端帧不含 \n; 服务端响应才追加)
    //    末尾是 CRC32 decimal string + 不带 delimiter
    let last_char = msg.chars().last().expect("non-empty");
    assert!(
        last_char.is_ascii_digit(),
        "msg tail must be a decimal digit (CRC32), got {:?}",
        last_char
    );

    // 4. 消息能成功 decode 成结构 (roundtrip)
    let parsed = parse_baostock_tcp_response(msg.as_bytes()).expect("parse login msg");
    assert_eq!(parsed.msg_type, "00");
    assert!(parsed.body.contains("login"));
}

/// Task 13: 解析真实登录响应 — error_code="0" 表示成功, 提取 session_id.
///
/// 真实响应格式 (msg_type="01", 不压缩):
/// ```text
/// <21-byte header> + body + \x01 + CRC32_dec + \n + <![CDATA[]]>\n
/// ```
#[test]
fn test_parse_login_response_success() {
    // 构造一个真实的响应: msg_type="01" (login response, 不压缩)
    let inner = "error_code=0\nsession_id=ABCDEFG12345\nerror_msg=success";
    let body_len = inner.chars().count(); // 按 chars 计数 (Python 一致)
    let header = format!("00.9.20\x0101\x01{:010}", body_len);
    let head_body = format!("{header}{inner}");
    let crc = baostock_crc32(head_body.as_bytes());

    let mut msg = Vec::new();
    msg.extend_from_slice(head_body.as_bytes());
    msg.push(b'\x01');
    msg.extend_from_slice(crc.to_string().as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(b"<![CDATA[]]>\n");

    let parsed = parse_baostock_tcp_response(&msg).expect("parse login response");
    assert_eq!(parsed.msg_type, "01");
    assert!(parsed.body.contains("error_code=0"));
    assert!(parsed.body.contains("session_id=ABCDEFG12345"));

    let session_id = parse_baostock_response(&parsed.body, "session_id")
        .unwrap()
        .expect("session_id should be present");
    assert_eq!(session_id, "ABCDEFG12345");

    let err_code = parse_baostock_response(&parsed.body, "error_code")
        .unwrap()
        .expect("error_code should be present");
    assert_eq!(err_code, "0");
}

/// Task 13: K线响应 msg_type="96" 时 body 是 zlib 压缩, 必须解压后才能 parse.
/// 解压后内层是 CDATA 包裹: `<![CDATA[ ... ]]>` 内含 key=value 行 + CSV 数据.
#[test]
fn test_parse_kline_response_decompresses() {
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    // 内层 CDATA: 含 error_code=0 + CSV 数据
    let inner_body = "error_code=0\nerror_msg=success\n\
                      date,open,high,low,close,volume,amount,pctChg\n\
                      2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50,0.37\n\
                      2024-01-16,13.55,13.70,13.50,13.65,15000,20000.00,0.74\n";
    let cdata_wrapped = format!("<![CDATA[{}]]>", inner_body);

    // zlib 压缩
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(cdata_wrapped.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();

    let body_len = compressed.len(); // 压缩后 byte 长度
    let header = format!("00.9.20\x0196\x01{:010}", body_len);
    // CRC32 over header + body (跟 Python 一致)
    let crc = {
        let mut h = header.as_bytes().to_vec();
        h.extend_from_slice(&compressed);
        baostock_crc32(&h)
    };

    let mut msg = Vec::new();
    msg.extend_from_slice(header.as_bytes());
    msg.extend_from_slice(&compressed);
    msg.push(b'\x01');
    msg.extend_from_slice(crc.to_string().as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(b"<![CDATA[]]>\n");

    let parsed = parse_baostock_tcp_response(&msg).expect("parse compressed kline response");
    assert_eq!(parsed.msg_type, "96");
    assert!(parsed
        .body
        .contains("date,open,high,low,close,volume,amount,pctChg"));
    assert!(parsed.body.contains("13.50"));
    assert!(parsed.body.contains("2024-01-16"));

    let klines = parse_baostock_response_kline(&parsed.body, "600000")
        .expect("parse_kline_response_kline should succeed");
    assert_eq!(klines.len(), 2);
    assert_eq!(klines[0].open, 13.55);
    assert_eq!(
        klines[0].date,
        chrono::NaiveDate::from_ymd_opt(2024, 1, 16).unwrap()
    );
}

// ============================================================================
// CRC32 helper (zlib-crc 兼容 baostock 协议)
// ============================================================================

/// baostock 协议用的 CRC32 (跟 zlib/PNG CRC32 一致).
/// 复制 zlib 的 CRC32 算法, 避免引入额外依赖 (zlib 已有, 但只用于 read).
fn baostock_crc32(buf: &[u8]) -> u32 {
    // 用简单的 CRC32 IEEE 多项式 (zlib 默认). 标准 table-driven.
    const POLY: u32 = 0xEDB88320;
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { POLY ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in buf {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}

/// 触发 build_kline_request_body 编译期可达性 (避免 unused warning).
#[test]
fn _kline_request_body_compiles() {
    let body = build_kline_request_body(
        "sh.600000",
        "date,open,high,low,close,volume,amount",
        "20240101",
        "20241231",
        "session_xxx",
    );
    assert!(body.contains("query_history_k_data_plus"));
    assert!(body.contains("sh.600000"));
}

/// 确保 BaostockTcpMessage 公开 (单元测试可用)
#[test]
fn _tcp_message_struct_exists() {
    let _ = BaostockTcpMessage {
        version: "00.9.20".to_string(),
        msg_type: "00".to_string(),
        body: "login".to_string(),
        crc32: 0,
    };
}

// ============================================================================
// Batch 1 P0 #2: Baostock logout body 4 字段 (login|user|pass|options=1)
// ============================================================================

/// P0 #2: logout body 协议修复 — 4 字段 (login|user|pass|options).
/// 之前 6 字段 (USER/PASS 重复 + session_id) 与 baostock 协议不符, 服务端会拒包.
#[test]
fn test_build_logout_body_format() {
    let body = build_logout_body("anonymous", "888888", "session_123");
    let parts: Vec<&str> = body.split('\x01').collect();
    assert_eq!(
        parts.len(),
        4,
        "logout body 应 4 字段, 实际 {} 字段: {body:?}",
        parts.len()
    );
    assert_eq!(parts[0], "login", "field 0 must be 'login'");
    assert_eq!(parts[1], "anonymous", "field 1 must be user");
    assert_eq!(parts[2], "888888", "field 2 must be pass");
    assert_eq!(parts[3], "1", "field 3 must be options='1' (logout)");
}

// ============================================================================
// Batch 1 P0 #3: Baostock read_tcp_response 加 timeout (防服务端挂起)
// ============================================================================

/// P0 #3: read_tcp_response 必须在 timeout 后返 Err, 不能永久 await.
/// 用内存双工流模拟对端保持连接但不发数据 (挂起), 不依赖本机端口权限.
#[tokio::test]
async fn test_read_tcp_response_times_out_on_hang() {
    let (mut stream, stalled_peer) = tokio::io::duplex(1);

    // 对端保持存活但不写；短超时后必须返 Err，而不是永久 await.
    let result = read_tcp_response(&mut stream, std::time::Duration::from_millis(20)).await;
    drop(stalled_peer);

    assert!(
        result.is_err(),
        "timeout 后 read_tcp_response 应返 Err, got: {result:?}"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("超时") || err_msg.contains("timeout"),
        "错误信息应含 '超时'/'timeout', got: {err_msg}"
    );
}
