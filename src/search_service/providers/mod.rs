//! 搜索引擎 provider 实现集合
//!
//! 原 `search_service.rs` 中所有 provider 按引擎拆分。

pub mod general_web;

pub use general_web::GeneralWebSearchProvider;
