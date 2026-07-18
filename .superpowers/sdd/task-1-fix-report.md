# Task 1 Fix Report — 修 2 review findings

**Status:** DONE
**Branch:** master (未 commit, 等 reviewer 确认)
**Source task:** commit `24faf29` PR-1 修 R-1 verify_predictions 假实现

## Findings 修法概要

### Finding #1: AGENTS.md §2.8 引用了不存在的脚本
**修法**: 新建 `tools/compliance/check.sh` 主入口 + `tools/compliance/lib/check_fake_impl.sh` 最小版, 让 §2.8 末尾引用的脚本非空。

### Finding #2: verify 逻辑在 prediction.rs 和 backfill_predictions.rs 之间逐字复制
**修法**: 在 `src/monitor/prediction.rs` 抽出 `pub async fn verify_one` + `pub struct VerifyOutcome`, 两处调用方都改为调用它。

---

## 1. Files changed

| File | Lines (after) | Change |
|------|---------------|--------|
| `tools/compliance/check.sh` | 50 (new) | 主入口, 当前只挂 check_fake_impl.sh; PR-2/3/4 会扩展 |
| `tools/compliance/lib/check_fake_impl.sh` | 53 (new) | 拦截 `update_.*result.*0\.0.*false` + 校验 e2e_prediction_verify.rs 存在 |
| `src/monitor/prediction.rs` | 172 | 抽出 `verify_one` + `VerifyOutcome`; `verify_predictions` 改调用它; 加 `.await` |
| `src/bin/backfill_predictions.rs` | 75 | 删本地 `read_close` 和 diesel 导入; 改调 `prediction::verify_one` |

两个 shell 脚本都 `chmod +x` 已确认 (`-rwxr-xr-x`)。

---

## 2. verify_one 签名

```rust
/// 单条 prediction 的 verify 结果。
/// actual_change 单位为 %; hit 为方向匹配 + |actual| > 0.5%。
#[derive(Debug, Clone, Copy)]
pub struct VerifyOutcome {
    pub actual_change: f64,
    pub hit: bool,
}

/// 共享 verify 逻辑 — 读 (pred_date, target_date) 两个本地 close, 算 actual_change, 判定 hit。
///
/// 被 `verify_predictions` (生产盘后回填) 和 `backfill_predictions` (历史回填) 复用, 避免逐字复制。
/// 返回 `None` 表示无法判定 (缺 close / prev_close <= 0), 调用方自行决定 warn-and-continue 还是 `?` 报错。
///
/// 注意: 此函数**不**写回数据库, 调用方负责 `update_prediction_result` — 这样 verify_predictions
/// 的"warn + 继续"语义和 backfill_predictions 的"`?` 失败"语义都能保留。
pub async fn verify_one(
    db: &DatabaseManager,
    code: &str,
    pred_date: &str,
    target_date: &str,
    direction: &str,
) -> Option<VerifyOutcome>
```

**Visibility 选择**: 用 `pub` (非 `pub(crate)`)。原因: `bin/backfill_predictions.rs` 是单独的 crate path (`src/bin/...`), 不在 lib 的 crate 内, 必须是 `pub` 才能从 `stock_analysis::monitor::prediction::verify_one` 访问到。e2e 测试 `tests/e2e_prediction_verify.rs` 通过同样路径也能用 (本次没改 e2e, 但保持一致)。

**为何是 `async`**: 函数内部用的是 `DatabaseManager::get()` + diesel sync API, 本身不需 async。但 `verify_predictions` 是 `pub async fn` (Task 1 主修复时定的签名, 适配 main 循环 .await), 让 `verify_one` 也 async 可以让调用方零成本 await, 避免在 main 循环里加 `block_on`。这个选择保持了 Task 1 阶段定的 async 边界。

**为何不写 DB**: 两种调用方语义不同:
- `verify_predictions`: warn + continue (生产盘后, 单条失败不应阻塞整体)
- `backfill_predictions`: `?` propagate (历史回填, bin 直接报错退出)
让 verify_one 只算不算写, 调用方各自决定错误处理, 双方语义都保留。

---

## 3. check_fake_impl.sh 内容

```bash
#!/usr/bin/env bash
#
# check_fake_impl.sh — AGENTS.md §2.8 假实现禁令门禁
#
# 目的: 拦截"写日志不操作数据"的 verify/save/notify/sync/update_result 类假实现。
# 反模式 (R-1 修复新增):
#   match db.update_prediction_result(&today, None, 0.0, false) { ... }
#   db.update_prediction_result(..., 0.0, false)?;
#
# 退出码:
#   0 = pass (没有发现假实现)
#   1 = fail (发现假实现模式 / 必须存在的 e2e 测试缺失)
#
# 配套:
#   AGENTS.md §2.8 假实现禁令 — 描述反模式 + 正例
#   tests/e2e_prediction_verify.rs — 真实 verify 行为的 e2e 测试 (必须存在)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_DIR="$REPO_ROOT/src"
E2E_TEST="$REPO_ROOT/tests/e2e_prediction_verify.rs"

EXIT_CODE=0

# 1. 拦截假实现模式: update_.*result.*0\.0.*false
#    在 src/ 下任何 .rs 文件命中即 fail。
FAKE_PATTERN="update_.*result.*0\.0.*false"
HITS=$(grep -RInE "$FAKE_PATTERN" "$SRC_DIR" --include="*.rs" 2>/dev/null || true)

if [ -n "$HITS" ]; then
    echo "[check_fake_impl] FAIL: 发现假实现模式 '$FAKE_PATTERN':"
    echo "$HITS" | sed 's/^/  /'
    EXIT_CODE=1
else
    echo "[check_fake_impl] OK: src/ 下未发现假实现模式"
fi

# 2. 必须存在 e2e 测试 (R-1 修复新增 — 验证 verify 真实行为)
if [ ! -f "$E2E_TEST" ]; then
    echo "[check_fake_impl] FAIL: 必须存在 e2e 测试 $E2E_TEST (AGENTS §2.8 验证)"
    EXIT_CODE=1
else
    echo "[check_fake_impl] OK: e2e 测试存在 ($E2E_TEST)"
fi

if [ $EXIT_CODE -eq 0 ]; then
    echo "[check_fake_impl] PASS"
else
    echo "[check_fake_impl] FAIL — 见上方"
fi

exit $EXIT_CODE
```

