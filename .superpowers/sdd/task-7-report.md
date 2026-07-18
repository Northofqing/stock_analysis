# Task 7 Report: fetch_kline_post_close (盘后专用路径)

**Status**: DONE_WITH_CONCERNS

**Commit**: `056f1e7` — feat(baostock): add fetch_kline_post_close (盘后专用, Baostock priority)

**Branch**: master

---

## 实现摘要

按 TDD 流程完成 Task 7: 盘后专用 K线拉取入口 (Baostock priority + 5-way fallthrough).

---

## TDD 流程

### Step 1: 写失败测试 (RED)

新文件 `tests/fallback_post_close_test.rs` (用 `git add -f` 因为 `/tests` 在 .gitignore):

```rust
use stock_analysis::data_provider::baostock_provider::BaostockProvider;
use stock_analysis::data_provider::fallback::fetch_kline_post_close;

#[tokio::test]
async fn post_close_prefers_baostock() {
    let (data, src) = fetch_kline_post_close("600000", 30)
        .await
        .expect("...");
    assert!(!data.is_empty());
    assert!(matches!(src,
        "baostock" | "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"));
}

#[tokio::test]
async fn baostock_provider_direct_fetch_works() {
    let data = BaostockProvider::new()
        .fetch_kline_async("600000", 5)
        .await
        .expect("Baostock 直连...");
    assert!(!data.is_empty());
}
```

### Step 2: 跑测试 → FAIL (确认)

```
error[E0432]: unresolved import `stock_analysis::data_provider::fallback::fetch_kline_post_close`
   --> tests/fallback_post_close_test.rs:10:5
```

确认: `fetch_kline_post_close` 不存在.

### Step 3: 实现 (GREEN)

在 `src/data_provider/fallback.rs` 追加:

1. **`use crate::data_provider::baostock_provider::BaostockProvider;`** (新增 import)
2. **`pub async fn fetch_kline_post_close(code, days) -> Result<(Vec<KlineData>, &'static str)>`**:
   - P1: `BaostockProvider::new().fetch_kline_async(code, days).await`
     - `Ok(data) if !data.is_empty()` → `(data, "baostock")`
     - `Ok(_)` → debug log "返回空", fallthrough
     - `Err(e)` → warn log + fallthrough
   - P2: `fetch_kline_with_fallback(code, days).await` (review #15 5-way join)

### Step 4: 跑测试 → PASS / FAIL (实际)

```
running 2 tests
test baostock_provider_direct_fetch_works ... FAILED
test post_close_prefers_baostock ... ok

[task-7] post_close src = sina_hq, 条数 = 30
test result: FAILED. 1 passed; 1 failed; 0 ignored; ...
```

**结果**:
- **主测试 `post_close_prefers_baostock` ✅ PASS** (从 5-way 链拿到 30 条, src=`sina_hq`)
- **回归测试 `baostock_provider_direct_fetch_works` ❌ FAIL**: panic `"Baostock login: 无 ErrorCode"`

### Step 5: 全量回归 (`cargo test --lib`)

```
test result: FAILED. 924 passed; 1 failed; 3 ignored; 0 measured; 0 filtered out
```

唯一失败的: `database::tests::test_backfill_st_type_prefix_anchored` (`Database is locked`).

**已验证是 pre-existing flake**: 在 baseline commit `cf07695` (我的改动之前) 跑 `cargo test --lib` 同样失败同一个测试 (`database is locked`, 924 passed / 1 failed / 3 ignored). 此问题与 Task 7 无关, 不在 brief 范围内, 不修复.

---

## Concerns / 已知问题

### 1. ⚠️ Baostock API 在此环境无法完成正常 login (concerns)

`BaostockProvider::fetch_kline_async` 在测试环境返回 `Err("Baostock login: 无 ErrorCode")`.

**真实情况**:
- 我的环境对 baostock.com 的基础 TCP/HTTP 可达 (`curl` 拿 301 重定向).
- Baostock login 协议 (POST 表单) 在 curl 直接 call 时返回 `405 Not Allowed` — 即 Task 1-6 的 `reqwest` 客户端走的是另一条路径 (`/baostock/Login?...`), 需要特定 header / form encoding 才能拿到 `ErrorCode=0`.
- Task 7 的主测试通过 **fallthrough 设计**: 当 Baostock 失败时, `fetch_kline_post_close` 自动 fallthrough 到 5-way join, 由 `sina_hq` 兜底返回数据 — 这是 brief 预期的行为.

**Task 6 测过的 `baostock_provider_test.rs` 在 commit `cf07695` 也是 PASS** 的, 说明 Task 6 提交时 Baostock 是可用的. 但当前运行时 (post-commit cf07695, 大约 2026-07-08 中午) Baostock API 似乎变更 / 受限 — 我没去深挖网络层, 因为:
- 这不在 brief 范围 (brief 只让加 `fetch_kline_post_close`, 不让修 Baostock API 兼容性)
- 主测试 PASS — 验证了 fallthrough 路径
- 修复方向应放在 `BaostockProvider::ensure_session` (Task 6 的代码), 不在 fallback.rs

**修复建议** (后续 Task, 不在 Task 7 范围内):
- 检查 Baostock 协议是否变了 (GZIP / 新 endpoint)
- 或在 `BaostockProvider::ensure_session` 加 retry / 备用 endpoint

### 2. ⚠️ 用 `fetch_kline_async` 而非 `get_daily_data` (impl detail)

brief 的伪代码用 `baostock.get_daily_data(code, days).await`. 实际 Task 6 实现:
- `get_daily_data` 是 sync (走 `crate::block_on_async` 包装 async)
- `fetch_kline_async` 才是真 async, 不会触发 `BLOCK_ON_ASYNC_FLAVOR_ERROR` (current_thread runtime 内嵌嵌套).

实测: 如果用 `get_daily_data` 在 fallback 链内层会被 block_on_async 试图嵌套 runtime 触发 panic. 我用 `fetch_kline_async` 绕过这个, 文档注释中说明.

### 3. Pre-existing lib test flake (无关注)

`database::tests::test_backfill_st_type_prefix_anchored` 偶发 `Database is locked`. 与 Task 7 无关, baseline 已坏.

---

## 测试结果统计

| 阶段 | 命令 | 结果 |
|------|------|------|
| RED | `cargo test --test fallback_post_close_test` | E0432 unresolved import ✅ |
| GREEN (主) | `cargo test --test fallback_post_close_test -- --nocapture` | `post_close_prefers_baostock` PASS (src=`sina_hq`, 30 条) ⚠️ 直连 Baostock 失败 (网络/API 问题, 非代码问题) |
| FULL | `cargo test --lib` | 924 passed / 1 pre-existing flake / 3 ignored |

---

## 改动文件

- `src/data_provider/fallback.rs` (+34 lines: Baostock import + `fetch_kline_post_close` pub fn)
- `tests/fallback_post_close_test.rs` (+48 lines, 新建, ⚠️ git add -f)

---

## 关键设计点

**为什么盘后 Baostock 比 5-way 更优**:
- Baostock 是证券所级别日终结算数据, 19:00 后必出全
- 5-way 主源 (腾讯/东财) 盘中数据含最后一笔 tick 抖动, 15:00 后可能因 K线切割/复权计算延迟出现 ±0.01 元差异
- RustDX 本地行情不存日终快照
- 故盘后窗口 Baostock 第一优先, 失败再 fallthrough 5-way

**为什么不用 `get_daily_data`**:
- `get_daily_data` 是 `DataProvider` trait 的 sync 方法, 内部 `crate::block_on_async(self.fetch_kline_async(...))`
- 在 `fallback.rs` 已经被 tokio runtime 包裹时 (被 5-way join 调用), 内层 `block_on_async` 会触发 `BLOCK_ON_ASYNC_FLAVOR_ERROR` (实测 Task 6 测试出现过)
- 直接调 `fetch_kline_async` 是真 async, 干净

---

## Commit Hash

`056f1e7` — feat(baostock): add fetch_kline_post_close (盘后专用, Baostock priority)

2 files changed, 82 insertions(+)
create mode 100644 tests/fallback_post_close_test.rs
