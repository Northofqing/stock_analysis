# 根因 E: 多 Agent 研判绕开 Freshness 门禁

> **优先级**: P0（Critical）
> **违反**: AGENTS.md §2.1（生产路径不能降级到假数据）+ §2.4（过期数据视同失败）+ §2.3（坏数据校验）
> **影响面**: 7 条持仓深度研判推送 + 任何调用 `run_multi_agent_analysis` 的下游
> **位置**:
>   - `src/deep_analyzer.rs:186-195` (`run_multi_agent_analysis` 入口)
>   - `src/deep_analyzer.rs:190` (K 线数据获取)
>   - `src/data_provider/service.rs:67-130` (`get_kline` 缓存层)
>   - `src/data_provider/mod.rs:59-90` (`KlineData` 数据结构)

---

## 一、复现路径

### 1.1 触发条件

```bash
# 盘后跑 review（行情已 4 小时过期）
cargo run --bin monitor -- --review
```

### 1.2 实际日志（Claude 16:15 run 摘录）

```
[16:15:48 WARN] [DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=quote source=eastmoney code=600703 action=reject reason=数据过期
[16:15:48 WARN] [DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=quote source=eastmoney code=603948 action=reject reason=数据过期
... (35+ 条持仓股全部 reject)
[16:16:10 INFO] [复盘] ▶ 多 Agent 研判 600703 三安光电
[16:16:10 INFO] [MultiAgent] 开始抓取数据：600703
[16:16:11 INFO] [MultiAgent] 数据抓取完成 — 财务=true 研报=true 新闻=true 板块=true 筹码=true 资金=true
[16:16:30 INFO] [复盘] 持仓深度研判 三安光电(600703):
... (1500 字研判)
```

**35+ 持仓股全部 DQ_FRESHNESS reject，但研判照样出**。**这就是 §2.4 重大违反**。

### 1.3 根因链

```
review_only (main.rs:653)
  └─> run_review_deep_analysis (main.rs:830)
      └─> run_multi_agent_analysis(code) (deep_analyzer.rs:186)
          ├─> service::service().get_kline(code, 250) (deep_analyzer.rs:190)
          │   └─> OnceCell 缓存（service.rs:67）  ← 绕过 freshness
          │       └─> HttpProvider::fetch_kline_data_internal (service.rs:84)
          │           └─> 网络成功 → 返回 4 小时前的行情
          │
          └─> 6 tool 并行 (deep_analyzer.rs:194-218)
              └─> 用"过期"行情生成 1500 字研判 → push_wechat
```

**关键点**：`service.rs:67` 内部**没有任何 freshness 校验**，连 K 线最后一次更新时间都没有记录。

---

## 二、代码定位证据

### 2.1 `run_multi_agent_analysis` 入口（不查 freshness）

```rust
// src/deep_analyzer.rs:186-195
pub async fn run_multi_agent_analysis(code: &str) -> Result<String> {
    log::info!("[MultiAgent] 开始抓取数据：{}", code);

    // 1. K 线（service 缓存）  ← 没有 freshness 校验
    let kline = service::service().get_kline(code, 250).await?;
    if kline.is_empty() {
        anyhow::bail!("K 线数据为空，无法进行多角色分析");
    }
    // ...
}
```

**对比 `market_data.rs:57`（有 freshness）**：
```rust
// src/bin/monitor/market_data.rs:57
if !validate_quote_freshness(update_time, "eastmoney", &code) {
    return None;  // ← freshness reject
}
```

**两套数据获取路径，freshness 行为不一致**——这是**横切关注点漏配**的典型表现。

### 2.2 `service::get_kline` 无时间戳字段

```rust
// src/data_provider/service.rs:67-130
pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
    let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
    cell.get_or_try_init(|| async move {
        // 三源回落：东方财富 → 腾讯 → RustDX
        // 任何成功结果都会缓存到进程退出为止
        ...
    }).await.cloned()
}
```

- `OnceCell` 缓存键是 `(code, days)`，**不包含时间**
- 一次进程内只拉一次，**缓存永驻**
- 没有任何 `last_update` 字段

### 2.3 `KlineData` 数据结构缺时间戳

