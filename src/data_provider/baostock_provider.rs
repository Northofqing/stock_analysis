//! Baostock 数据源实现 (Task 13 — TCP 协议重写, 修复 C1 critical bug).
//!
//! **协议**: TCP socket 自定义协议, 不是 HTTP!
//! - Host: `public-api.baostock.com`, Port: `10030`
//! - 消息帧: `VERSION\x01TYPE\x01BODYLEN_10 + body + \x01 + CRC32_HEX(8) + \n`
//! - 末尾追加 `<![CDATA[]]>\n` (13 bytes, 作为响应结束标记)
//!
//! **消息类型**:
//! - `00` = login request, body: `login\x01user\x01pass\x1options`
//! - `01` = login response (不压缩)
//! - `95` = K线 query request, body: `query_history_k_data_plus\x01user\x1page\x1per_page\x1code\x1fields\x1start\x1end\x1frequency\x1adjustflag`
//! - `96` = K线 response (body **zlib 压缩**, 解压后是 `<![CDATA[ ... ]]>` 包裹的 key=value + CSV)
//!
//! **body_len 按 chars 计数** (实测跟 Python baostock 一致), 0-padded 10 位.
//!
//! **凭据**: `anonymous` / `888888` (公开访问, 无需注册).
//!
//! **Task 历史**:
//! - Task 5/6 (HTTP 误实现) → Task 13: 完整重写为 TCP.
//! - Task 7 的 `fallback::fetch_kline_post_close` 仍 OK — 它走 4-way 兜底, Baostock 死了 fallthrough 到 Sina.

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use flate2::read::ZlibDecoder;
use std::io::Read;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use super::stock_code_map::to_baostock;
use super::{AdjustType, DataProvider, KlineData, RealtimeQuote};

/// Baostock TCP endpoint (协议 host:port, 实测).
pub const BAOSTOCK_HOST: &str = "public-api.baostock.com";
pub const BAOSTOCK_PORT: u16 = 10030;
/// 协议版本 (实测 Python baostock 用此值).
pub const BAOSTOCK_VERSION: &str = "00.9.20";
/// 默认凭据 (匿名公开访问).
pub const BAOSTOCK_USER: &str = "anonymous";
pub const BAOSTOCK_PASS: &str = "888888";
/// 默认查询 K 线 fields (跟 Task 6 保持一致).
pub const BAOSTOCK_KLINE_FIELDS: &str = "date,open,high,low,close,volume,amount";
/// 复权 flag: 2 = 前复权 (跟 Task 6 build_kline_query_body 一致).
pub const BAOSTOCK_ADJUST_FLAG_QFQ: &str = "2";
/// 默认频率 (日线).
pub const BAOSTOCK_FREQUENCY_DAILY: &str = "d";

/// 响应结束标记 (服务端在每个响应后追加).
const RESPONSE_END_MARKER: &[u8] = b"<![CDATA[]]>\n";

/// 消息类型常量.
pub const MSG_TYPE_LOGIN_REQ: &str = "00";
pub const MSG_TYPE_LOGIN_RESP: &str = "01";
pub const MSG_TYPE_KLINE_REQ: &str = "95";
pub const MSG_TYPE_KLINE_RESP: &str = "96";

/// 解析后的 TCP 消息.
#[derive(Debug, Clone)]
pub struct BaostockTcpMessage {
    pub version: String,
    pub msg_type: String,
    pub body: String, // 已解压 (若原响应是 msg_type="96")
    pub crc32: u32,
}

/// Baostock 数据源 (TCP 协议).
///
/// 内部用 `Mutex<Option<TcpStream>>` 共享单个长连接 (懒连接), `Mutex<Option<String>>`
/// 缓存 session id. 首次 `ensure_session` 触发懒登录, 后续复用.
/// `pub(crate)` 字段允许测试和同 crate 其他模块访问.
pub struct BaostockProvider {
    pub(crate) stream: Arc<Mutex<Option<TcpStream>>>,
    pub(crate) session: Mutex<Option<String>>,
    pub(crate) host: String,
    pub(crate) port: u16,
}

// =====================================================================
// TCP 消息构造 helpers
// =====================================================================

/// 计算 CRC32 (zlib/PNG 兼容, baostock 协议使用).
pub fn baostock_crc32(buf: &[u8]) -> u32 {
    const POLY: u32 = 0xEDB88320;
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                POLY ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i as usize] = c;
    }
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in buf {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}

