# Task 8 Report — 启动日志 + BR-014/015 + 用户文档 + README

## Status
✅ All 6 steps complete. Commit `8cb92b1` on master.

## Steps Executed

### Step 1: Startup log in `src/bin/monitor/main.rs`
Inserted 2 lines after `[broker] 启动完成` (line 970, after config load + broker detect):
```rust
log::info!(
    "[启动] K线 fallback chain (盘中): sina_hq (P1) → tencent_qfq (P2) → eastmoney_qfq (P3) → rustdx_none (P4) | review #15 + #16"
);
log::info!("[启动] 盘后路径: baostock (P1) → 4-way join (P2, post_close)");
```
- Placement: right after `let broker_src = ...; log::info!("[broker] ...")` (around line 968), config load is already done (line 913 `load_all()`), broker detect complete.
- Concise: 2 lines, no per-source enum dump (cross-reference `[fallback]` in-process log if deeper detail needed).

### Step 2: BR-014 + BR-015 in `docs/business_rules.md`
Added 2 rows to the main table after BR-012:
```markdown
| BR-014 | ✅ registered | Sina (hq.sinajs.cn) 接入 fallback priority 1 — GBK 编码 + 公开 HTTP + JSONP 解析, IP 独立于腾讯/东财 | `src/data_provider/sina_provider.rs`, `src/data_provider/stock_code_map.rs` |
| BR-015 | ✅ registered | Baostock (baostock.com) 盘后专用日终数据, 无限调用, WebSocket-like session + 复权 (adjustflag=2) | `src/data_provider/baostock_provider.rs`, `src/data_provider/fallback.rs` |
```
- Note: BR-013 is intentionally absent (gap in numbering, leave to later).
- Both BRs reflect the actual code (Phase 1 Tasks 1–7), do not introduce new capability — pure documentation.

### Step 3: `docs/sina_baostock_integration.md` (new, 113 lines)
User-facing integration doc covering:
- 背景 (4 源 → 4+1, Sina 加 priority 1, Baostock 盘后)
- K线 Fallback 链 (4-way 盘中竞速表 + 代码入口)
- 盘后路径 (Baostock → 4-way fallthrough)
- 配置 (无新增必需 env, 仅 BAOSTOCK_BASE_URL 可选)
- 故障排查表 (Sina 503 / Baostock login / 字段缺失 / 全失败)
- **已知限制 B-002** (明确引用 progress.md Bug Log)
- 数据流时序 (盘中 + 盘后 ASCII diagram)
- 参考代码 + 测试文件指针

### Step 4: README `## 架构 (DDD 限界上下文)` extension
Added 1 subsection `### 数据源 (Phase 1, review #15 + #16)` with 2-row priority table linking to `sina_baostock_integration.md` and `business_rules.md`.

### Step 5: `cargo build`
- ✅ Compiled in 28.94s (existing warnings only, no new warnings introduced).
- No network call — purely log + docs changes, build deterministic.

### Step 6: Commit
```
8cb92b1 docs(data): add Sina+Baostock integration docs, BR-014/015, startup log
4 files changed, 115 insertions(+)
```
Used `git add -f` for docs/* (`.gitignore` excludes `docs/` at root).

## Files Touched
- `src/bin/monitor/main.rs` — +6 lines (2 log + 4 comment)
- `docs/business_rules.md` — +2 rows in BR table
- `docs/sina_baostock_integration.md` — new, 113 lines
- `README.md` — +10 lines (### 数据源 subsection)

## Findings
- **No code logic touched.** Per brief: "本 Task 纯文档 + 启动日志".
- **Brief prompt vs brief content discrepancy**: User-facing prompt referenced "5-way" but the actual fallback chain has 4 sources (Sina/Tencent/Eastmoney/Rustdx). The in-function log in `src/data_provider/fallback.rs:88-92` already says "4-way 竞速链" — confirmed by reading the code (lines 97-103 `enum SourceResult { Sina, Tencent, Eastmoney, Rustdx }`). **Used "4-way" everywhere** to match actual code and to match `task-8-brief.md` body.
- **`docs/` in `.gitignore`** — `git add -f` required (line 7 of gitignore). Used it.
- **No new tests needed** — no logic changes, no dependency changes, no API surface changes. Build-only verification was sufficient per brief Step 5.
- **B-002 referenced** — user docs explicitly call out the Baostock login protocol anomaly (current environment, no `ErrorCode` line), pointing to `progress.md` Bug Log for full context.

## Next Task
Task 9 (Phase 2: Sina 新闻集成). Skeleton plans out:
- Reuse `SinaProvider` from Phase 1 (same base URL, GBK decode, `stock_code_map`)
- Add `SinaNewsProvider::fetch_flash_news()` for flash news source
- Integrate into `fetch_news_with_fallback` chain
- TDD sequence: failing test → impl → fallthrough integration → tests → commit

## Git Status
- Branch: `master`
- 7 commits ahead of `stock_analysis/master` (ready to push after Phase 1 final verification).
- Branch is clean (working tree).