```rust
// src/data_provider/mod.rs:55-90
pub struct KlineData {
    pub date: NaiveDate,  // ← 是 K 线日期，不是 fetch_time
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub pct_chg: f64,
    pub intraday_price: Option<f64>,
    pub settled: bool,
    // 缺：pub fetched_at: DateTime<Local>,
    // 缺：pub source: String,
}
```

**没有 `fetched_at` 字段**——意味着即使想加 freshness gate，也**没有数据可校验**。这是**领域模型缺字段**。

### 2.4 `run_review_deep_analysis` 也不查 freshness

```rust
// src/bin/monitor/main.rs:830-880
async fn run_review_deep_analysis() {
    let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
    // ...
    let results: Vec<(String, String, Option<String>)> = stream::iter(codes)
        .map(|(code, name)| async move {
            let deep = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                stock_analysis::deep_analyzer::run_multi_agent_analysis(&code),
            ).await;
            // ...
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;
    // 直接 push_wechat，没有 freshness gate
}
```

**只有 300s 超时**，**没有 freshness 校验**。

---

## 三、影响面分析

### 3.1 直接违反

- **§2.1**: "生产路径禁止 mock 数据" — 用过期行情当"当前行情"喂 LLM 是事实上的降级到假数据
- **§2.4**: "超过过期阈值的数据视同失败，显式报错，不沿用" — **沿用了**
- **§2.3**: "数据进入计算前做校验" — **跳过了**

### 3.2 受影响路径

| 路径 | 文件:行 | 是否 fresh |
|------|---------|-----------|
| 放量分析·持仓 | `main.rs:690` | ❌（走 `fetcher.get_daily_data`，无 freshness） |
| 持仓深度研判 | `main.rs:830` → `deep_analyzer.rs:190` | ❌ |
| 优选候选 | `main.rs:790` → `opportunity::run_post_close_candidates` | ❌ |
| 因子 IC 分析 | `main.rs:649` | ❌ |
| 涨跌幅 TopN | `market_data.rs:57` | ✅ |
| 北向资金 | `async_overview.rs:34-42` | ❌（无 freshness 字段） |

**结论**：只有"涨跌幅 TopN"这一条路径有 freshness 校验。**其它所有研判路径都裸奔**。

### 3.3 业务后果

- **盘后 review 推送 1500 字/票 × 7 票 = 1.1 万字噪声**（用户观感 2）
- LLM 拿到 4 小时前的行情，**因子归因是错的**（"放量"是上午的，但研判说"今日尾盘"）
- **多 Agent 6 工具的"6 个独立观点"幻觉**（Claude 根因 C）——本质是同一份过期数据 × 6 个 prompt

---

## 四、修复方案（按 ROI 排序）

### 方案 α: 最小侵入 — 在 deep_analyzer 加 freshness gate（**推荐 P0**）

**位置**: `src/deep_analyzer.rs:190` 之后

**实现**:
```rust
let kline = service::service().get_kline(code, 250).await?;
if kline.is_empty() {
    anyhow::bail!("K 线数据为空，无法进行多角色分析");
}

// 修复 P0-E: freshness gate（AGENTS §2.4）
let now = chrono::Local::now();
let kline_age_hours = kline.last()
    .map(|k| (now - chrono::NaiveDateTime::new(k.date, chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap())).num_hours())
    .unwrap_or(999);
let cfg = stock_analysis::config::get_monitor_config();
let is_post_close = stock_analysis::market_session::is_post_close(now);
let max_age_secs = if is_post_close { cfg.dq_post_close_quote_stale_sec } else { cfg.dq_quote_stale_sec };
if kline_age_hours * 3600 > max_age_secs as i64 {
    anyhow::bail!("[MultiAgent] {} K线过期 {}h > 阈值 {}s, 拒绝研判", code, kline_age_hours, max_age_secs);
}
```

**工作量**: 2h
**风险**: 需要新增 `market_session::is_post_close()` 和 `dq_post_close_quote_stale_sec` 配置

### 方案 β: 中等侵入 — `KlineData` 加 `fetched_at` 字段（**P1 推荐**）

**位置**: `src/data_provider/mod.rs:55-90`