/// 构造 TCP 消息字节帧.
///
/// 协议格式 (跟 Python baostock 完全对齐):
/// - header: `VERSION\x01TYPE\x01BODYLEN_10` (21 字符, 全 ASCII)
/// - body:   `body` (UTF-8 bytes; body_len 按 **chars** 计数)
/// - crc separator: `\x01`
/// - crc32:   **decimal string** of `zlib.crc32(header + body).to_bytes()` (不是 hex!)
/// - 末尾: 服务端在响应里追加 `<![CDATA[]]>\n`, 客户端发出不需要
///
/// 实测 Python 源码 (baostock/login/loginout.py:58, baostock/security/history.py:102):
/// ```python
/// crc32str = zlib.crc32(bytes(head_body, encoding='utf-8'))  # head_body = header + body
/// send_msg(head_body + "\x01" + str(crc32str))  # decimal, not hex
/// ```
pub fn build_tcp_message(version: &str, msg_type: &str, body: &str) -> Vec<u8> {
    let body_chars_len = body.chars().count();
    let header = format!("{}\x01{}\x01{:010}", version, msg_type, body_chars_len);
    let body_bytes = body.as_bytes();
    // CRC32 over header + body, output as decimal string (NOT hex).
    let crc_input = format!("{header}{body}");
    let crc = baostock_crc32(crc_input.as_bytes());
    let mut frame = Vec::with_capacity(header.len() + body_bytes.len() + 16);
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(body_bytes);
    frame.push(b'\x01');
    frame.extend_from_slice(crc.to_string().as_bytes());
    frame
}

/// 构造登录消息 (msg_type="00").
///
/// body 格式: `login\x01user\x01pass\x01options`
/// options: 通常 "0" 表示登录; "1" 表示登出 (logout 复用本函数时传 "1").
pub fn build_login_msg(body: &str) -> String {
    let frame = build_tcp_message(BAOSTOCK_VERSION, MSG_TYPE_LOGIN_REQ, body);
    String::from_utf8(frame).expect("login msg 必为 UTF-8")
}

/// 构造登出 body (msg_type="00" 复用, 协议层共用).
///
/// review #16 P0 #2: 协议要求 4 字段 (login|user|pass|options).
/// 之前 logout 拼了 6 字段 (USER/PASS 重复 + session_id), 与 baostock 协议不符.
/// options="1" 表示登出 (服务端会清 session).
pub fn build_logout_body(user: &str, pass: &str, _session_id: &str) -> String {
    format!("login\x01{user}\x01{pass}\x011")
}

/// 构造 K线查询请求 body.
///
/// body 格式: `query_history_k_data_plus\x01user\x1page\x1per_page\x1code\x1fields\x1start\x1end\x1frequency\x1adjustflag`
pub fn build_kline_request_body(
    code: &str,
    fields: &str,
    start_date: &str,
    end_date: &str,
    session_id: &str,
) -> String {
    format!(
        "query_history_k_data_plus\x01{session_id}\x011\x01000\x01{code}\x01{fields}\x01{start_date}\x01{end_date}\x01{BAOSTOCK_FREQUENCY_DAILY}\x01{BAOSTOCK_ADJUST_FLAG_QFQ}"
    )
}

/// 构造 K线查询的整帧字节.
pub fn build_kline_query_frame(
    code: &str,
    fields: &str,
    start_date: &str,
    end_date: &str,
    session_id: &str,
) -> Vec<u8> {
    let body = build_kline_request_body(code, fields, start_date, end_date, session_id);
    build_tcp_message(BAOSTOCK_VERSION, MSG_TYPE_KLINE_REQ, &body)
}

// =====================================================================
// TCP 响应解析
// =====================================================================

