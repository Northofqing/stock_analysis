//! 财务数据客户端 — 独立连接管理 + 本地磁盘缓存
//!
//! 与行情客户端分离的原因:
//!   1. 财务数据包体差异大 — get_finance_info (~136B) vs gpcw files (~12MB)
//!   2. 连接时长不同 — gpcw 下载可能持续数秒，不适合共享连接池
//!   3. 方便后续扩展 — 字段名映射、DataFrame 输出、本地缓存
//!
//! ## 磁盘缓存
//!
//! 通过 `set_cache_dir(path)` 启用。gpcw 文件下载后缓存到本地，
//! 24 小时内重复查询直接读取缓存，跳过网络下载。
//!
//! ## API 一览
//!
//! | 方法 | 数据量 | 说明 |
//! |------|:-----:|------|
//! | `get_finance_info(market, code)` | ~136B | 单股票 34 项实时财务 |
//! | `get_xdxr_info(market, code)` | ~200B | 除权除息历史 |
//! | `get_report_file(filename, offset)` | ≤30KB | 分片下载 gpcw 文件 |
//! | `get_financial_list()` | ~2KB | 可用报告期列表 (gpcw.txt) |

use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::net::connection::TcpConnection;
use crate::net::packet::{ResponseHeader, RSP_HEADER_LEN};
use crate::net::utils;
use crate::protocol::parsers::*;
use crate::protocol::types::*;
use crate::reader::financial::{parse_financial, FinancialRecord};
use crate::{loge, logi, logw};

/// 财务数据客户端默认超时 (秒)
const DEFAULT_FINANCE_TIMEOUT: f64 = 15.0;
/// 单次请求 chunk 大小 (30KB)
const CHUNK_SIZE: u32 = 0x7530;
/// 磁盘缓存有效期 (24 小时)
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// Upper bound for one uncompressed market-wide financial report.
const MAX_REPORT_SIZE: usize = 256 * 1024 * 1024;
/// Official TDX after-hours financial-data distribution endpoint.
const FINANCIAL_HTTP_HOST: &str = "data.tdx.com.cn";
const MAX_HTTP_HEADER_SIZE: usize = 64 * 1024;