**实现**:
```rust
pub struct KlineData {
    pub date: NaiveDate,
    // ... 现有字段
    pub fetched_at: DateTime<Local>,  // ← 新增
    pub source: String,  // ← 新增: "eastmoney" | "gtimg" | "rustdx"
}
```

- 三个 provider 填充 `fetched_at = Local::now()`
- `service::get_kline` 缓存时同时缓存 `fetched_at`
- 业务层用 `KlineData::fetched_at` + `validate_quote_freshness` 统一校验

**工作量**: 1d（影响面广，需改 3 个 provider + service + 5+ 调用方）
**收益**: freshness gate 一劳永逸，不再漏配

### 方案 γ: 彻底重构 — freshness 拦截器模式（**P2**）

引入 trait：
```rust
#[async_trait]
trait FreshnessGuard {
    async fn fetch_with_freshness(&self, code: &str) -> Result<DataWithTimestamp>;
}
```

所有数据获取都过这个 trait。**工作量 1w**，建议在方案 β 落地稳定后再考虑。

### 推荐路径

**先做方案 α**（2h P0），**同步方案 β**（1d P1），**方案 γ** 留给后续优化。

---

## 五、合并 Gate Checklist

> 任何修复 E 的 PR 合并前**必须**逐项勾选。

### 5.1 修复完整性

- [ ] `src/deep_analyzer.rs:190` 后加 freshness gate
- [ ] 新增 `src/market_session.rs::is_post_close()`
- [ ] `config/monitor.toml` 加 `dq_post_close_quote_stale_sec = 86400`
- [ ] 方案 β: `KlineData` 加 `fetched_at` + `source` 字段（可选，作为 P1）
- [ ] 方案 β: 3 个 provider (`HttpProvider` / `GtimgProvider` / `RustdxProvider`) 填充 `fetched_at`

### 5.2 业务规则登记（§2.10）

- [ ] `docs/业务规则清单-registry.md` 新增 **BR-010: 多 Agent 研判必须 freshness 校验**（占位即可，详情见 §2.4）

### 5.3 单元测试

- [ ] 测试 1: `mock_kline_yesterday()` → 盘后跑 multi_agent → 期望 `Err("K线过期")`
- [ ] 测试 2: `mock_kline_today_realtime()` → 盘内跑 multi_agent → 期望 `Ok(...)`
- [ ] 测试 3: `mock_kline_4h_ago()` → 盘后跑 multi_agent → 期望 `Err("K线过期")`
- [ ] 测试 4: `mock_kline_fresh()` + 关闭 `is_post_close` 路径 → 期望 `Ok(...)`

### 5.4 集成测试

- [ ] CI 加场景：`--review` 跑通时，7 条持仓研判的 K 线 `fetched_at` 必须在 `dq_post_close_quote_stale_sec` 内
- [ ] 沙箱跑：`bash tools/compliance/lib/check_data_freshness.sh` 必须通过

### 5.5 旧模块接入检查表

> 新能力上线后，**必须对照旧模块**，逐个回答

- [ ] 列出所有调用 `run_multi_agent_analysis` 的位置：`rg "run_multi_agent_analysis" src/`
- [ ] 对每个调用点：是否走 freshness gate？
  - 已接入 → 记录 PR
  - 未接入 → 记录未接入理由
- [ ] 列出所有调用 `service::service().get_kline` 的位置：`rg "service.*get_kline" src/`
- [ ] 对每个调用点：是否走 freshness gate？

### 5.6 Review 检查（§2 红线）

- [ ] review 第 5 步：grep `src/deep_analyzer.rs` + `src/data_provider/service.rs` + `src/bin/monitor/main.rs` 确认无其他 `get_kline` / `run_multi_agent_analysis` 绕开 freshness
- [ ] review 第 5 步：检查 `docs/业务规则清单-registry.md` BR-010 登记完整
- [ ] review 第 5 步：检查单元测试 + 集成测试覆盖盘内/盘后/边界 3 个场景

### 5.7 数据红线（§2）

- [ ] §2.1: 不静默降级到假数据 ✅（方案 α/β 都显式 bail）
- [ ] §2.4: 过期数据视同失败 ✅
- [ ] §2.3: 坏数据校验 ✅
- [ ] §2.10: 业务规则登记 ✅