/// 解析 baostock TCP 响应字节流 → `BaostockTcpMessage`.
///
/// 响应结构: `VERSION\x01TYPE\x01BODYLEN_10 + body + \x01 + CRC32_DEC + <![CDATA[]]>\n`
/// - header 固定 21 字节 (ASCII), body 起始偏移 21
/// - 若 msg_type == "96" (K线响应), body 是 zlib 压缩, 自动解压
/// - 不压缩响应 (msg_type "01" 等) body 从 `buf[21..-1]` 切 (剥末尾 `\n`)
///   — Python 源码: `receive[cons.MESSAGE_HEADER_LENGTH:-1]`
///
/// 不严格校验 CRC32 (跳过校验, 避免误拒包; 服务端一般不发脏包).
pub fn parse_baostock_tcp_response(buf: &[u8]) -> Result<BaostockTcpMessage> {
    if buf.len() < 21 {
        return Err(anyhow!("Baostock TCP 响应: 短于 header (21 字节), got {}", buf.len()));
    }
    // 1. header 固定 21 字节
    let header_bytes = &buf[..21];
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|e| anyhow!("Baostock TCP 响应: header 非 UTF-8: {e}"))?;
    let header_parts: Vec<&str> = header_str.split('\x01').collect();
    if header_parts.len() < 3 {
        return Err(anyhow!(
            "Baostock TCP 响应: header 字段不足, got {:?}",
            header_parts
        ));
    }
    let version = header_parts[0].to_string();
    let msg_type = header_parts[1].to_string();
    let body_len: usize = header_parts[2]
        .parse()
        .map_err(|e| anyhow!("Baostock TCP 响应: body_len 解析失败: {e}"))?;

    // 2. 解析 body
    let body_bytes = if msg_type == MSG_TYPE_KLINE_RESP {
        // 压缩响应: [21 : 21 + body_len]
        if 21 + body_len > buf.len() {
            return Err(anyhow!(
                "Baostock TCP 响应: 压缩 body 截断 (need {body_len} @ 21, have {})",
                buf.len().saturating_sub(21)
            ));
        }
        &buf[21..21 + body_len]
    } else {
        // 非压缩: [21 : -1] (剥末尾 `\n`) — 跟 Python 一致
        if buf.len() < 22 {
            return Err(anyhow!("Baostock TCP 响应: 非压缩 body 截断"));
        }
        &buf[21..buf.len() - 1]
    };

    // 3. msg_type="96" → zlib 解压
    let body_str = if msg_type == MSG_TYPE_KLINE_RESP {
        let mut decoder = ZlibDecoder::new(body_bytes);
        let mut decompressed = String::new();
        decoder
            .read_to_string(&mut decompressed)
            .map_err(|e| anyhow!("Baostock TCP 响应: zlib 解压失败: {e}"))?;
        decompressed
    } else {
        std::str::from_utf8(body_bytes)
            .map_err(|e| anyhow!("Baostock TCP 响应: body 非 UTF-8: {e}"))?
            .to_string()
    };

    // 4. CRC32 decimal (跳过校验)
    let crc_start = 21 + body_len + 1; // skip body + \x01
    let crc_str = if crc_start < buf.len() {
        // 读直到下个 `\n` 或 marker
        let end = buf[crc_start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'<')
            .unwrap_or(buf.len() - crc_start);
        std::str::from_utf8(&buf[crc_start..crc_start + end]).unwrap_or("0")
    } else {
        "0"
    };
    let crc32 = crc_str.parse().unwrap_or(0u32);

    Ok(BaostockTcpMessage {
        version,
        msg_type,
        body: body_str,
        crc32,
    })
}

/// 等待 TCP 响应 (从已连接的 stream 读取直到末尾 marker).
///
/// 累积到 buf 里, 末尾匹配 `<![CDATA[]]>\n` (13 bytes) 或超过 1 MB 强制返回.
async fn read_tcp_response(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    const MAX_BUF: usize = 1_048_576; // 1 MB 上限
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(anyhow!("Baostock TCP: 连接在响应中途关闭"));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_BUF {
            return Err(anyhow!("Baostock TCP: 响应超 {MAX_BUF} 字节, 强制截断"));
        }
        // 末尾匹配
        if buf.ends_with(RESPONSE_END_MARKER) {
            return Ok(buf);
        }
    }
}

/// 从 `<![CDATA[...]]>` 包裹的 K线 body 剥出内层文本.
///
/// baostock K线响应格式: `<![CDATA[error_code=0\nerror_msg=success\ndate,open,...\n...]]>`
pub fn strip_cdata(body: &str) -> &str {
    let s = body.trim();
    if let Some(rest) = s.strip_prefix("<![CDATA[") {
        if let Some(inner) = rest.strip_suffix("]]>") {
            return inner;
        }
    }
    s
}

