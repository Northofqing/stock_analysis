//! 板块专用客户端
//!
//! 封装 TdxDirectClient，内置板块查询限制，避免误用通用客户端。
//!
//! ## 限制策略
//!
//! | 数据类型 | 限制 | 理由 |
//! |---------|------|------|
//! | K线 (日/周/月) | 无限制 | 数据量小，常用 |
//! | K线 (60min) | 默认 200，上限 800 | 高频需求少 |
//! | K线 (30min/15min/5min) | 默认 50，上限 200 | 高频数据量大 |
//! | K线 (1min) | **禁用** | 板块无此需求 |
//! | 分时数据 | **禁用** | 板块聚合数据无意义 |
//! | 逐笔成交 | **禁用** | 板块无买卖方向 |
//! | 实时行情 | 允许 | 单次查询，量小 |

use std::sync::Mutex;

use crate::error::{Result, TdxError};
use crate::net::direct_client::TdxDirectClient;
use crate::protocol::constants::DEFAULT_PORT;
use crate::protocol::types::{BlockInfoMeta, IndexBar, SecurityQuote};
use crate::reader::block::{parse_block, BlockRecord};

use super::types::BlockFileSnapshot;

const BLOCK_DOWNLOAD_CHUNK_SIZE: u32 = 0x7530;
const MAX_BLOCK_FILE_SIZE: u32 = 16 * 1024 * 1024;

fn supported_block_file(filename: &str) -> bool {
    matches!(
        filename,
        crate::protocol::constants::BLOCK_FG
            | crate::protocol::constants::BLOCK_GN
            | crate::protocol::constants::BLOCK_SZ
    )
}

fn validate_block_meta(filename: &str, meta: &BlockInfoMeta) -> Result<()> {
    if meta.size == 0 || meta.size > MAX_BLOCK_FILE_SIZE {
        return Err(TdxError::InvalidData(format!(
            "TDX block file {filename} has invalid source size {}",
            meta.size
        )));
    }
    if meta.hash_value.len() != 64 || !meta.hash_value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(TdxError::InvalidData(format!(
            "TDX block file {filename} has invalid source hash"
        )));
    }
    Ok(())
}

fn stable_block_snapshot(
    filename: &str,
    before: BlockInfoMeta,
    bytes: Vec<u8>,
    after: BlockInfoMeta,
) -> Result<BlockFileSnapshot> {
    validate_block_meta(filename, &before)?;
    validate_block_meta(filename, &after)?;
    if before.size != after.size || before.hash_value != after.hash_value {
        return Err(TdxError::InvalidData(format!(
            "TDX block file {filename} changed during download"
        )));
    }
    if bytes.len() != before.size as usize {
        return Err(TdxError::InvalidData(format!(
            "TDX block file {filename} is partial: expected {} bytes, received {}",
            before.size,
            bytes.len()
        )));
    }
    let records = parse_block(&bytes)?;
    if records.is_empty() {
        return Err(TdxError::InvalidData(format!(
            "TDX block file {filename} contains no source records"
        )));
    }
    Ok(BlockFileSnapshot {
        filename: filename.to_owned(),
        hash: before.hash_value,
        records,
    })
}

/// K线级别限制配置
struct KlineLimit {
    /// 默认返回条数
    default_count: u16,
    /// 最大允许条数
    max_count: u16,
    /// 是否允许查询
    allowed: bool,
}

/// 板块专用客户端
///
/// 内置板块查询限制，不暴露分时/逐笔等高频接口。
pub struct TdxBlockClient {
    client: Mutex<TdxDirectClient>,
}

impl TdxBlockClient {
    /// 创建板块客户端
    ///
    /// - `ip`: 服务器地址
    /// - `port`: 端口 (默认 7709)
    /// - `timeout`: 超时秒数 (默认 5.0)
    pub fn new(ip: &str, port: u16, timeout: f64) -> Self {
        Self {
            client: Mutex::new(TdxDirectClient::new(ip, port, timeout)),
        }
    }

    /// 使用默认端口和超时创建
    pub fn with_default(ip: &str) -> Self {
        Self::new(ip, DEFAULT_PORT, 5.0)
    }

    /// 更新服务器地址
    pub fn set_server(&self, ip: &str, port: u16) {
        self.client.lock().unwrap().set_server(ip, port);
    }

    /// 更新超时
    pub fn set_timeout(&self, timeout: f64) {
        self.client.lock().unwrap().set_timeout(timeout);
    }

