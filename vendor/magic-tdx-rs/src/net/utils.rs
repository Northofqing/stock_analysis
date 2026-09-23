//! 网络客户端公共工具
//!
//! 提取 client.rs / direct_client.rs / async_client.rs 中的重复逻辑

use flate2::read::ZlibDecoder;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::net::connection::TcpConnection;
use crate::net::packet::{ResponseHeader, RSP_HEADER_LEN};
use crate::protocol::constants::*;
use crate::protocol::parsers::parse_security_bars;
use crate::protocol::types::{SecurityBar, XdXrInfo};

/// 复权上下文数据量档位
///
/// 控制 `fetch_context_bars_for_adjust` 拉取的历史 K 线数量,
/// 用于覆盖早期除权除息事件的因子计算。
///
/// | 档位 | 根数 | 约覆盖年限 (日K) |
/// |------|------|------------------|
/// | Low  | 2400 | ~10 年 |
/// | Mid  | 4800 | ~20 年 (默认) |
/// | High | 7200 | ~30 年 |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FqContextTier {
    /// 约 10 年 (3 页 × 800 根)
    Low = 2400,
    /// 约 20 年 (6 页 × 800 根) — 默认
    #[default]
    Mid = 4800,
    /// 约 30 年 (9 页 × 800 根)
    High = 7200,
}

impl FqContextTier {
    /// 计算对应的翻页数
    pub fn pages(&self) -> u32 {
        *self as u32 / MAX_KLINE_COUNT as u32
    }
}

// ================================================================
// 交易阶段检测
// ================================================================

/// 交易阶段 (简化判断: 不考量交易所节假日)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingPhase {
    /// 交易时段 (工作日 9:30-15:00)
    Trading,
    /// 盘前/盘后 (工作日非交易时段)
    PrePost,
    /// 休市 (周末)
    Closed,
}

/// 限流分档乘数: (日K乘数, 分时乘数)
const PHASE_MULTIPLIER: [(f64, f64); 3] = [
    (1.0, 1.0), // Trading: 保持基础限流
    (2.0, 1.5), // PrePost: 放宽
    (4.0, 3.0), // Closed:  大幅放宽
];

/// 检测当前交易阶段
pub fn detect_trading_phase() -> TradingPhase {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    detect_trading_phase_at(now)
}

fn detect_trading_phase_at(unix_seconds: u64) -> TradingPhase {
    // 先整体平移到 UTC+8，再从同一个本地时间戳计算星期和日内秒数。
    // epoch 1970-01-01 是周四 (day 4)
    let local_seconds = unix_seconds.saturating_add(8 * 3600);
    let day_sec = local_seconds % 86400;
    let week_day = ((local_seconds / 86400 + 4) % 7) as u8; // 0=Sun, ..., 6=Sat

    // 周末
    if week_day == 0 || week_day == 6 {
        return TradingPhase::Closed;
    }

    let morning = 9 * 3600 + 30 * 60..=11 * 3600 + 30 * 60;
    let afternoon = 13 * 3600..=15 * 3600;
    if morning.contains(&day_sec) || afternoon.contains(&day_sec) {
        TradingPhase::Trading
    } else {
        TradingPhase::PrePost
    }
}

/// 获取今日日期 (YYYYMMDD 格式)
///
/// 用于 `get_minute_time_data` 委托给 `get_history_minute_time_data`，
/// 避免实时分时 API (0x051d) 的价格编码异常。
pub fn today_yyyymmdd() -> u32 {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // UTC+8 时区偏移
    let ts = now + 8 * 3600;
    let days = ts / 86400;
    // 从 epoch 天数计算年月日 (简化算法)
    let mut y: u64 = 1970;
    let mut remaining = days;
    loop {
        let days_in_year =
            if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) {
                366
            } else {
                365
            };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0u32;
    for &d in &month_days {
        if remaining < d {
            break;
        }
        remaining -= d;
        m += 1;
    }
    let d = remaining + 1;
    (y as u32) * 10000 + (m + 1) * 100 + d as u32
}

// ================================================================
// 请求限流器
// ================================================================

/// 最大允许的请求速率 (req/s)
const MAX_RATE_LIMIT: u32 = 200;

/// 请求速率限制器
///
/// 通过最小请求间隔实现限流。上限 200 req/s。
/// 支持交易阶段自动调整: 交易时段保守，休市时段放宽。
pub struct RateLimiter {
    enabled: AtomicBool,
    inner: Mutex<RateLimiterInner>,
}

