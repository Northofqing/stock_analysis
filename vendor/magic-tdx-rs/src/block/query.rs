//! 板块查询引擎
//!
//! 提供板块搜索、列表、成分查询功能。
//! 数据由调用方传入 (来自网络或本地文件)。

use std::collections::HashSet;

use crate::error::{Result, TdxError};
use crate::reader::block::BlockRecord;

use super::types::{BlockConstituents, BlockInfo, BlockType};

/// 搜索输入最大长度
const MAX_SEARCH_LEN: usize = 20;

/// 搜索结果最大数量
const MAX_RESULTS: usize = 200;

/// 指数代码到板块名称的映射
///
/// block_zs.dat 按板块名称组织，不按指数代码索引。
/// 仅收录常用指数，完整列表可通过 `list_blocks(BlockType::Index)` 获取。
pub fn index_code_to_name(code: &str) -> Option<&'static str> {
    match code {
        // ── 宽基指数 ──
        "000001" => Some("上证指数"),
        "399001" => Some("深证成指"),
        "399006" => Some("创业板指"),
        "000300" => Some("沪深300"),
        "000016" => Some("上证50"),
        "000905" => Some("中证500"),
        "000852" => Some("中证1000"),
        "000688" => Some("科创50"),
        "399673" => Some("创业板50"),
        "399005" => Some("中小100"),
        "399300" => Some("沪深300"),
        // ── 上证系列 ──
        "000010" => Some("上证180"),
        "000015" => Some("红利指数"),
        "000033" => Some("上证380"),
        "000062" => Some("上证红利"),
        // ── 深证系列 ──
        "399004" => Some("深证100R"),
        "399007" => Some("深证300"),
        "399008" => Some("中小板指"),
        "399106" => Some("深证综指"),
        _ => None,
    }
}

/// 板块查询引擎
pub struct BlockQuery;

impl Default for BlockQuery {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockQuery {
    pub const fn new() -> Self {
        Self
    }

    /// 查询指数成分 (仅精确代码匹配)
    pub fn get_index_constituents(
        &self,
        code: &str,
        raw_data: &[BlockRecord],
    ) -> Result<BlockConstituents> {
        let block_name = index_code_to_name(code)
            .ok_or_else(|| TdxError::InvalidData(format!("unknown index code '{}'", code)))?;

        let codes: Vec<String> = raw_data
            .iter()
            .filter(|r| r.blockname == block_name)
            .map(|r| r.code.clone())
            .collect();

        if codes.is_empty() {
            return Err(TdxError::InvalidData(format!(
                "no constituents found for index '{}' ({})",
                code, block_name
            )));
        }

        Ok(BlockConstituents {
            block_name: block_name.to_string(),
            block_type: BlockType::Index,
            codes,
        })
    }