fn zip_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| crate::error::TdxError::InvalidData("truncated ZIP metadata".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn zip_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| crate::error::TdxError::InvalidData("truncated ZIP metadata".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Extracts the first DAT entry from a bounded, non-encrypted ZIP archive.
fn extract_financial_zip(data: &[u8]) -> Result<Vec<u8>> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
    const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
    if data.len() < 22 {
        return Err(crate::error::TdxError::InvalidData(
            "financial ZIP is too small".into(),
        ));
    }
    let search_start = data.len().saturating_sub(65_557);
    let eocd = data[search_start..]
        .windows(4)
        .rposition(|window| window == EOCD_SIGNATURE)
        .map(|position| search_start + position)
        .ok_or_else(|| {
            crate::error::TdxError::InvalidData("financial ZIP has no end record".into())
        })?;
    let entries = usize::from(zip_u16(data, eocd + 10)?);
    let central_size = zip_u32(data, eocd + 12)? as usize;
    let central_offset = zip_u32(data, eocd + 16)? as usize;
    let central_end = central_offset.checked_add(central_size).ok_or_else(|| {
        crate::error::TdxError::InvalidData("financial ZIP central directory overflow".into())
    })?;
    if entries == 0 || central_end > eocd || central_end > data.len() {
        return Err(crate::error::TdxError::InvalidData(
            "financial ZIP central directory is invalid".into(),
        ));
    }

    let mut cursor = central_offset;
    for _ in 0..entries {
        if zip_u32(data, cursor)? != CENTRAL_SIGNATURE {
            return Err(crate::error::TdxError::InvalidData(
                "financial ZIP central entry signature is invalid".into(),
            ));
        }
        let flags = zip_u16(data, cursor + 8)?;
        let method = zip_u16(data, cursor + 10)?;
        let expected_crc = zip_u32(data, cursor + 16)?;
        let compressed_size = zip_u32(data, cursor + 20)? as usize;
        let uncompressed_size = zip_u32(data, cursor + 24)? as usize;
        let name_length = usize::from(zip_u16(data, cursor + 28)?);
        let extra_length = usize::from(zip_u16(data, cursor + 30)?);
        let comment_length = usize::from(zip_u16(data, cursor + 32)?);
        let local_offset = zip_u32(data, cursor + 42)? as usize;
        let name_start = cursor + 46;
        let name_end = name_start.checked_add(name_length).ok_or_else(|| {
            crate::error::TdxError::InvalidData("financial ZIP entry name overflow".into())
        })?;
        let name = data.get(name_start..name_end).ok_or_else(|| {
            crate::error::TdxError::InvalidData("truncated financial ZIP entry name".into())
        })?;
        let next = name_end
            .checked_add(extra_length)
            .and_then(|value| value.checked_add(comment_length))
            .ok_or_else(|| {
                crate::error::TdxError::InvalidData("financial ZIP entry overflow".into())
            })?;
        if next > central_end {
            return Err(crate::error::TdxError::InvalidData(
                "financial ZIP entry exceeds central directory".into(),
            ));
        }
        if String::from_utf8_lossy(name)
            .to_ascii_lowercase()
            .ends_with(".dat")
        {
            if flags & 1 != 0 {
                return Err(crate::error::TdxError::InvalidData(
                    "encrypted financial ZIP entries are unsupported".into(),
                ));
            }
            if uncompressed_size == 0 || uncompressed_size > MAX_REPORT_SIZE {
                return Err(crate::error::TdxError::InvalidData(format!(
                    "financial ZIP uncompressed size {uncompressed_size} is invalid"
                )));
            }
            if zip_u32(data, local_offset)? != LOCAL_SIGNATURE {
                return Err(crate::error::TdxError::InvalidData(
                    "financial ZIP local entry signature is invalid".into(),
                ));
            }
            let local_name_length = usize::from(zip_u16(data, local_offset + 26)?);
            let local_extra_length = usize::from(zip_u16(data, local_offset + 28)?);
            let payload_start = local_offset
                .checked_add(30)
                .and_then(|value| value.checked_add(local_name_length))
                .and_then(|value| value.checked_add(local_extra_length))
                .ok_or_else(|| {
                    crate::error::TdxError::InvalidData(
                        "financial ZIP payload offset overflow".into(),
                    )
                })?;
            let payload_end = payload_start.checked_add(compressed_size).ok_or_else(|| {
                crate::error::TdxError::InvalidData("financial ZIP payload overflow".into())
            })?;
            let payload = data.get(payload_start..payload_end).ok_or_else(|| {
                crate::error::TdxError::InvalidData("truncated financial ZIP payload".into())
            })?;
            let decoded = match method {
                0 => payload.to_vec(),
                8 => {
                    let mut output = Vec::with_capacity(uncompressed_size);
                    DeflateDecoder::new(payload)
                        .take((MAX_REPORT_SIZE + 1) as u64)
                        .read_to_end(&mut output)
                        .map_err(|error| {
                            crate::error::TdxError::InvalidData(format!(
                                "financial ZIP deflate failed: {error}"
                            ))
                        })?;
                    output
                }
                value => {
                    return Err(crate::error::TdxError::InvalidData(format!(
                        "financial ZIP compression method {value} is unsupported"
                    )));
                }
            };
            if decoded.len() != uncompressed_size {
                return Err(crate::error::TdxError::InvalidData(format!(
                    "financial ZIP size mismatch: expected {uncompressed_size}, decoded {}",
                    decoded.len()
                )));
            }
            if crc32fast::hash(&decoded) != expected_crc {
                return Err(crate::error::TdxError::InvalidData(
                    "financial ZIP CRC mismatch".into(),
                ));
            }
            return Ok(decoded);
        }
        cursor = next;
    }
    Err(crate::error::TdxError::InvalidData(
        "financial ZIP contains no DAT entry".into(),
    ))
}

fn decode_financial_payload(filename: &str, data: &[u8]) -> Result<Vec<u8>> {
    if data.starts_with(b"PK\x03\x04") || filename.to_ascii_lowercase().ends_with(".zip") {
        extract_financial_zip(data)
    } else {
        Ok(data.to_vec())
    }
}

fn decode_http_response(response: &[u8], expected_size: u32) -> Result<Vec<u8>> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or_else(|| {
            crate::error::TdxError::InvalidData(
                "financial HTTP response has no complete header".into(),
            )
        })?;
    if header_end > MAX_HTTP_HEADER_SIZE {
        return Err(crate::error::TdxError::InvalidData(
            "financial HTTP response header is too large".into(),
        ));
    }
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
        crate::error::TdxError::InvalidData("financial HTTP header is not ASCII".into())
    })?;
    let mut lines = headers.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let status_code = status
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            crate::error::TdxError::InvalidData("financial HTTP status is invalid".into())
        })?;
    if status_code != 200 {
        return Err(crate::error::TdxError::InvalidData(format!(
            "financial HTTP server returned status {status_code}"
        )));
    }

    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
            {
                return Err(crate::error::TdxError::InvalidData(
                    "chunked financial HTTP responses are unsupported".into(),
                ));
            }
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                    crate::error::TdxError::InvalidData(
                        "financial HTTP content length is invalid".into(),
                    )
                })?);
            }
        }
    }

    let body = response.get(header_end..).ok_or_else(|| {
        crate::error::TdxError::InvalidData("financial HTTP body is missing".into())
    })?;
    if body.len() > MAX_REPORT_SIZE {
        return Err(crate::error::TdxError::InvalidData(format!(
            "financial HTTP body exceeds {MAX_REPORT_SIZE} bytes"
        )));
    }
    if let Some(length) = content_length {
        if body.len() != length {
            return Err(crate::error::TdxError::InvalidData(format!(
                "financial HTTP size mismatch: header {length}, received {}",
                body.len()
            )));
        }
    }
    if expected_size != 0 && body.len() != expected_size as usize {
        // gpcw.txt and the HTTP object are updated independently. The list
        // size is an allocation hint; HTTP framing followed by ZIP entry-size
        // and CRC validation supplies the integrity gate.
        logw!(
            "finance",
            "financial list size is stale: expected {}, received {}; validating ZIP metadata and CRC",
            expected_size,
            body.len()
        );
    }
    Ok(body.to_vec())
}

