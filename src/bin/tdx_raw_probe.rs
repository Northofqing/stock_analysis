//! 原始 TDX K 线响应探针 — 诊断 E2103 "security bar row 0 is truncated"。
//!
//! 用法: cargo run --bin tdx_raw_probe -- <server-ip> [code] [market]
//!   bars: 连指定 TDX 服务器 → 握手 → 发日线请求 → dump 响应头/body 字节 + 尝试解析。
//!   --quote: 同上但发实时行情请求 (0x5053E), dump quote 行原始字节 (用于验证时间编码)。
//! 只读探测，无写入、无推送。

use std::time::Duration;

use magic_tdx_rs::net::connection::TcpConnection;
use magic_tdx_rs::net::packet::{ResponseHeader, RSP_HEADER_LEN};
use magic_tdx_rs::net::utils::{build_security_bars_packet, perform_handshake};
use magic_tdx_rs::protocol::parsers::parse_security_bars;

fn hex_dump(prefix: &str, body: &[u8], max: usize) {
    eprintln!("{prefix} (len={})", body.len());
    for (i, b) in body.iter().take(max).enumerate() {
        print!("{b:02x} ");
        if (i + 1) % 16 == 0 {
            println!();
        }
    }
    println!();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quote_mode = args.iter().any(|a| a == "--quote");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let server = positional
        .get(1)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "218.75.126.9".to_string());
    let code = positional
        .get(2)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "605178".to_string());
    let market: u8 = positional.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    eprintln!(
        "== probing {server}:7709 code={code} market={market} mode={} ==",
        if quote_mode { "quote" } else { "bars" }
    );

    let mut conn = TcpConnection::connect(&server, 7709, 5.0)
        .unwrap_or_else(|e| panic!("connect {server}: {e}"));
    perform_handshake(&mut conn).unwrap_or_else(|e| panic!("handshake: {e}"));

    if quote_mode {
        // 实时行情请求 (格式同 direct_client.rs get_security_quotes)
        let stock_len: u16 = 1;
        let pkgdatalen = (stock_len as u32) * 7 + 12;
        let mut pkt = Vec::with_capacity(26 + stock_len as usize * 7);
        pkt.extend_from_slice(&0x010Cu16.to_le_bytes());
        pkt.extend_from_slice(&0x02006320u32.to_le_bytes());
        pkt.extend_from_slice(&(pkgdatalen as u16).to_le_bytes());
        pkt.extend_from_slice(&(pkgdatalen as u16).to_le_bytes());
        pkt.extend_from_slice(
            &magic_tdx_rs::protocol::constants::CMD_SECURITY_QUOTES.to_le_bytes(),
        );
        pkt.extend_from_slice(&0u32.to_le_bytes());
        pkt.extend_from_slice(&0u16.to_le_bytes());
        pkt.extend_from_slice(&stock_len.to_le_bytes());
        pkt.push(market);
        let mut code_buf = [0u8; 6];
        code_buf[..code.len().min(6)].copy_from_slice(&code.as_bytes()[..code.len().min(6)]);
        pkt.extend_from_slice(&code_buf);
        conn.send(&pkt)
            .unwrap_or_else(|e| panic!("send quote: {e}"));

        let head_buf = conn
            .recv(RSP_HEADER_LEN)
            .unwrap_or_else(|e| panic!("recv header: {e}"));
        let header =
            ResponseHeader::parse(&head_buf).unwrap_or_else(|e| panic!("header parse: {e}"));
        eprintln!(
            "header: seq={} method={} zip_size={} unzip_size={}",
            header.seq, header.method, header.zip_size, header.unzip_size
        );
        let zip = header.zip_size as usize;
        let mut body_buf = Vec::with_capacity(zip);
        while body_buf.len() < zip {
            let remaining = zip - body_buf.len();
            let chunk = conn
                .recv(remaining)
                .unwrap_or_else(|e| panic!("recv body: {e}"));
            body_buf.extend_from_slice(&chunk);
        }
        let body = if zip != header.unzip_size as usize {
            magic_tdx_rs::net::utils::decompress_zlib(&body_buf)
                .unwrap_or_else(|e| panic!("decompress: {e}"))
        } else {
            body_buf
        };
        hex_dump("quote body", &body, 128);
        if body.len() >= 2 {
            let count = u16::from_le_bytes([body[0], body[1]]);
            eprintln!("declared quote count = {count}");
        }
        match magic_tdx_rs::protocol::parsers::parse_security_quotes(&body) {
            Ok(quotes) => {
                eprintln!("QUOTE PARSE OK: {} records", quotes.len());
                for q in quotes.iter().take(2) {
                    eprintln!(
                        "  {} price={} last_close={} open={} high={} low={} servertime='{}'",
                        q.code, q.price, q.last_close, q.open, q.high, q.low, q.servertime
                    );
                }
            }
            Err(e) => eprintln!("QUOTE PARSE FAIL: {e}"),
        }
        return;
    }

    // 日线请求: category=9, count=10
    let pkt = build_security_bars_packet(9, market, &code, 0, 10, 0);
    conn.send(&pkt).unwrap_or_else(|e| panic!("send: {e}"));

    let head_buf = conn
        .recv(RSP_HEADER_LEN)
        .unwrap_or_else(|e| panic!("recv header: {e}"));
    let header = ResponseHeader::parse(&head_buf).unwrap_or_else(|e| panic!("header parse: {e}"));
    eprintln!(
        "header: seq={} method={} zip_size={} unzip_size={}",
        header.seq, header.method, header.zip_size, header.unzip_size
    );

    let zip = header.zip_size as usize;
    let mut body_buf = Vec::with_capacity(zip);
    while body_buf.len() < zip {
        let remaining = zip - body_buf.len();
        let chunk = conn
            .recv(remaining)
            .unwrap_or_else(|e| panic!("recv body: {e}"));
        body_buf.extend_from_slice(&chunk);
    }
    let body = if zip != header.unzip_size as usize {
        magic_tdx_rs::net::utils::decompress_zlib(&body_buf)
            .unwrap_or_else(|e| panic!("decompress: {e}"))
    } else {
        body_buf
    };

    hex_dump("body", &body, 64);

    // 前 2 字节 = 记录数
    if body.len() >= 2 {
        let count = u16::from_le_bytes([body[0], body[1]]);
        eprintln!("declared record count = {count}");
    }

    match parse_security_bars(&body, 9) {
        Ok(bars) => eprintln!("PARSE OK: {} bars, first={:?}", bars.len(), bars.first()),
        Err(e) => eprintln!("PARSE FAIL: {e}"),
    }
}