    /// 按名称搜索板块 (行业/概念)
    ///
    /// 使用 `contains()` 做子串匹配，非正则。
    pub fn search_blocks(
        &self,
        keyword: &str,
        block_type: BlockType,
        raw_data: &[BlockRecord],
    ) -> Result<Vec<BlockInfo>> {
        if keyword.is_empty() {
            return Err(TdxError::InvalidData(
                "search keyword cannot be empty".into(),
            ));
        }
        if keyword.len() > MAX_SEARCH_LEN {
            return Err(TdxError::InvalidData(format!(
                "search keyword too long (max {} chars)",
                MAX_SEARCH_LEN
            )));
        }

        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for record in raw_data {
            if record.blockname.contains(keyword) && !seen.contains(&record.blockname) {
                seen.insert(record.blockname.clone());
                let count = raw_data
                    .iter()
                    .filter(|r| r.blockname == record.blockname)
                    .count();
                results.push(BlockInfo {
                    name: record.blockname.clone(),
                    block_type,
                    stock_count: count,
                });
                if results.len() >= MAX_RESULTS {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// 列出指定类型的所有板块
    pub fn list_blocks(&self, block_type: BlockType, raw_data: &[BlockRecord]) -> Vec<BlockInfo> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for record in raw_data {
            if !seen.contains(&record.blockname) {
                seen.insert(record.blockname.clone());
                let count = raw_data
                    .iter()
                    .filter(|r| r.blockname == record.blockname)
                    .count();
                results.push(BlockInfo {
                    name: record.blockname.clone(),
                    block_type,
                    stock_count: count,
                });
            }
        }

        results
    }

    /// 获取板块成分股
    pub fn get_block_constituents(
        &self,
        block_name: &str,
        block_type: BlockType,
        raw_data: &[BlockRecord],
    ) -> Result<BlockConstituents> {
        let codes: Vec<String> = raw_data
            .iter()
            .filter(|r| r.blockname == block_name)
            .map(|r| r.code.clone())
            .collect();

        if codes.is_empty() {
            return Err(TdxError::InvalidData(format!(
                "block '{}' not found or has no constituents",
                block_name
            )));
        }

        Ok(BlockConstituents {
            block_name: block_name.to_string(),
            block_type,
            codes,
        })
    }
}

/// 全局 BlockQuery 实例
static BLOCK_QUERY: BlockQuery = BlockQuery::new();

/// 获取全局 BlockQuery 实例
pub fn block_query() -> &'static BlockQuery {
    &BLOCK_QUERY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_records() -> Vec<BlockRecord> {
        vec![
            BlockRecord {
                blockname: "沪深300".into(),
                block_type: 2,
                code_index: 0,
                code: "600519".into(),
            },
            BlockRecord {
                blockname: "沪深300".into(),
                block_type: 2,
                code_index: 1,
                code: "000858".into(),
            },
            BlockRecord {
                blockname: "上证50".into(),
                block_type: 2,
                code_index: 0,
                code: "600028".into(),
            },
            BlockRecord {
                blockname: "新能源".into(),
                block_type: 2,
                code_index: 0,
                code: "300750".into(),
            },
            BlockRecord {
                blockname: "新能源".into(),
                block_type: 2,
                code_index: 1,
                code: "002594".into(),
            },
            BlockRecord {
                blockname: "华为概念".into(),
                block_type: 2,
                code_index: 0,
                code: "002230".into(),
            },
        ]
    }

    #[test]
    fn test_index_code_lookup() {
        assert_eq!(index_code_to_name("000300"), Some("沪深300"));
        assert_eq!(index_code_to_name("000016"), Some("上证50"));
        assert_eq!(index_code_to_name("999999"), None);
    }

    #[test]
    fn test_get_index_constituents() {
        let records = make_test_records();
        let q = BlockQuery::new();
        let result = q.get_index_constituents("000300", &records).unwrap();
        assert_eq!(result.block_name, "沪深300");
        assert_eq!(result.codes.len(), 2);
    }

    #[test]
    fn test_get_index_constituents_unknown() {
        let records = make_test_records();
        let q = BlockQuery::new();
        assert!(q.get_index_constituents("999999", &records).is_err());
    }

    #[test]
    fn test_search_blocks() {
        let records = make_test_records();
        let q = BlockQuery::new();
        let results = q
            .search_blocks("新", BlockType::Industry, &records)
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_blocks_empty_keyword() {
        let records = make_test_records();
        let q = BlockQuery::new();
        assert!(q.search_blocks("", BlockType::Industry, &records).is_err());
    }

    #[test]
    fn test_search_blocks_long_keyword() {
        let records = make_test_records();
        let q = BlockQuery::new();
        let long_keyword = "a".repeat(MAX_SEARCH_LEN + 1);
        assert!(q
            .search_blocks(&long_keyword, BlockType::Industry, &records)
            .is_err());
    }

    #[test]
    fn test_list_blocks() {
        let records = make_test_records();
        let q = BlockQuery::new();
        let results = q.list_blocks(BlockType::Industry, &records);
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_get_block_constituents() {
        let records = make_test_records();
        let q = BlockQuery::new();
        let result = q
            .get_block_constituents("新能源", BlockType::Industry, &records)
            .unwrap();
        assert_eq!(result.codes.len(), 2);
    }

    #[test]
    fn test_get_block_constituents_not_found() {
        let records = make_test_records();
        let q = BlockQuery::new();
        assert!(q
            .get_block_constituents("不存在", BlockType::Industry, &records)
            .is_err());
    }

    #[test]
    fn test_block_type_from_str() {
        assert_eq!(BlockType::from_str("index"), Some(BlockType::Index));
        assert_eq!(BlockType::from_str("industry"), Some(BlockType::Industry));
        assert_eq!(BlockType::from_str("concept"), Some(BlockType::Concept));
        assert_eq!(BlockType::from_str("unknown"), None);
    }
}