    /// 获取 K 级别限制配置
    fn kline_limit(category: u8) -> KlineLimit {
        match category {
            // 日/周/月/季/年 — 无限制
            4 | 5 | 6 | 10 | 11 => KlineLimit {
                default_count: 100,
                max_count: 800,
                allowed: true,
            },
            // 60min — 默认 200
            3 => KlineLimit {
                default_count: 200,
                max_count: 800,
                allowed: true,
            },
            // 30min — 默认 50
            2 => KlineLimit {
                default_count: 50,
                max_count: 200,
                allowed: true,
            },
            // 15min — 默认 50
            1 => KlineLimit {
                default_count: 50,
                max_count: 200,
                allowed: true,
            },
            // 5min — 默认 50
            0 => KlineLimit {
                default_count: 50,
                max_count: 200,
                allowed: true,
            },
            // 1min (8) / 扩展1min (7) — 禁用
            7 | 8 => KlineLimit {
                default_count: 0,
                max_count: 0,
                allowed: false,
            },
            // 其他 — 按日线处理
            _ => KlineLimit {
                default_count: 100,
                max_count: 800,
                allowed: true,
            },
        }
    }

    // ================================================================
    // 板块 K 线 (核心功能)
    // ================================================================

    /// 获取板块 K 线数据
    ///
    /// 自动应用板块限制:
    /// - 日/周/月: 无限制 (max 800)
    /// - 分钟级: 默认 50，max 200
    /// - 1min: 禁用，返回错误
    ///
    /// `code`: 板块代码 (88xxxx)
    /// `category`: K线种类 (0=5min, 1=15min, 3=60min, 4=day, 5=week, 6=month)
    /// `count`: 请求条数 (0=使用默认值)
    pub fn get_block_bars(
        &self,
        category: u8,
        code: &str,
        start: u32,
        count: u16,
    ) -> Result<Vec<IndexBar>> {
        let limit = Self::kline_limit(category);

        if !limit.allowed {
            return Err(TdxError::InvalidData(format!(
                "category={} (1min/扩展1min) is not allowed for block queries",
                category
            )));
        }

        let actual_count = if count == 0 {
            limit.default_count
        } else {
            count.min(limit.max_count)
        };

        self.client
            .lock()
            .unwrap()
            .get_index_bars_inner(category, 1, code, start, actual_count, 0)
    }

    /// 获取板块 K 线 (使用默认条数)
    pub fn get_block_bars_default(&self, category: u8, code: &str) -> Result<Vec<IndexBar>> {
        self.get_block_bars(category, code, 0, 0)
    }

    // ================================================================
    // 板块实时行情
    // ================================================================

    /// 获取板块实时行情
    ///
    /// `codes`: 板块代码列表 (88xxxx)
    pub fn get_block_quotes(&self, codes: &[&str]) -> Result<Vec<SecurityQuote>> {
        let pairs: Vec<(u8, &str)> = codes.iter().map(|&c| (1u8, c)).collect();
        self.client
            .lock()
            .unwrap()
            .get_security_quotes_inner(&pairs)
    }

    // ================================================================
    // 板块列表 (从服务器下载 .dat 文件)
    // ================================================================

    /// 从服务器下载并解析板块文件
    ///
    /// `block_file`: 文件名，如 `"block_fg.dat"`, `"block_gn.dat"`, `"block_zs.dat"`
    ///
    /// 返回板块成分股级别的记录。同一板块名称会出现多次（每个成分股一条）。
    /// 使用 `BlockQuery::list_blocks()` 可按板块名称去重聚合。
    pub fn get_block_list(&self, block_file: &str) -> Result<Vec<BlockRecord>> {
        self.client
            .lock()
            .unwrap()
            .get_and_parse_block_info(block_file)
    }

    /// Downloads one complete block file and proves its source version stayed stable.
    pub fn get_block_snapshot(&self, block_file: &str) -> Result<BlockFileSnapshot> {
        if !supported_block_file(block_file) {
            return Err(TdxError::Unsupported(format!(
                "unsupported TDX block file {block_file:?}"
            )));
        }
        let client = self
            .client
            .lock()
            .map_err(|_| TdxError::Connection("TDX block client lock is poisoned".into()))?;
        let (before, bytes, after) = client.download_stable_block_file(
            block_file,
            BLOCK_DOWNLOAD_CHUNK_SIZE,
            MAX_BLOCK_FILE_SIZE,
        )?;
        validate_block_meta(block_file, &before)?;
        stable_block_snapshot(block_file, before, bytes, after)
    }