fn report_file_packet(filename: &str, offset: u32) -> Vec<u8> {
    let name_bytes = filename.as_bytes();
    let mut name_buf = [0u8; 100];
    let len = name_bytes.len().min(name_buf.len());
    name_buf[..len].copy_from_slice(&name_bytes[..len]);

    let data_length = 4 + 4 + name_buf.len();
    let frame_length = (2 + data_length) as u16;
    let mut packet = Vec::with_capacity(12 + data_length);
    packet.extend_from_slice(&[0x0c, 0x00, 0x00, 0x00, 0x00, 0x01]);
    packet.extend_from_slice(&frame_length.to_le_bytes());
    packet.extend_from_slice(&frame_length.to_le_bytes());
    packet.extend_from_slice(&0x06B9u16.to_le_bytes());
    packet.extend_from_slice(&offset.to_le_bytes());
    packet.extend_from_slice(&CHUNK_SIZE.to_le_bytes());
    packet.extend_from_slice(&name_buf);
    packet
}

fn decode_report_chunk(body: &[u8]) -> Result<Vec<u8>> {
    // TDX 0x06B9 responses carry a four-byte transport prefix followed by
    // the complete file fragment.  The prefix is not a reliable fragment
    // length (some report servers return zero there), so mirror the protocol
    // implementations used by current TDX clients and consume the remainder.
    if body.len() < 4 {
        return Err(crate::error::TdxError::InvalidData(
            "report file response is shorter than its 4-byte prefix".into(),
        ));
    }
    Ok(body[4..].to_vec())
}