struct RateLimiterInner {
    /// 基准最小间隔 (用户设定)
    base_interval: Duration,
    /// 当前生效的最小间隔 (根据阶段调整)
    min_interval: Duration,
    last_request: Option<Instant>,
    phase: TradingPhase,
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        let base = Duration::from_millis(min_interval_ms);
        Self {
            enabled: AtomicBool::new(min_interval_ms > 0),
            inner: Mutex::new(RateLimiterInner {
                base_interval: base,
                min_interval: base,
                last_request: None,
                phase: TradingPhase::Trading,
            }),
        }
    }

    /// 等待直到可以发送下一个请求
    pub fn wait(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(last) = inner.last_request {
            let elapsed = last.elapsed();
            if elapsed < inner.min_interval {
                std::thread::sleep(inner.min_interval - elapsed);
            }
        }
        inner.last_request = Some(Instant::now());
    }

    /// 设置每秒请求数 (0 = 禁用, 超过 200 自动降为 200)
    pub fn set_rps(&self, rps: u32) {
        let mut inner = self.inner.lock().unwrap();
        if rps == 0 {
            self.enabled.store(false, Ordering::Relaxed);
        } else {
            let capped = rps.min(MAX_RATE_LIMIT);
            inner.base_interval = Duration::from_millis(1000 / capped as u64);
            // 重新应用当前阶段的乘数
            let mult = PHASE_MULTIPLIER[inner.phase as usize].0;
            let adjusted_ms = (inner.base_interval.as_millis() as f64 / mult) as u64;
            inner.min_interval = Duration::from_millis(adjusted_ms.max(1));
            self.enabled.store(true, Ordering::Relaxed);
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// 设置交易阶段，自动调整限流强度
    ///
    /// - `Trading`: 使用基准限流 (最保守)
    /// - `PrePost`: 基准限流 / 2
    /// - `Closed`:  基准限流 / 4
    pub fn set_phase(&self, phase: TradingPhase) {
        let mut inner = self.inner.lock().unwrap();
        let mult = PHASE_MULTIPLIER[phase as usize].0;
        let adjusted_ms = (inner.base_interval.as_millis() as f64 / mult) as u64;
        inner.min_interval = Duration::from_millis(adjusted_ms.max(1));
        inner.phase = phase;
    }

    /// 获取当前阶段
    pub fn phase(&self) -> TradingPhase {
        self.inner.lock().unwrap().phase
    }

    /// 自动检测并设置交易阶段
    pub fn auto_detect_phase(&self) -> TradingPhase {
        let phase = detect_trading_phase();
        self.set_phase(phase);
        phase
    }
}

// ================================================================
// 股票代码编解码
// ================================================================

/// 股票代码 → 6 字节定长数组
pub fn code_bytes(code: &str) -> [u8; 6] {
    let mut buf = [0u8; 6];
    let bytes = code.as_bytes();
    let len = bytes.len().min(6);
    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

// ================================================================
// 市场代码自动识别 (F10 / Profile 共享)
// ================================================================

/// 自动识别市场代码
///
/// 根据股票代码首字符判断市场:
/// - '6' → 上海 (1)
/// - '0' | '3' → 深圳 (0)
///
/// # Errors
/// 代码长度不为 6 位或首字符无法识别时返回错误。
pub fn auto_market(code: &str) -> Result<u8> {
    if code.len() != 6 {
        return Err(crate::error_codes::ErrorCode::INVALID_STOCK_CODE
            .err(format!("{} (必须为 6 位数字)", code)));
    }
    match code.chars().next() {
        Some('6') => Ok(MARKET_SH),
        Some('0') | Some('3') => Ok(MARKET_SZ),
        _ => Err(crate::error_codes::ErrorCode::UNKNOWN_CODE_FORMAT
            .err(format!("无法自动识别市场代码: {}", code))),
    }
}

// ================================================================
// GBK 编码 (F10 / Profile 共享)
// ================================================================

/// 编码字符串为 GBK 字节
pub fn encode_gbk(s: &str) -> Result<Vec<u8>> {
    let (encoded, _, _) = encoding_rs::GBK.encode(s);
    Ok(encoded.into_owned())
}

/// 编码字符串为 GBK 字节并填充到指定长度
pub fn encode_gbk_padded(s: &str, target_len: usize) -> Result<Vec<u8>> {
    let mut bytes = encode_gbk(s)?;
    if bytes.len() < target_len {
        bytes.resize(target_len, 0);
    } else if bytes.len() > target_len {
        bytes.truncate(target_len);
    }
    Ok(bytes)
}

// ================================================================
// 请求包构建
// ================================================================

/// 构建 security bars 请求包
pub fn build_security_bars_packet(
    category: u8,
    market: u8,
    code: &str,
    start: u32,
    count: u16,
    fq: u8,
) -> Vec<u8> {
    let code_buf = code_bytes(code);
    let mut pkt = Vec::with_capacity(38);
    pkt.extend_from_slice(&0x010Cu16.to_le_bytes());
    pkt.extend_from_slice(&0x01016408u32.to_le_bytes());
    pkt.extend_from_slice(&0x001Cu16.to_le_bytes());
    pkt.extend_from_slice(&0x001Cu16.to_le_bytes());
    pkt.extend_from_slice(&CMD_SECURITY_BARS.to_le_bytes());
    pkt.extend_from_slice(&(market as u16).to_le_bytes());
    pkt.extend_from_slice(&code_buf);
    pkt.extend_from_slice(&(category as u16).to_le_bytes());
    pkt.extend_from_slice(&(fq as u16).to_le_bytes());
    pkt.extend_from_slice(&(start as u16).to_le_bytes());
    pkt.extend_from_slice(&count.to_le_bytes());
    pkt.extend_from_slice(&0u32.to_le_bytes());
    pkt.extend_from_slice(&0u32.to_le_bytes());
    pkt.extend_from_slice(&0u16.to_le_bytes());
    pkt
}

/// 构建 index bars 请求包 (与 security bars 格式相同, 语义区分)
pub fn build_index_bars_packet(
    category: u8,
    market: u8,
    code: &str,
    start: u32,
    count: u16,
    fq: u8,
) -> Vec<u8> {
    build_security_bars_packet(category, market, code, start, count, fq)
}

/// A server fault observed on 2026-09-23: count=800 (20 03) with no row bytes.
/// A row needs at least 16 bytes after the two-byte count header. Zero rows
/// remains a valid empty response and follows the existing retry policy.
pub fn declares_rows_without_payload(body: &[u8]) -> bool {
    if body.len() >= 18 {
        return false;
    }
    if body.len() < 2 {
        return true;
    }
    u16::from_le_bytes([body[0], body[1]]) > 0
}

// ================================================================
// 握手 / 响应处理
// ================================================================

/// 执行 TDX 三步握手协议 (同步)
pub fn perform_handshake(conn: &mut TcpConnection) -> Result<()> {
    for cmd in &[SETUP_CMD1, SETUP_CMD2, SETUP_CMD3] {
        conn.send(cmd)?;
        let (head, body) = read_response_raw(conn)?;
        if head.zip_size != head.unzip_size {
            let _ = decompress_zlib(&body)?;
        }
    }
    Ok(())
}

/// 从 TCP 连接读取响应头 + 压缩体
fn read_response_raw(conn: &mut TcpConnection) -> Result<(ResponseHeader, Vec<u8>)> {
    let head_buf = conn.recv(RSP_HEADER_LEN)?;
    let header = ResponseHeader::parse(&head_buf)?;
    let zip_size = header.zip_size as usize;
    let mut body = Vec::with_capacity(zip_size);
    while body.len() < zip_size {
        let chunk = conn.recv(zip_size - body.len())?;
        body.extend_from_slice(&chunk);
    }
    Ok((header, body))
}

/// zlib 解压
pub fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| crate::error_codes::ErrorCode::DECOMPRESS_FAILED.err(format!("{}", e)))?;
    Ok(decompressed)
}