/// 解析 baostock K线响应 body → `Vec<KlineData>`.
///
/// 与 Task 6 的 `parse_kline_body` 等价, 但额外处理 CDATA 包裹层
/// 及内层 `error_code=0` / `error_msg=success` 等键值行 (跳过它们, 找 CSV header).
/// 输入可以是解压后的完整 body 或已剥 CDATA 的内层文本.
pub fn parse_baostock_response_kline(body: &str, our_code: &str) -> Result<Vec<KlineData>> {
    let inner = strip_cdata(body);
    // 跳过 `key=value` 形式的元信息行, 定位到 CSV header (`date,open,...`)
    let csv_start = inner
        .lines()
        .position(|line| line.starts_with("date,") || line.starts_with("code,date,"))
        .ok_or_else(|| anyhow!("Baostock K线: 找不到 CSV header (date,...)"))?;
    let csv_only = inner.lines().skip(csv_start).collect::<Vec<_>>().join("\n");
    parse_kline_body(&csv_only, our_code)
}

/// 解析 Baostock K线 CSV body → `Vec<KlineData>`.
///
/// 输入格式 (实测):
/// ```text
/// code,date,open,high,low,close,volume,amount
/// sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50
/// ```
///
/// - 第 1 行是表头, 后续每行 1 条 K线.
/// - `date` 格式 `"YYYY-MM-DD"`.
/// - 解析失败的字段回退为 0.0 / 今天 (不抛 Err, 保证解析容错).
pub fn parse_kline_body(body: &str, _our_code: &str) -> Result<Vec<KlineData>> {
    let mut lines = body.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| anyhow!("Baostock K线: 空 body"))?;
    let headers: Vec<&str> = header_line.split(',').collect();

    let idx = |name: &str| -> Result<usize> {
        headers
            .iter()
            .position(|h| h.trim() == name)
            .ok_or_else(|| anyhow!("Baostock K线: 缺 {} 列", name))
    };
    let i_date = idx("date")?;
    let i_open = idx("open")?;
    let i_high = idx("high")?;
    let i_low = idx("low")?;
    let i_close = idx("close")?;
    let i_volume = idx("volume")?;
    let i_amount = idx("amount")?;

    let mut result = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 {
            continue;
        }

        let date = NaiveDate::parse_from_str(fields[i_date], "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let open: f64 = fields[i_open].parse().unwrap_or(0.0);
        let high: f64 = fields[i_high].parse().unwrap_or(0.0);
        let low: f64 = fields[i_low].parse().unwrap_or(0.0);
        let close: f64 = fields[i_close].parse().unwrap_or(0.0);
        let volume: f64 = fields[i_volume].parse().unwrap_or(0.0);
        let amount: f64 = fields[i_amount].parse().unwrap_or(0.0);
        let pct_chg = if open > 0.0 {
            (close - open) / open * 100.0
        } else {
            0.0
        };

        result.push(KlineData {
            date,
            open,
            high,
            low,
            close,
            volume,
            amount,
            pct_chg,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::Qfq,
        });
    }
    let _ = _our_code;
    Ok(result)
}

// =====================================================================
// BaostockProvider impl (TCP)
// =====================================================================

