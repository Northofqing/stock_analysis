wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-8-brief.md: 96 lines
/main.rs`
- Modify: `docs/business_rules.md` (加 BR-014 + BR-015)
- Create: `docs/sina_baostock_integration.md`

- [ ] **Step 1: Add startup log**

```rust
// src/bin/monitor/main.rs (在 main 开头, after config 加载)
log::info!(
    "[启动] K线 fallback chain: sina_hq → tencent_qfq → eastmoney_qfq → rustdx_none (4-way join, review #15)"
);
log::info!("[启动] 盘后路径: baostock → 4-way join (post_close)");
```

- [ ] **Step 2: Add BR-014 + BR-015 to business_rules.md**

```markdown
| BR-014 | ✅ registered | Sina (hq.sinajs.cn) 接入 fallback priority 1 — GBK 编码 + 公开 HTTP + JSONP 解析, IP 独立于腾讯/东财 | `src/data_provider/sina_provider.rs`, `src/data_provider/stock_code_map.rs` |
| BR-015 | ✅ registered | Baostock (baostock.com) 盘后专用日终数据, 无限调用, WebSocket-like session + 复权 (adjustflag=2) | `src/data_provider/baostock_provider.rs`, `src/data_provider/fallback.rs` |
```

- [ ] **Step 3: Write user-facing docs**

```markdown
<!-- docs/sina_baostock_integration.md -->
# Sina + Baostock 数据源集成

## 背景
review #15 后 fallback 链有 4 源 (腾讯/东财/RustDX), 全是公开 HTTP/TCP, 风险同质.
本次加 2 个新源:
- **Sina** (priority 1): 公开 HTTP, 域名独立, 0 费用
- **Baostock** (盘后专用): 证券所级别日终, 无限流

## Fallback 链
[流程图: Sina (1) → 腾讯 (2) → 东财 (3) → RustDX (4)]

## 盘后路径
[流程图: Baostock → 4-way join]

## 配置
- 无新增 env var (自动启用)
- 可选 `BAOSTOCK_BASE_URL` (默认 baostock.com)

## 故障排查
- Sina 503: 偶发, fallthrough 自动处理
- Baostock login 失败: 重试 1 次, fallthrough
```

- [ ] **Step 4: Update README**

```markdown
<!-- README.md 在 ## Architecture 段 -->
### 数据源
- K线 fallback: Sina → 腾讯 → 东财 → RustDX (4-way join)
- 盘后日终: Baostock (独立路径)
- 详见 `docs/sina_baostock_integration.md`
```

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 912+ passed.

- [ ] **Step 6: Commit**

```bash
git add src/bin/monitor/main.rs docs/business_rules.md docs/sina_baostock_integration.md README.md
git commit -m "docs(data): add Sina+Baostock integration docs, BR-014/015, startup log"
```

---

## Final verification

```bash
cargo build --release
cargo test --lib
cargo clippy -- -D warnings
```

All must pass. Then push to master.

---

# Phase 2: Sina 新闻集成 (Tasks 9-12)

> 复用 Phase 1 的 Sina 客户端 (同 base URL, 同 GBK 处理, 同 stock_code_map).

---