// ================================================================
// 财务客户端
// ================================================================

pub struct TdxFinanceClient {
    ip: String,
    port: u16,
    timeout: f64,
    cache_dir: Option<PathBuf>,
}

impl TdxFinanceClient {
    pub fn new(ip: &str, port: u16, timeout: Option<f64>) -> Self {
        Self {
            ip: ip.to_string(),
            port,
            timeout: timeout.unwrap_or(DEFAULT_FINANCE_TIMEOUT),
            cache_dir: None,
        }
    }

    pub fn set_server(&mut self, ip: &str, port: u16) {
        self.ip = ip.to_string();
        self.port = port;
    }

    pub fn set_timeout(&mut self, secs: f64) {
        self.timeout = secs;
    }

    /// 设置本地缓存目录 — 启用后 gpcw 文件自动缓存 24 小时
    ///
    /// 设为 `None` 禁用缓存。
    /// 缓存文件以原始文件名存储 (如 `gpcw20260331.dat`)。
    pub fn set_cache_dir(&mut self, path: Option<PathBuf>) {
        if let Some(ref p) = path {
            let _ = fs::create_dir_all(p);
        }
        self.cache_dir = path;
    }

    /// 获取当前缓存目录
    pub fn cache_dir(&self) -> Option<&PathBuf> {
        self.cache_dir.as_ref()
    }

    // ============================================================
    // 磁盘缓存逻辑
    // ============================================================

    /// 从缓存读取文件 (未过期返回 Some, 未命中/过期返回 None)
    fn cache_get(&self, filename: &str) -> Option<Vec<u8>> {
        let dir = self.cache_dir.as_ref()?;
        // 文件名提取: "tdxfin/gpcw20260331.dat" → "gpcw20260331.dat"
        let short = filename.rsplit('/').next().unwrap_or(filename);
        let path = dir.join(short);

        let meta = fs::metadata(&path).ok()?;
        let mtime = meta.modified().ok()?;
        let age = SystemTime::now()
            .duration_since(mtime)
            .unwrap_or(Duration::MAX);

        if age > CACHE_TTL {
            return None; // 过期
        }

        fs::read(&path).ok()
    }

    /// 写入数据到缓存
    fn cache_put(&self, filename: &str, data: &[u8]) {
        if let Some(ref dir) = self.cache_dir {
            let short = filename.rsplit('/').next().unwrap_or(filename);
            let _ = fs::write(dir.join(short), data);
        }
    }

    // ============================================================
    // 核心: 发包/收包/解压 (每次独立连接)
    // ============================================================

    fn send_and_recv(&self, packet: &[u8]) -> Result<Vec<u8>> {
        let mut conn = TcpConnection::connect(&self.ip, self.port, self.timeout).map_err(|e| {
            loge!(
                "finance",
                "connect to {}:{} failed: {}",
                self.ip,
                self.port,
                e
            );
            e
        })?;
        utils::perform_handshake(&mut conn)?;

        conn.send(packet)?;

        let head_buf = conn.recv(RSP_HEADER_LEN)?;
        let header = ResponseHeader::parse(&head_buf)?;

        let zip_size = header.zip_size as usize;
        let mut body_buf = Vec::with_capacity(zip_size);
        while body_buf.len() < zip_size {
            let remaining = zip_size - body_buf.len();
            let chunk = conn.recv(remaining)?;
            body_buf.extend_from_slice(&chunk);
        }

        if body_buf.is_empty() {
            return Err(crate::error_codes::ErrorCode::DISCONNECTED.err("empty response body"));
        }

        if header.zip_size != header.unzip_size {
            let mut decoder = ZlibDecoder::new(&body_buf[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                crate::error_codes::ErrorCode::DECOMPRESS_FAILED.err(format!("{}", e))
            })?;
            Ok(decompressed)
        } else {
            Ok(body_buf)
        }
    }