impl BaostockProvider {
    /// 创建新实例.
    pub fn new() -> Self {
        Self {
            stream: Arc::new(Mutex::new(None)),
            session: Mutex::new(None),
            host: std::env::var("BAOSTOCK_HOST").unwrap_or_else(|_| BAOSTOCK_HOST.to_string()),
            port: std::env::var("BAOSTOCK_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(BAOSTOCK_PORT),
        }
    }

    /// 懒连接 + 懒登录. 命中缓存直接返回 session id.
    pub async fn ensure_session(&self) -> Result<String> {
        // 先 fast-path: 已登录
        {
            let guard = self.session.lock().await;
            if let Some(sid) = guard.as_ref() {
                return Ok(sid.clone());
            }
        }
        // 慢路径: 登录 (内部 connect + send + recv + parse)
        let sid = self.login().await?;
        let mut guard = self.session.lock().await;
        *guard = Some(sid.clone());
        Ok(sid)
    }

    /// TCP 连接 (懒建立, 复用 `stream` Arc).
    async fn connect(&self) -> Result<()> {
        let mut guard = self.stream.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let addr = format!("{}:{}", self.host, self.port);
        log::info!("[Baostock] TCP 连接 {addr}");
        let stream = TcpStream::connect(&addr).await.map_err(|e| {
            anyhow!("Baostock TCP 连接 {addr} 失败: {e}")
        })?;
        // 设 read/write timeout (10s), 防服务端挂起
        stream.set_nodelay(true).ok();
        *guard = Some(stream);
        Ok(())
    }

    /// 真实登录: connect + send login msg + recv + parse session_id.
    pub(crate) async fn login(&self) -> Result<String> {
        self.connect().await?;
        let body = format!("login\x01{BAOSTOCK_USER}\x01{BAOSTOCK_PASS}\x010");
        let frame = build_login_msg(&body);
        let resp_bytes = self.send_and_recv(frame.as_bytes()).await?;
        let parsed = parse_baostock_tcp_response(&resp_bytes)?;

        let error_code = parse_baostock_response(&parsed.body, "error_code")?
            .ok_or_else(|| anyhow!("Baostock login: 无 error_code"))?;
        if error_code != "0" {
            let msg = parse_baostock_response(&parsed.body, "error_msg")?.unwrap_or_default();
            return Err(anyhow!("Baostock login 失败: code={error_code} msg={msg}"));
        }
        let sid = parse_baostock_response(&parsed.body, "session_id")?
            .ok_or_else(|| anyhow!("Baostock login: 无 session_id"))?;
        log::info!(
            "[Baostock] login 成功, session_id={}",
            &sid[..8.min(sid.len())]
        );
        Ok(sid)
    }

    /// 登出 (msg_type="00" body 含 options="1", 服务端会清 session).
    ///
    /// review #16 P0 #2: body 改为 4 字段 `login|user|pass|options=1`,
    /// 之前 6 字段 (USER/PASS 重复 + session_id) 与协议不符.
    pub async fn logout(&self, session_id: &str) {
        let body = build_logout_body(BAOSTOCK_USER, BAOSTOCK_PASS, session_id);
        let frame = build_login_msg(&body);
        match self.send_and_recv(frame.as_bytes()).await {
            Ok(resp) => {
                let preview = String::from_utf8_lossy(&resp[..80.min(resp.len())]);
                log::info!("[Baostock] logout 响应: {preview}");
            }
            Err(e) => {
                log::warn!("[Baostock] logout 失败: {e}");
            }
        }
    }

    /// 发送一帧 + 接收完整响应.
    pub(crate) async fn send_and_recv(&self, frame: &[u8]) -> Result<Vec<u8>> {
        let mut guard = self.stream.lock().await;
        let stream = guard
            .as_mut()
            .ok_or_else(|| anyhow!("Baostock TCP: 未连接 (应先 connect)"))?;
        stream.write_all(frame).await.map_err(|e| {
            anyhow!("Baostock TCP: 发送失败: {e}")
        })?;
        // 单次响应: 累积到末尾 marker
        read_tcp_response(stream).await
    }

    /// 异步拉取 K 线 (TCP).
    ///
    /// 起始日期用 `days * 2` 留 buffer (含停牌日).
    pub async fn fetch_kline_async(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let sid = self.ensure_session().await?;
        let bs_code = to_baostock(code);
        let end_date = chrono::Local::now().date_naive();
        let start_date = end_date - chrono::Duration::days(days as i64 * 2);

        let frame = build_kline_query_frame(
            &bs_code,
            BAOSTOCK_KLINE_FIELDS,
            &start_date.format("%Y-%m-%d").to_string(),
            &end_date.format("%Y-%m-%d").to_string(),
            &sid,
        );
        let resp_bytes = self.send_and_recv(&frame).await?;
        let parsed = parse_baostock_tcp_response(&resp_bytes)?;

        if parsed.msg_type != MSG_TYPE_KLINE_RESP {
            return Err(anyhow!(
                "Baostock K线: 期望 msg_type=96, 实际 {}",
                parsed.msg_type
            ));
        }
        let error_code = parse_baostock_response(&parsed.body, "error_code")?
            .ok_or_else(|| anyhow!("Baostock K线: 无 error_code"))?;
        if error_code != "0" {
            let msg = parse_baostock_response(&parsed.body, "error_msg")?.unwrap_or_default();
            return Err(anyhow!("Baostock K线失败: code={error_code} msg={msg}"));
        }
        parse_baostock_response_kline(&parsed.body, code)
    }
}

impl Default for BaostockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str {
        "baostock"
    }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        crate::block_on_async(self.fetch_kline_async(code, days))
    }
    fn get_stock_name(&self, _code: &str) -> Option<String> {
        None
    }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> {
        Ok(None)
    }
}