// ================================================================
// 复权上下文拉取
// ================================================================

/// 为复权计算获取额外的历史 K 线
///
/// 当除权除息事件早于请求的 K 线数据时，向后翻页拉取更早的 K 线用于因子计算。
/// `send_fn`: 发包回调 (适配不同客户端连接模型)
pub fn fetch_context_bars_for_adjust<F: Fn(&[u8]) -> Result<Vec<u8>>>(
    send_fn: F,
    category: u8,
    market: u8,
    code: &str,
    bars: &[SecurityBar],
    xdxr: &[XdXrInfo],
) -> Vec<SecurityBar> {
    fetch_context_bars_for_adjust_with_tier(
        send_fn,
        category,
        market,
        code,
        bars,
        xdxr,
        FqContextTier::default(),
    )
}

/// 为复权计算获取额外的历史 K 线上下文 (可指定档位)
///
/// `tier` 控制最大翻页数: Low=3页(2400根), Mid=6页(4800根), High=9页(7200根)
pub fn fetch_context_bars_for_adjust_with_tier<F: Fn(&[u8]) -> Result<Vec<u8>>>(
    send_fn: F,
    category: u8,
    market: u8,
    code: &str,
    bars: &[SecurityBar],
    xdxr: &[XdXrInfo],
    tier: FqContextTier,
) -> Vec<SecurityBar> {
    if bars.is_empty() || xdxr.is_empty() {
        return Vec::new();
    }

    let earliest_event = xdxr
        .iter()
        .filter(|x| x.category == 1)
        .map(|x| x.year * 10000 + x.month * 100 + x.day)
        .min();

    let Some(ee_date) = earliest_event else {
        return Vec::new();
    };

    let first_bar_date = bars[0].year * 10000 + bars[0].month * 100 + bars[0].day;

    if first_bar_date <= ee_date {
        return Vec::new();
    }

    let max_per_page = MAX_KLINE_COUNT as u32;
    let max_pages = tier.pages();
    let mut context = Vec::new();
    let mut offset = max_per_page;

    for _page in 0..max_pages {
        let pkt = build_security_bars_packet(category, market, code, offset, MAX_KLINE_COUNT, 0);
        let body = match send_fn(&pkt) {
            Ok(b) => b,
            Err(_) => break,
        };
        let batch = match parse_security_bars(&body, category) {
            Ok(b) => b,
            Err(_) => break,
        };
        if batch.is_empty() {
            break;
        }

        let batch_first_date =
            batch[0].year as u32 * 10000 + batch[0].month as u32 * 100 + batch[0].day as u32;

        let len_before = context.len();
        context.splice(0..0, batch);

        if batch_first_date <= ee_date {
            break;
        }

        offset += max_per_page;
        if context.len() == len_before {
            break;
        }
    }

    context
}

// ================================================================
// 单元测试
// ================================================================

#[cfg(test)]
#[path = "../../tests/internal/net_utils.rs"]
mod tests;