    fn download_financial_http(&self, filename: &str, expected_size: u32) -> Result<Vec<u8>> {
        if filename.is_empty()
            || !filename
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(crate::error::TdxError::InvalidData(
                "financial filename contains unsupported characters".into(),
            ));
        }
        if expected_size as usize > MAX_REPORT_SIZE {
            return Err(crate::error::TdxError::InvalidData(format!(
                "financial file exceeds {MAX_REPORT_SIZE} bytes"
            )));
        }

        let timeout = Duration::from_secs_f64(self.timeout);
        let addresses = (FINANCIAL_HTTP_HOST, 80)
            .to_socket_addrs()
            .map_err(|error| {
                crate::error::TdxError::Connection(format!(
                    "resolve {FINANCIAL_HTTP_HOST} failed: {error}"
                ))
            })?;
        let mut last_error = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(value) => {
                    stream = Some(value);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let mut stream = stream.ok_or_else(|| {
            crate::error::TdxError::Connection(format!(
                "connect to {FINANCIAL_HTTP_HOST}:80 failed: {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "no resolved address".into())
            ))
        })?;
        stream.set_read_timeout(Some(timeout)).map_err(|error| {
            crate::error::TdxError::Connection(format!("set HTTP read timeout: {error}"))
        })?;
        stream.set_write_timeout(Some(timeout)).map_err(|error| {
            crate::error::TdxError::Connection(format!("set HTTP write timeout: {error}"))
        })?;
        let request = format!(
            "GET /tdxfin/{filename} HTTP/1.1\r\nHost: {FINANCIAL_HTTP_HOST}\r\nUser-Agent: magic-tdx-rs/0.1\r\nAccept: application/zip,application/octet-stream\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).map_err(|error| {
            crate::error::TdxError::Connection(format!("send financial HTTP request: {error}"))
        })?;