    /// 获取行业板块列表 (block_fg.dat)
    ///
    /// 返回筛选类标签板块，如融资融券、破净资产、高股息等。
    pub fn get_industry_blocks(&self) -> Result<Vec<BlockRecord>> {
        self.get_block_list(crate::protocol::constants::BLOCK_FG)
    }

    /// 获取概念板块列表 (block_gn.dat)
    ///
    /// 返回概念板块，如5G概念、一带一路、碳中和等。
    pub fn get_concept_blocks(&self) -> Result<Vec<BlockRecord>> {
        self.get_block_list(crate::protocol::constants::BLOCK_GN)
    }

    /// 获取指数成分列表 (block_zs.dat)
    ///
    /// 返回指数成分板块，如沪深300、上证50、创业板指等。
    pub fn get_index_blocks(&self) -> Result<Vec<BlockRecord>> {
        self.get_block_list(crate::protocol::constants::BLOCK_SZ)
    }

    // ================================================================
    // 不暴露的接口 (明确禁用)
    // ================================================================

    // 以下接口不暴露给板块客户端:
    // - get_minute_time_data: 板块分时数据无意义
    // - get_transaction_data: 板块逐笔成交无买卖方向
    // - get_security_bars: 应使用 get_block_bars (带限制)
    // - get_history_minute_time_data: 同上
    // - get_history_transaction_data: 同上
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::BlockInfoMeta;

    fn fixture_block_bytes() -> Vec<u8> {
        const HEADER_SIZE: usize = 384;
        const STOCK_AREA_SIZE: usize = 2_800;
        let mut bytes = vec![0; HEADER_SIZE];
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(b"Power\0\0\0\0");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(b"600396\0");
        bytes.resize(HEADER_SIZE + 2 + 9 + 4 + STOCK_AREA_SIZE, 0);
        bytes
    }

    fn fixture_meta(size: usize, hash_byte: char) -> BlockInfoMeta {
        BlockInfoMeta {
            size: u32::try_from(size).unwrap(),
            hash_value: std::iter::repeat_n(hash_byte, 64).collect(),
        }
    }

    #[test]
    fn block_snapshot_rejects_source_version_change() {
        let bytes = fixture_block_bytes();
        let before = fixture_meta(bytes.len(), 'a');
        let after = fixture_meta(bytes.len(), 'b');

        let error = stable_block_snapshot("block_fg.dat", before, bytes, after).unwrap_err();

        assert!(error.to_string().contains("changed during download"));
    }

    #[test]
    fn block_snapshot_preserves_exact_file_hash_and_rows() {
        let bytes = fixture_block_bytes();
        let before = fixture_meta(bytes.len(), 'a');
        let after = fixture_meta(bytes.len(), 'a');

        let snapshot = stable_block_snapshot("block_fg.dat", before, bytes, after).unwrap();

        assert_eq!(snapshot.filename, "block_fg.dat");
        assert_eq!(snapshot.hash, "a".repeat(64));
        assert_eq!(snapshot.records.len(), 1);
        assert_eq!(snapshot.records[0].blockname, "Power");
        assert_eq!(snapshot.records[0].code, "600396");
    }

    #[test]
    fn test_kline_limit_daily() {
        let limit = TdxBlockClient::kline_limit(4); // day
        assert!(limit.allowed);
        assert_eq!(limit.default_count, 100);
        assert_eq!(limit.max_count, 800);
    }

    #[test]
    fn test_kline_limit_5min() {
        let limit = TdxBlockClient::kline_limit(0); // 5min
        assert!(limit.allowed);
        assert_eq!(limit.default_count, 50);
        assert_eq!(limit.max_count, 200);
    }

    #[test]
    fn test_kline_limit_60min() {
        let limit = TdxBlockClient::kline_limit(3); // 60min
        assert!(limit.allowed);
        assert_eq!(limit.default_count, 200);
        assert_eq!(limit.max_count, 800);
    }

    #[test]
    fn test_kline_limit_1min_disabled() {
        let limit = TdxBlockClient::kline_limit(8); // 1min
        assert!(!limit.allowed);
    }

    #[test]
    fn test_new_client() {
        let client = TdxBlockClient::new("127.0.0.1", 7709, 5.0);
        // Just verify it compiles and creates
        let _ = client;
    }

    #[test]
    fn test_with_default() {
        let client = TdxBlockClient::with_default("127.0.0.1");
        let _ = client;
    }
}