**设计取舍**:
- 最小版只做 §2.8 引用的 2 件事 (模式 grep + e2e 存在性); PR-3 的 §2.9 脚本会扩展更多模式 (含 `verify` / `save` / `notify` / `sync` 等假实现变体)。
- 模式与 AGENTS §2.8 的反模式示例**逐字一致** (`update_.*result.*0\.0.*false`)。
- `set -uo pipefail` 但故意不用 `-e` — 两个检查独立 fail-fast, 一次跑完两个, 报告更全。

---

## 4. 跑的测试 + 结果

| Command | Result |
|---------|--------|
| `cargo build --bin backfill_predictions` | OK (29s) |
| `cargo build --bin monitor` | OK (0.59s, 增量) |
| `cargo test --test e2e_prediction_verify` | **2/2 pass** (test_verify_predictions_writes_real_actual_change, test_verify_predictions_miss_for_bearish_prediction_on_up_day) |
| `cargo test --lib` | **459/459 pass** (2.44s) |
| `bash tools/compliance/check.sh` | **PASS** (check_fake_impl.sh OK) |

(doctest 失败 3 个在 `src/strategy/mod.rs` 与本次任务无关, 已在 task-1-report.md 记录为 pre-existing, CLAUDE.md `cargo test` 只跑 `--lib` 不受影响)

---

## 5. Deviations / Concerns

### 5.1 `verify_one` 返回 `Option` 而非 `Result<Option, Error>` (plan 原文是 `Result`)
Plan 描述的签名是 `Result<VerifyOutcome, Box<dyn std::error::Error>>`。我改成了 `Option<VerifyOutcome>`。

**理由**:
- `read_stock_daily_close` 内部用 `.ok().and_then(|r| r.close)`, 已经把 diesel error 吃掉返回 `Option` (在 prediction.rs 现有实现里就是这样, 是 `read_stock_daily_close` 的设计, 不是我引入的)。
- 共享函数要复用 `read_stock_daily_close` → 错误粒度只能是 `Option`。
- 两种调用方都不需要 diesel 错误粒度: `verify_predictions` 是 warn-and-skip, `backfill_predictions` 的 DB 错误由 `update_prediction_result` 那行的 `?` 捕获, 那里仍是 `Result<_, Error>`。
- 这是用最小的类型忠实表达"无法判定"的语义, 不是过早抽象。

### 5.2 `backfill_predictions` 删了 `read_close` 和 diesel 导入
原 bin 有自己的 `read_close(db, code, date: NaiveDate) -> Result<f64>`, 我换成共享 `verify_one` 后, 整个 helper 不再需要 (verify_one 内部用 `&str` 调 `read_stock_daily_close`)。bin 内 `use diesel::RunQueryDsl` 也跟着删了 (不再直接用 sql_query)。这是 DRY 的副产物, 不是额外重构。

### 5.3 `check_fake_impl.sh` 的模式只覆盖 R-1 一个反模式
Plan 提到"PR-3 的 §2.9 脚本会扩展它"。我按 plan 边界, 最小版只做 §2.8 引用的 2 件事。**不**去提前覆盖 §2.9 的 `verify`/`save`/`notify`/`sync` 等模式 (YAGNI, 留给 PR-3)。

### 5.4 `verify_one` 是 `async` 但内部没有 `.await`
Plan 描述的签名是 `pub async fn verify_one(... )`, 内部实现都是同步调用。`async` 是为了匹配 `verify_predictions` 的 async 签名 (Task 1 主修复时定的, 适配 main 循环 .await), 让调用方零成本 await。性能上 Rust 的 async zero-cost future 不会有运行时开销。

### 5.5 AGENTS.md §2.5 隔离保持
verify_one 复用 `read_stock_daily_close` (与生产相同的本地库读), 但调用方仍然只对 `TEST_CODE` 标的做 verify (e2e 测试用 `TST001`/`TST002`)。生产盘后 verify 只 verify 真实持仓的代码, 与测试 e2e 物理隔离。

---

## 6. Self-check

- [x] DRY: verify 逻辑只 1 份, 在 `verify_one` 里
- [x] YAGNI: 没顺手重构其它 (没动 `read_stock_daily_close`, 没动 `update_prediction_result`, 没动 e2e 测试)
- [x] 无新依赖
- [x] shell 脚本已 `chmod +x`
- [x] e2e 测试通过 (`cargo test --test e2e_prediction_verify` 2/2)
- [x] lib 测试通过 (`cargo test --lib` 459/459)
- [x] AGENTS.md §2.8 末尾引用 (`tools/compliance/lib/check_fake_impl.sh`) 现在指向真实存在的脚本
- [x] 没 commit