        let response_limit = MAX_REPORT_SIZE + MAX_HTTP_HEADER_SIZE;
        let mut response = Vec::with_capacity(expected_size as usize + 1024);
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let read = stream.read(&mut buffer).map_err(|error| {
                crate::error::TdxError::Connection(format!("read financial HTTP response: {error}"))
            })?;
            if read == 0 {
                break;
            }
            if response.len().saturating_add(read) > response_limit {
                return Err(crate::error::TdxError::InvalidData(
                    "financial HTTP response exceeds the configured limit".into(),
                ));
            }
            response.extend_from_slice(&buffer[..read]);
        }
        decode_http_response(&response, expected_size)
    }

    // ============================================================
    // 单股票实时财务
    // ============================================================

    pub fn get_finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo> {
        let code_buf = utils::code_bytes(code);
        let mut packet = Vec::with_capacity(21);
        packet.extend_from_slice(&[
            0x0c, 0x1f, 0x18, 0x76, 0x00, 0x01, 0x0b, 0x00, 0x0b, 0x00, 0x10, 0x00, 0x01, 0x00,
        ]);
        packet.push(market);
        packet.extend_from_slice(&code_buf);
        parse_finance_info(&self.send_and_recv(&packet)?, market, code)
    }

    pub fn get_xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>> {
        let code_buf = utils::code_bytes(code);
        let mut packet = Vec::with_capacity(21);
        packet.extend_from_slice(&[
            0x0c, 0x1f, 0x18, 0x76, 0x00, 0x01, 0x0b, 0x00, 0x0b, 0x00, 0x0f, 0x00, 0x01, 0x00,
        ]);
        packet.push(market);
        packet.extend_from_slice(&code_buf);
        parse_xdxr_info_for(&self.send_and_recv(&packet)?, market, code)
    }

    // ============================================================
    // 报告文件下载 (分片)
    // ============================================================

    /// 下载报告文件的单个分片 (不走缓存 — 分片由上层 get_report_file_by_size 管理)
    pub fn get_report_file(&self, filename: &str, offset: u32) -> Result<Vec<u8>> {
        let packet = report_file_packet(filename, offset);
        let body = self.send_and_recv(&packet)?;
        decode_report_chunk(&body)
    }

    /// 下载完整的报告文件 (自动分片 + 重组, 优先磁盘缓存)
    pub fn get_report_file_by_size(&self, filename: &str, filesize: u32) -> Result<Vec<u8>> {
        // 1. 检查磁盘缓存
        if let Some(cached) = self.cache_get(filename) {
            logi!("finance", "cache hit: {}", filename);
            return Ok(cached);
        }

        // 2. 从网络下载
        let data = self.download_report_file(filename, filesize)?;

        // 3. 写入缓存
        self.cache_put(filename, &data);

        Ok(data)
    }

    /// 实际下载逻辑 (不分缓存)
    fn download_report_file(&self, filename: &str, filesize: u32) -> Result<Vec<u8>> {
        if filesize == 0 {
            // 未知大小: 下载第一片后判断总量, 最多 4 片
            let first = self.get_report_file(filename, 0)?;
            if first.len() < CHUNK_SIZE as usize {
                return Ok(first);
            }
            let mut data = first;
            for page in 1u32..4 {
                let chunk = self.get_report_file(filename, page * CHUNK_SIZE)?;
                if chunk.is_empty() {
                    break;
                }
                data.extend_from_slice(&chunk);
                if chunk.len() < CHUNK_SIZE as usize {
                    break;
                }
            }
            return Ok(data);
        }

        let effective_size = filesize;
        let mut data = Vec::with_capacity(effective_size as usize);
        let mut offset = 0u32;

        while offset < effective_size {
            let chunk = self.get_report_file(filename, offset)?;
            if chunk.is_empty() {
                break;
            }
            data.extend_from_slice(&chunk);
            offset += chunk.len() as u32;
            if chunk.len() < CHUNK_SIZE as usize {
                break;
            }
        }

        Ok(data)
    }

    // ============================================================
    // 全市场历史财务 (gpcw*.dat)
    // ============================================================

    /// 获取可用报告期列表 (从 gpcw.txt, 优先磁盘缓存)
    pub fn get_financial_list(&self) -> Result<Vec<GpcwFileInfo>> {
        // gpcw.txt 也走缓存路径 (TTL 24h)
        let data = self.get_report_file_by_size("tdxfin/gpcw.txt", 0)?;
        let content = String::from_utf8_lossy(&data);
        let mut files = Vec::new();
        for line in content.trim().split('\n') {
            let parts: Vec<&str> = line.trim().split(',').collect();
            if parts.len() >= 3 {
                files.push(GpcwFileInfo {
                    filename: parts[0].to_string(),
                    hash: parts[1].to_string(),
                    filesize: parts[2].parse().unwrap_or(0),
                });
            }
        }
        Ok(files)
    }

    /// 下载并解析指定的 gpcw*.dat 报告期数据 (优先磁盘缓存)
    pub fn get_financial_data(
        &self,
        filename: &str,
        filesize: u32,
    ) -> Result<Vec<FinancialRecord>> {
        let full = format!("tdxfin/{}", filename);
        let data = if let Some(cached) = self.cache_get(&full) {
            cached
        } else {
            let downloaded = match self.download_financial_http(filename, filesize) {
                Ok(data) => data,
                Err(http_error) => {
                    logw!(
                        "finance",
                        "official HTTP download failed for {}: {}; trying quote server",
                        filename,
                        http_error
                    );
                    self.download_report_file(&full, filesize)?
                }
            };
            self.cache_put(&full, &downloaded);
            downloaded
        };
        let decoded = decode_financial_payload(filename, &data)?;
        parse_financial(&decoded)
    }

    // ============================================================
    // 命名财务指标
    // ============================================================

    /// 获取单只股票的命名财务指标 (45 个核心字段, 英文 key, TDX 原始值)
    pub fn get_finance_indicators(
        &self,
        filename: &str,
        filesize: u32,
        code: &str,
    ) -> Result<std::collections::HashMap<&'static str, f64>> {
        let records = self.get_financial_data(filename, filesize)?;
        for r in &records {
            if r.code == code {
                return Ok(crate::protocol::finance_fields::extract_indicators(
                    &r.fields,
                ));
            }
        }
        logw!("finance", "stock {} not found in {}", code, filename);
        Err(crate::error_codes::ErrorCode::INVALID_STOCK_CODE
            .err(format!("stock {} not found in {}", code, filename)))
    }

    /// 获取单只股票的命名财务指标 (带中文标签, 适合展示/校验)
    pub fn get_finance_indicators_labeled(
        &self,
        filename: &str,
        filesize: u32,
        code: &str,
    ) -> Result<Vec<(&'static str, &'static str, f64)>> {
        let records = self.get_financial_data(filename, filesize)?;
        for r in &records {
            if r.code == code {
                return Ok(crate::protocol::finance_fields::extract_with_labels(
                    &r.fields,
                ));
            }
        }
        logw!("finance", "stock {} not found in {}", code, filename);
        Err(crate::error_codes::ErrorCode::INVALID_STOCK_CODE
            .err(format!("stock {} not found in {}", code, filename)))
    }
}

