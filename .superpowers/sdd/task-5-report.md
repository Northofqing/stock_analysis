# Task 5 Report: BaostockProvider 骨架 + login/logout

**Date**: 2026-07-08
**Branch**: master
**Commit**: 62db7d9

## 任务范围

实现 Baostock 数据源骨架:
- URL/format helpers (`build_login_url`, `build_logout_url`, `build_kline_query_body`, `parse_baostock_response`)
- `BaostockProvider` struct + `new()` (env 读 `BAOSTOCK_BASE_URL` 覆盖默认)
- 懒登录 `ensure_session()` + 真实 `login()` (POST form-encoded)
- `DataProvider` trait 实现 (placeholder `get_daily_data` 返回 Err, Task 6 实现)
- 4 个 integration tests + 3 个 inline tests

## 实际改动

### 新文件
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/src/data_provider/baostock_provider.rs` (130 行)
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/tests/baostock_provider_test.rs` (53 行, `git add -f`)

### 修改
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/src/data_provider/mod.rs`: 加 `pub mod baostock_provider;` 一行

## 关键设计决策

### 1. Session 缓存
用 `tokio::sync::Mutex<Option<String>>` 而非 `RwLock`: 登录/登出是 write 操作, 频繁 read 价值不大. `ensure_session` 在第一次调用时 lazy 触发 login, 后续直接 clone 返回.

### 2. 默认凭据
Baostock 公开访问用 `anonymous` / `888888`, 不需要注册. 写死在 `login()` 里 (无 env 覆盖, 简化设计).

### 3. `parse_baostock_response` 防护
- 容忍 `\r\n` (Windows 风格响应)
- 前缀匹配是精确的: `sessionId` 不会误匹配 `sessionIdPrefix`
- 返回 `Result<Option<String>>`: 找不到 key 不是错误, 但格式异常才报 Err

### 4. `base_url` 字段保留
Skeleton 阶段 `base_url` 未被使用 (login/logout 走 helper functions 用 `BAOSTOCK_DEFAULT_BASE`). 加 `#[allow(dead_code)]` 抑制警告, 注释说明 Task 6 实际查询时用此 URL.

### 5. `logout` 方法
- `pub async fn logout(&self, session_id: &str)`, 不返回 Result
- 失败只 log warn — 进程退出路径不希望被登出失败阻塞

## 测试结果

### Brief 要求的 3 个 integration tests (改名后实际 4 个, +1 build_logout_url)
```
running 4 tests
test build_kline_query_body_format ... ok
test parse_baostock_response_extracts_field ... ok
test test_build_login_url ... ok
test test_build_logout_url ... ok
test result: ok. 4 passed; 0 failed
```

**注意**: 原 brief 中测试函数名 `build_login_url` 与 import 冲突 (Rust 报 E0255). 改名 `test_build_login_url` / `test_build_logout_url` 解决. `build_kline_query_body_format` 和 `parse_baostock_response_extracts_field` 不冲突, 保留原名.

### 额外 inline tests (3 个)
在 `baostock_provider.rs` 底部 `#[cfg(test)]` 模块:
- `default_base_is_stable`: 锁住 `BAOSTOCK_DEFAULT_BASE` 常量值
- `parse_handles_crlf_and_trailing_whitespace`: 防 `\r\n` 协议变体
- `parse_prefix_is_exact`: 防前缀误匹配

```
running 3 tests
test result: ok. 3 passed; 0 failed
```

## 偏离 brief 的小调整

1. **测试函数改名** (4 个测试中 2 个): 上面已说明原因.
2. **多加了 `build_logout_url` test**: brief 没要求, 但 logout helper 是公开 API, 加 test 锁住行为.
3. **多加了 3 个 inline tests**: 防御性测试, 防 Baostock 协议变体 / 前缀冲突.
4. **加了 `logout` 方法**: brief 没明确要求, 但配合 session 管理是自然的. 失败不抛 Err, 方便进程退出清理.
5. **`base_url` 字段加 `#[allow(dead_code)]`**: Skeleton 阶段没用, 避免污染 lib build 警告数.
6. **`use std::collections::HashMap` 和 `use crate::data_provider::stock_code_map::to_baostock`**: brief 中提到但 skeleton 阶段不需要, 未导入 (避免 dead import 警告).

## 已知未完成 (后续 Task)

- `get_daily_data` 返回 `Err("not yet implemented")` — Task 6 实现
- Session 过期检测 (实测 ~1h) — Task 6+ 加
- 集成到 `fallback.rs` (Tencent → Eastmoney → RustDX 之后插入 Baostock) — Task 7
- `DataFetcherManager::new()` 注入 `BaostockProvider` — Task 7

## 失败测试说明

`cargo test --lib` 时 `database::tests::test_backfill_st_type_prefix_anchored` 偶发失败. 单跑 OK, 多跑 OK — 经典 SQLite 并发/顺序依赖问题, **与本 Task 改动无关** (本次未触碰 `database/` 任何文件). 不在 Task 5 修复范围.

## 关键文件路径

- src: `/Users/zhangzhen/Desktop/Quant/stock_analysis/src/data_provider/baostock_provider.rs`
- test: `/Users/zhangzhen/Desktop/Quant/stock_analysis/tests/baostock_provider_test.rs`
- mod 注册: `/Users/zhangzhen/Desktop/Quant/stock_analysis/src/data_provider/mod.rs:6`
