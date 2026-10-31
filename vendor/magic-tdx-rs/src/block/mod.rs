//! 板块查询模块
//!
//! 提供指数成分、行业板块、概念板块的查询功能。
//!
//! ## 模块结构
//!
//! | 文件 | 说明 |
//! |------|------|
//! | `types.rs` | 数据类型定义 (BlockType, BlockInfo, BlockConstituents) |
//! | `query.rs` | 查询引擎 (搜索、列表、成分查询) |
//! | `client.rs` | 板块专用客户端 (内置限制，禁用分时/逐笔) |
//!
//! ## 数据来源
//!
//! | 文件 | 类型 | 板块数 | 说明 |
//! |------|------|:------:|------|
//! | `block_zs.dat` | 指数成分 | 110 | 沪深300、上证50等 |
//! | `block_fg.dat` | 行业板块 | 155 | 通达信板块标签 (非中证/申万) |
//! | `block_gn.dat` | 概念板块 | 270 | 一带一路、华为概念等 |
//!
//! ## 客户端限制
//!
//! `TdxBlockClient` 内置板块查询限制，避免误用通用客户端:
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
//!
//! ## 安全
//!
//! - 搜索在内存数据上执行 (非数据库)，无 SQL 注入风险
//! - 输入长度限制 20 字符
//! - 结果数量限制 200 条
//! - 使用 `contains()` 而非正则，避免正则注入

pub mod client;
pub mod query;
pub mod types;

pub use client::TdxBlockClient;
pub use query::{block_query, index_code_to_name, BlockQuery};
pub use types::{BlockConstituents, BlockFileSnapshot, BlockInfo, BlockType};