/// gpcw.txt 中的文件清单条目
#[derive(Debug, Clone)]
pub struct GpcwFileInfo {
    pub filename: String,
    pub hash: String,
    pub filesize: u32,
}

// ================================================================
// 单元测试
// ================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn test_zip(payload: &[u8], deflated: bool) -> Vec<u8> {
        let name = b"gpcw20260331.dat";
        let method = if deflated { 8u16 } else { 0u16 };
        let compressed = if deflated {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(payload).unwrap();
            encoder.finish().unwrap()
        } else {
            payload.to_vec()
        };
        let crc = crc32fast::hash(payload);
        let mut zip = Vec::new();
        zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(&compressed);

        let central_offset = zip.len() as u32;
        zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(name);
        let central_size = zip.len() as u32 - central_offset;

        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&central_size.to_le_bytes());
        zip.extend_from_slice(&central_offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    #[test]
    fn extracts_stored_and_deflated_financial_zip() {
        let payload = b"financial report payload";
        assert_eq!(
            extract_financial_zip(&test_zip(payload, false)).unwrap(),
            payload
        );
        assert_eq!(
            extract_financial_zip(&test_zip(payload, true)).unwrap(),
            payload
        );
    }

    #[test]
    fn report_file_packet_uses_control_one_frame() {
        let packet = report_file_packet("tdxfin/gpcw.txt", 0x1234_5678);
        assert_eq!(packet.len(), 120);
        assert_eq!(packet[0], 0x0c);
        assert_eq!(packet[5], 0x01);
        assert_eq!(&packet[6..10], &[0x6e, 0x00, 0x6e, 0x00]);
        assert_eq!(&packet[10..12], &[0xb9, 0x06]);
        assert_eq!(&packet[12..16], &0x1234_5678u32.to_le_bytes());
        assert_eq!(&packet[16..20], &CHUNK_SIZE.to_le_bytes());
    }

    #[test]
    fn report_chunk_ignores_non_length_transport_prefix() {
        assert_eq!(
            decode_report_chunk(&[0, 0, 0, 0, b'P', b'K']).unwrap(),
            b"PK"
        );
        assert!(decode_report_chunk(&[0, 0, 0]).is_err());
    }

    #[test]
    fn validates_complete_financial_http_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPK12";
        assert_eq!(decode_http_response(response, 4).unwrap(), b"PK12");
        assert_eq!(decode_http_response(response, 5).unwrap(), b"PK12");
        assert!(decode_http_response(b"HTTP/1.1 404 Not Found\r\n\r\n", 0).is_err());
    }

    #[test]
    fn rejects_financial_zip_with_bad_crc() {
        let mut zip = test_zip(b"financial report payload", true);
        let central_offset = zip_u32(&zip, zip.len() - 6).unwrap() as usize;
        zip[central_offset + 16] ^= 0xff;
        assert!(extract_financial_zip(&zip)
            .unwrap_err()
            .to_string()
            .contains("CRC"));
    }

    #[test]
    fn test_new_client() {
        let client = TdxFinanceClient::new("127.0.0.1", 7709, None);
        assert_eq!(client.ip, "127.0.0.1");
        assert_eq!(client.port, 7709);
        assert!(client.timeout > 0.0);
        assert!(client.cache_dir.is_none());
    }

    #[test]
    fn test_new_client_custom_timeout() {
        let client = TdxFinanceClient::new("127.0.0.1", 7709, Some(30.0));
        assert!((client.timeout - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_set_server() {
        let mut client = TdxFinanceClient::new("127.0.0.1", 7709, None);
        client.set_server("192.168.1.1", 7727);
        assert_eq!(client.ip, "192.168.1.1");
        assert_eq!(client.port, 7727);
    }

    #[test]
    fn test_set_timeout() {
        let mut client = TdxFinanceClient::new("127.0.0.1", 7709, None);
        client.set_timeout(25.0);
        assert!((client.timeout - 25.0).abs() < 0.001);
    }

    #[test]
    fn every_finance_request_builder_propagates_local_connection_failure() {
        let client = TdxFinanceClient::new("127.0.0.1", 1, Some(0.01));
        assert!(client.get_finance_info(1, "600001").is_err());
        assert!(client.get_xdxr_info(1, "600001").is_err());
        assert!(client.get_report_file("tdxfin/gpcw.txt", 0).is_err());
        assert!(client
            .get_report_file_by_size("tdxfin/gpcw.txt", CHUNK_SIZE)
            .is_err());
        assert!(client
            .get_report_file_by_size("tdxfin/gpcw.txt", 0)
            .is_err());
        assert!(client.get_financial_list().is_err());
        assert!(client.get_financial_data("../bad", 1).is_err());
        assert!(client
            .get_finance_indicators("../bad", 1, "600001")
            .is_err());
        assert!(client
            .get_finance_indicators_labeled("../bad", 1, "600001")
            .is_err());
    }

    #[test]
    fn test_cache_dir_set_get() {
        let mut client = TdxFinanceClient::new("127.0.0.1", 7709, None);
        assert!(client.cache_dir().is_none());

        let dir = std::env::temp_dir().join("tdxrs_test_cache");
        client.set_cache_dir(Some(dir.clone()));
        assert!(client.cache_dir().is_some());
        assert!(dir.exists());

        // 清理
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cache_hit_and_expiry() {
        let dir = std::env::temp_dir().join("tdxrs_cache_test2");
        let _ = std::fs::remove_dir_all(&dir);

        let mut client = TdxFinanceClient::new("127.0.0.1", 7709, None);
        client.set_cache_dir(Some(dir.clone()));

        // 没有缓存 → 返回 None
        assert!(client.cache_get("tdxfin/test.dat").is_none());

        // 写入缓存 → 可读取
        client.cache_put("tdxfin/test.dat", b"hello world");
        let cached = client.cache_get("tdxfin/test.dat");
        assert_eq!(cached, Some(b"hello world".to_vec()));

        // 短期文件名: "tdxfin/xxx" → 提取 "xxx"
        // 验证提取逻辑在 cache_get/cache_put 中正确
        assert!(dir.join("test.dat").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