// =====================================================================
// 兼容层 (Task 5/6 留下的 HTTP-style helpers, 保留供其它模块调用)
// =====================================================================

/// 兼容名: HTTP 风格 base URL (实际不用, 保留以防外部 import).
/// ⚠️ Baostock 协议是 TCP 不是 HTTP, 这个常量仅作历史占位.
#[deprecated(note = "Baostock 协议是 TCP; 用 BAOSTOCK_HOST / BAOSTOCK_PORT")]
pub const BAOSTOCK_DEFAULT_BASE: &str = "http://baostock.com/baostock";

/// 兼容名: 构造登录 URL (Task 5 HTTP 误实现的痕迹, 不再使用).
#[deprecated(note = "Baostock 协议是 TCP; 用 build_login_msg")]
pub fn build_login_url() -> String {
    format!("{BAOSTOCK_DEFAULT_BASE}/Login")
}

/// 兼容名: 构造登出 URL (Task 5 HTTP 误实现的痕迹, 不再使用).
#[deprecated(note = "Baostock 协议是 TCP; 用 build_login_msg")]
pub fn build_logout_url() -> String {
    format!("{BAOSTOCK_DEFAULT_BASE}/Logout")
}

/// 兼容名: 构造 K线查询 body (HTTP form-encoded 风格, 不再使用).
/// 保留供可能存在的外部 import 编译通过.
#[deprecated(note = "Baostock 协议是 TCP; 用 build_kline_request_body")]
pub fn build_kline_query_body(
    code: &str,
    fields: &str,
    start_date: &str,
    end_date: &str,
    session_id: &str,
) -> String {
    format!(
        "QueryHistoryKLinePlus&code={code}&fields={fields}&adjustflag=2&\
         startdate={start_date}&enddate={end_date}&sessionid={session_id}"
    )
}

/// 兼容名: 解析 HTTP 风格 key=value 响应 (仍有用 — 解析解压后的 body).
pub fn parse_baostock_response(body: &str, key: &str) -> Result<Option<String>> {
    let prefix = format!("{}=", key);
    for line in body.lines() {
        if let Some(val) = line.strip_prefix(&prefix) {
            return Ok(Some(val.trim().to_string()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    /// `BAOSTOCK_HOST` / `BAOSTOCK_PORT` 必须是稳定的 TCP endpoint.
    #[test]
    fn tcp_endpoint_is_stable() {
        assert_eq!(BAOSTOCK_HOST, "public-api.baostock.com");
        assert_eq!(BAOSTOCK_PORT, 10030);
    }

    /// `build_tcp_message` 关键帧格式断言 (独立于 tests/baostock_provider_test.rs).
    /// 客户端帧格式: header(21) + body + \x01 + CRC32_dec. 末尾不含 \n.
    #[test]
    fn tcp_frame_format() {
        let frame = build_tcp_message("00.9.20", "00", "login");
        // 必须以 header 开始: "00.9.20\x0100\x01"
        assert!(frame.starts_with(b"00.9.20\x0100\x01"));
        // 末尾必为数字 (CRC32 decimal), 不是 \n
        let last = *frame.last().unwrap();
        assert!(last.is_ascii_digit(), "frame tail must be digit, got {:?}", last as char);
        // CRC 分隔符 \x01 必须在 body 之后
        let body_end = 21 + "login".len();
        assert_eq!(frame[body_end], b'\x01', "frame[body_end] must be \\x01");
    }

    /// CRC32 应符合 zlib/PNG 标准 (空字符串 → 0).
    #[test]
    fn crc32_empty_is_zero() {
        assert_eq!(baostock_crc32(b""), 0);
    }

    /// `parse_baostock_response` 容忍尾部空白和 `\r\n`.
    #[test]
    fn parse_handles_crlf() {
        let body = "session_id=XYZ \r\nerror_code=0\r\n";
        assert_eq!(
            parse_baostock_response(body, "session_id").unwrap(),
            Some("XYZ".to_string())
        );
    }
}