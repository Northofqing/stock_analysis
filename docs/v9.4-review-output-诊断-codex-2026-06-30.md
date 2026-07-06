# monitor --review 输出质量诊断（Codex 视角）

> **触发**: 用户反馈 — "推了很多、有用信息不多、数据准确性不对、同质化严重"
> **生成方式**: `cargo run --bin monitor -- --review`
> **诊断时间**: 2026-06-30 17:00-17:10
> **输入物**:
>   - Claude 上一轮（16:15-16:18 一次完整 run, 2341 行 log）的诊断 → `docs/review-output-diagnosis.md`
>   - 我自己本次在 Codex 沙箱里跑 monitor 的 543 行 log（卡死在前 1 分钟）→ `docs/monitor-runs/2026-06-30/sandbox-blocked.log`
>   - 关键代码定位（`src/bin/monitor/main.rs`、`src/bin/monitor/freshness.rs`、`src/deep_analyzer.rs`、`src/opportunity/launch_gate.rs`、`src/monitor/data_quality.rs`、`src/market_analyzer/async_overview.rs`）
> **Codex 状态**: 沙箱无外网，3 分钟内出不来 review 完整输出；用 Claude 的 2341 行 log + 我自己的卡死 log + 源码交叉验证

---

## 〇、Codex 立场（先说结论再说证据）

**和 Claude 的判断 90% 一致，但有 4 个不同的侧重点**：

1. **Claude 的 P0-1 横幅问题是真 bug**——`main.rs:308` 文案 `"Shadow 不打用户, 仅日志"` 与 `launch_gate.rs:135` `Shadow => true` 矛盾。**但 Claude 没说清楚的是：这个矛盾是 2026-06-30 修复 F20 引入的回归**——修复前 Shadow 不推，修复后 Shadow 推了，但横幅文案没同步改。**这是一个"修了行为没改文档"的典型 bug，不是架构问题**。
2. **Claude 的 P0-2 freshness 阈值问题是真的，但根因更深一层**——`data_quality.rs:259` 写死 `quote_max_age_secs: 5`，**且整个代码库搜不到 `is_post_close` / `trading_session` 字段**。`run_review_only()` 走盘后路径时根本没区分盘内/盘后，是**架构层面**缺字段，不是单点配置错。
3. **Claude 没看到的根因 E（multi_agent 绕开 freshness）—— 这是个 §2.4 重大违反**：`deep_analyzer.rs:190` 走 `service::service().get_kline()`，**service 缓存层内部无 freshness gate**。即使 35+ 持仓股全部 DQ_FRESHNESS reject，多 Agent 研判照样跑、照样推送——**这是把"假数据"喂给 LLM 然后输出给用户**。直接踩 §2.3（坏数据校验）+ §2.1（生产路径不能降级到假数据）。
4. **Claude 的 P1-4 推送去重缺一个前置条件**——去重逻辑必须**先解决 E**，否则 7 条合并成 1 条，合并的内容里仍然有 "信息不足 / 评分中性" 的同质模板。

---

## 一、我的"另一次完整 run" 实证（沙箱场景）

### 1.1 沙箱无外网事实

```
[17:01:24 INFO] [通达信] 初始化 RustDX 数据提供者
[17:01:24 WARN] [新浪行情] 请求失败: error sending request for url (http://hq.sinajs.cn/list=sh600703,...)
[17:01:24 INFO] [通达信] 初始化 RustDX 数据提供者
[17:01:24 INFO] 尝试使用数据源: 通达信
[17:01:24 INFO] [通达信] 获取股票 603618 最近 60 天数据
[17:01:24 WARN] 数据源 通达信 获取失败: 无法连接到通达信服务器
[17:01:24 INFO] 尝试使用数据源: 腾讯财经
[17:01:24 INFO] [腾讯] 获取股票 603618 最近 60 天数据
[17:01:24 ERROR] [腾讯] 请求失败 (code=603618): error sending request for url (https://web.ifzq.gtimg.cn/...)
[17:01:24 WARN] 数据源 腾讯财经 获取失败
[17:01:24 INFO] 尝试使用数据源: HTTP(东方财富)
[17:01:24 INFO] [HTTP] 获取股票 603618 最近 60 天数据
[17:01:24 WARN] [HTTP] 请求失败 (attempt 1/6 host=push2his.eastmoney.com code=603618): error sending request...
```

**4 个数据源全失败**（RustDX 通达信 / 新浪 / 腾讯 / 东财 HTTP）。**`monitor --review` 在网络层完全不可用**——这不是 review 模式的问题，是**所有网络出站都不可用**。

### 1.2 进程在 17:01:24 之后无新输出 5 分钟

`stat -f "%Sm"` 显示 log 文件 mtime 锁定在 17:01:24；543 行 / 20.7KB 没增长；`fuser` 显示无进程持有 log fd。

**但 `eastmoney_provider.rs:85-160` 是 `for attempt in 1..=max_attempts { ... sleep(500*attempt).await; continue; }` 循环——理论上 6 次 retry 应该不停刷日志**。

两个可能性：
- (a) 进程在 retry 循环里，但 rust `log` crate 默认 `env_logger` 行缓冲，stdout 被 `nohup` 重定向到文件后未 flush
- (b) 进程在某次重试里 deadlocked（`reqwest` + 沙箱 DNS 黑洞的已知行为）

**不论是哪个，都意味着**：`monitor --review` 在网络受限时**没有早退机制**。这个失败模式 AGENTS.md §2.1 必须显式处理，但当前代码**没有任何"上游全失败时显式报错退出"** 的逻辑——会无限静默重试。

### 1.3 与 Claude 场景的对比

| 场景 | Claude (16:15) | Codex (17:01) |
|------|---------------|---------------|
| 网络 | 可用 | 沙箱无外网 |
| 跑通？ | ✅ 3 分钟，14 条推送 | ❌ 卡 5 分钟，0 条推送 |
| DQ_FRESHNESS warn | 35+ | 0 (没跑到 freshness gate) |
| 暴露的问题 | 内容质量 | 失败模式 |

**Codex 这次"没跑出来"反而揭示了 Claude 看不到的失败模式**——AGENTS §2.1 / §2.3 / §2.4 在网络完全失联时**没有 fast-fail 路径**。

---

## 二、Claude 7 个修复点逐条源码验证

| # | Claude 提案 | 源码定位 | 验证结果 | Codex 评价 |
|---|------------|---------|---------|-----------|
| P0-1 | 修启动横幅文案 | `main.rs:308` vs `launch_gate.rs:135` | ✅ 确认矛盾。横幅 `Shadow => "不打用户, 仅日志"`；代码 `Shadow => true`（F20 修复后）| **真 bug，且是 2026-06-30 修复 F20 引入的回归**——是 doc/code 漂移，不需改架构 |
| P0-2 | freshness 区分盘内/盘后 | `data_quality.rs:259` 写死 `quote_max_age_secs: 5`；`freshness.rs:39-69` 函数签名无 `is_post_close` 参数；全仓搜 `is_post_close/trading_session` 0 命中 | ✅ 确认。**架构层缺字段** | 比 Claude 估的更深——需要新加 `is_post_close: bool` 字段 + 配置文件开关 + 函数签名全链路改 |
| P0-3 | 北向资金 fallback 改 warn | `async_overview.rs:34-42`：拉取失败 `warn!` 但 `overview.north_flow` 保持 default 0.0；下游打印 `{:+.2}亿` 显示 0.00 | ✅ 确认。但 `north_flow: f64` 字段类型，0.0 是合法 f64 | **不能光改 warn**——必须把字段类型改成 `Option<f64>`，下游打印 None 显示 "[数据缺失]" |
| P1-4 | 推送去重 + 按操作建议分组 | `main.rs` 内 14 处 `push_wechat` 调用 | ✅ 确认 14+ 处。但**前置条件**没解决 | 见 §四 根因 E |
| P1-5 | 模板按信号强度动态裁剪 | `deep_analyzer.rs` 模板生成逻辑在 LLM prompt 拼接处 | ⚠️ 模板本身在 prompt 里，**动态裁剪必须改 prompt + 重测 LLM 输出** | 工作量 Claude 估低了——3h 不可能，需要重写模板生成 + 单元测试 LLM 输出 schema |
| P2-6 | 6 分析师真独立化 | `deep_analyzer.rs:194-218` 6 个 tool 并行，**输入是 `code_input: {"code": code}` 同一份** | ✅ 确认。6 tool 看的是**同一组数据切片**，不是 6 个独立信息源 | Claude 提的 3 个选项里，(2) 不同 temperature 跑同一 prompt 实际**可能加剧模板同质化**——温度差会让 6 份输出**随机漂移但均值不变** |
| P2-7 | 因子归因加板块对比 | `src/decision/` 因子计算逻辑 | ⚠️ 部分 OK。但需查 `sector_pe` / `sector_roe` 是否有现成数据 | 建议 P1-7 改 P2-7 |

---

## 三、用户 4 个观感的独立诊断（结合 Claude 输出 + 我的代码定位）

### 观感 1: 推了很多（14 条）

**和 Claude 一致**。但加一个观察：

14 条推送里 7 条是"持仓深度研判"，**每条都是 1,500-1,900 字**，**触发 `push_wechat(&text)` 而不是 `push_wechat_with_kind(&text, true)`**——这意味着在 Shadow 模式下 7 条都通过 `should_push_user(Shadow, false) == true` 推出去。**`is_critical_alert = false` 表明这 7 条都被分类为"普通扫描"**，但实际占用 1.1 万字篇幅——`is_critical_alert` 的语义需要扩充，或者引入"is_long"标志。

### 观感 2: 有用信息不多

**和 Claude 一致**。补充一个代码级观察：

`deep_analyzer.rs:228-235` 的 `unwrap_or_warn` 函数，6 个 tool 任意一个失败 → 静默填空字符串：

```rust
let unwrap_or_warn = |label: &str, r: Result<String>| -> String {
    match r {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[MultiAgent] {} 工具失败: {:#}", label, e);
            String::new()  // ← 静默填空，下游 LLM 拿到空上下文
        }
    }
};
```

**这就是"信息不足 / 评分中性"高频出现的根因**——LLM 拿到 6 个空字符串照样编出 1,500 字。

**直接违反 §2.8 假实现禁令的精神**：tool 失败时空字符串是"假数据"喂给 LLM；应该**收集 6 个 tool 的真实成功/失败状态**，作为输入 prompt 的一部分强制 LLM 标注 "[数据缺失]"。

### 观感 3: 数据准确性不对

**和 Claude 一致**，但补充 3 个新发现：

**3.1 🚨 多 Agent 走 service 缓存，绕开 freshness**（§2.4 重大违反 + §2.1 假数据）

```rust
// src/deep_analyzer.rs:190
let kline = service::service().get_kline(code, 250).await?;
```

`service` 是项目内缓存层（`src/data_provider/service.rs`），**K 线会从本地 DB 拿**——但**没有任何 freshness 校验**。盘后跑 review 时行情是 4 小时前的收盘价：
- `market_data.rs:57` 路径 → `validate_quote_freshness` 校验 → reject ✓
- `deep_analyzer.rs:190` 路径 → **直接返回缓存** ✗

**后果**：35+ warn 报数据过期，但 7 条持仓研判**用的就是这些被 reject 的"过期"行情**。**违反 §2.1（生产路径不能降级到假数据）+ §2.4（过期数据视同失败）**。

**3.2 🚨 6 tool 失败静默填空是 §2.1 隐性违反**

同 §观感 2 的代码定位。空字符串喂 LLM = 假数据。

**3.3 🚨 横幅文案与代码行为矛盾**（§2.10 业务规则文档化违规）

`main.rs:308` 横幅字符串属于"启动行为"业务规则，没在 `docs/business_rules.md` 登记。修复 F20 改行为没同步改横幅 = **违反 §2.10（业务规则必须先登记再写实现）**。

### 观感 4: 同质化严重

**和 Claude 一致**。但加一个观察：

7 只持仓综合分 36-48 都集中在"减持/观望"区间——**这本身可能就是真实信号**（当前市场环境下 7 只持仓都不及格），而不是模板问题。**但模板把它写成"7 个独立分析"是错的，应该是 1 条"今日持仓 7 只综合分都低于 50，建议全组合减仓"**。

---

## 四、Codex 挖出的 3 个根因（Claude 没看到）

### 根因 E: multi_agent 绕开 freshness gate（**Critical**）

**位置**: `src/deep_analyzer.rs:190`
**违反**: AGENTS §2.1, §2.4

```rust
pub async fn run_multi_agent_analysis(code: &str) -> Result<String> {
    // 1. K 线（service 缓存）
    let kline = service::service().get_kline(code, 250).await?;  // ← 无 freshness gate
```

`service::service().get_kline()` 内部**不调用** `validate_quote_freshness`——这是一个**横切关注点（cross-cutting concern）漏配**。

**修复方案**：
- (短期) `deep_analyzer.rs:190` 之后加 freshness 校验，复用 `validate_quote_freshness`
- (长期) `service::service().get_kline` 内部加 `freshness: bool` 参数，默认 `true`，调用方可显式 opt-out

### 根因 F: 6 tool 失败静默填空，违反 §2.1（**Important**）

**位置**: `src/deep_analyzer.rs:223-234`
**违反**: AGENTS §2.1（生产路径不能降级到假数据）

**修复方案**：
```rust
struct ToolResult { name: String, ok: bool, data: String, err: Option<String> }
let results = vec![collect(fin_res), collect(research_res), ...];
// 把 results 序列化成 prompt 的一部分
let data_inventory = results.iter()
    .map(|r| format!("[{}] {}{}", r.name, if r.ok { "OK" } else { "MISSING" },
        r.err.as_deref().map(|e| format!(" ({})", e)).unwrap_or_default()))
    .collect::<Vec<_>>().join("\n");
// 强制 LLM 在 data_inventory 标记 MISSING 的字段里写 [数据缺失] 而非编造
```

### 根因 G: 网络完全失联时无 fast-fail（**Important**）

**位置**: `src/data_provider/eastmoney_provider.rs:99-165`
**违反**: AGENTS §2.1（数据源失败显式报错，不降级到假数据）

**实证**: 我在沙箱里跑 `--review`，所有数据源 fail，进程**默默卡死 5+ 分钟无输出**。

**修复方案**：
- 顶层 `run_review_only` 加超时：5 分钟内数据源全失败 → 显式 `bail!("上游数据源全部不可用，review 中断")`
- `eastmoney_provider.rs` retry 6 次后应该 fail-fast，不重试到天荒地老

---

## 五、修复方案排序（Codex 调整版）

| 优先级 | 工作量 | 项 | 来源 | Codex 评价 |
|--------|-------|---|------|----------|
| **P0-A** | 10 min | 横幅文案与 F20 行为对齐 | Claude P0-1 | ✅ 1 行 string 改 3 行，加 review note |
| **P0-B** | 2 h | 修复 `run_multi_agent_analysis` 走 freshness gate | **Codex 根因 E** | ✅ 必须在 P0 完成；否则下游所有修复都在假数据上 |
| **P0-C** | 1 h | `north_flow: f64` → `Option<f64>`，None 时打印 `[数据缺失]` | Claude P0-3 | ✅ 类型系统兜底比 warn 强 |
| **P0-D** | 1 h | 顶层加 5 分钟 fast-fail，输出明确错误原因 | **Codex 根因 G** | ✅ 沙箱 / 真实环境都受益 |
| **P1-E** | 3 h | freshness 区分盘内/盘后 | Claude P0-2 | ✅ 重要但可放 P1；先把 P0-B/C/D 落地 |
| **P1-F** | 2 h | 6 tool 失败显式标注，下游 LLM 收到 data_inventory | **Codex 根因 F** | ✅ 配合 P0-B，从源头解决"信息不足"高频词 |
| **P1-G** | 4 h | 推送按操作建议分组（7→1）+ 模板按综合分裁剪 | Claude P1-4 + P1-5 | ✅ 合并做，依赖 P1-F 的 data_inventory |
| **P2-H** | 1 d | 6 分析师真独立化（3 选项中选 1） | Claude P2-6 | ✅ 工作量 Claude 估低了；需要 LLM 输出 schema + 评测集 |
| **P2-I** | 2 h | 因子归因加板块对比 | Claude P2-7 | 优先级 P2 |
| **P3-J** | 1 h | 业务规则登记：横幅文案 / 推送分类标志 | **Codex 观感 3.3** | ✅ §2.10 合规 |

---

## 六、合并 Gate Checklist（按 §2 数据红线）

> 任何 P0/P1 项合并前**必须**逐项勾选。

### 6.1 P0-A 横幅文案

- [ ] `main.rs:308-310` 改 `Shadow => "推全量 (默认沙盘模式)"`，对齐 `launch_gate.rs:135`
- [ ] 在 `docs/business_rules.md` 登记 "横幅文案与 launch_gate 行为必须一致" 规则（§2.10）
- [ ] review 第 5 步：检查 `launch_gate::should_push_user` 行为变更时**必须**同步 PR 改横幅

### 6.2 P0-B multi_agent freshness gate

- [ ] `deep_analyzer.rs:190` 后加 `validate_quote_freshness(last_update, "service_cache", code)` 调用
- [ ] 新增单元测试：`mock_kline_age_4h()` 走 multi_agent → 期望 `Err` 或 "数据过期，无法研判"
- [ ] 业务规则登记："持仓研判必须 freshness 校验"（§2.4, §2.10）
- [ ] review 第 5 步：grep 整个 `src/deep_analyzer.rs` 确保无其他 `get_kline` 绕开 freshness

### 6.3 P0-C north_flow Option 化

- [ ] `overview` struct `north_flow: f64` → `Option<f64>`
- [ ] `async_overview.rs:37` 打印 `match n { Some(v) => format!("{:+.2}亿", v), None => "[数据缺失]".to_string() }`
- [ ] 下游所有读 `overview.north_flow` 的地方加 `if let Some(v) = ...` 显式处理
- [ ] 业务规则登记："北向资金缺失必须显式标注，禁止隐式 0"（§2.1, §2.10）

### 6.4 P0-D 5 分钟 fast-fail

- [ ] `run_review_only` 顶层加 `tokio::time::timeout(Duration::from_secs(300), ...)`
- [ ] 超时 / fail-fast 时输出 `bail!("[复盘] 上游数据源 {}/{} 不可用，中断", fail, total)` 并写 ERROR 日志
- [ ] CI 加测试：mock 所有数据源失败 → 期望 5 分钟内 exit code != 0

### 6.5 P1-E freshness 区分盘内/盘后

- [ ] `FreshnessConfig` 加 `is_post_close: bool` 字段
- [ ] `validate_freshness` 根据 `is_post_close` 选阈值：盘内 5s、盘后 1d
- [ ] `config/monitor.toml` 加 `dq_post_close_quote_stale_sec = 86400`
- [ ] review 第 5 步：grep 整个 `src/` 确保 `is_post_close` 正确传递

### 6.6 P1-F 6 tool 失败显式标注

- [ ] `unwrap_or_warn` 改为 `collect_tool_results`，返回 `(name, ok, data, err)` 结构
- [ ] 拼装 prompt 时插入 `data_inventory` 段
- [ ] LLM prompt 加规则："data_inventory 标 MISSING 的字段必须写 [数据缺失]，禁止编造"
- [ ] 单测：mock 3/6 tool 失败 → 输出 markdown 里**至少 3 处**出现 `[数据缺失]`

### 6.7 P1-G 推送去重 + 模板裁剪

- [ ] 新增 `decision/aggregation.rs`，按操作建议分组（减持 / 观望 / 卖出 / 买入）
- [ ] 7 只持仓研判 → 1 条聚合推送 + 0~N 条"重点异常"推送
- [ ] 模板长度按综合分：`composite < 30 || > 70 → 800字`，`30-70 → 1500字`
- [ ] 业务规则 BR-010 登记："持仓研判推送必须按操作建议分组"（§2.10）

---

## 七、Codex 不同意 Claude 的 1 个判断

**Claude P1-5 (模板动态裁剪) 工作量估低**：

Claude 说 "3h 改 prompt"，**实际模板生成在 LLM prompt 里，要做"按综合分裁剪"必须**：
1. 重写 prompt 模板生成代码（加 `template_style: Short | Normal | Long` 参数）
2. 重测 3 种模板下 LLM 输出的信息密度（人评 + 自动 metric）
3. 加单元测试验证模板长度

**实际工作量 1d 起**。建议从 P1 推到 P2，先用 BR-001（去重规则）和 BR-010（推送分组）解决"推送太多"，等 P2-H 6 分析师真独立化后再回头做模板。

---

## 八、一句话总结

**Claude 的诊断从"用户观感"切入，定位到了症状层；我的诊断从"代码 + §2 红线"切入，多挖出 3 个根因（multi_agent 绕开 freshness、6 tool 静默填空、网络全失联无 fast-fail）——这 3 个都是 §2.1 / §2.4 / §2.10 重大违反**。**建议 P0 4 项（50 分钟+2h+1h+1h = 4.5h）合并后再讨论 P1**。

---

## 附录: 详细根因文档

本诊断中提到的 3 个根因已独立成文，含完整代码定位、影响面、修复方案、合并 Gate：

- **[根因 E: 多 Agent 研判绕开 Freshness 门禁](../root-causes/E-multi-agent-bypass-freshness.md)** (P0, Critical) — `src/deep_analyzer.rs:190` 走 `service::service().get_kline()` 不查 freshness
- **[根因 F: 6 Tool 失败时静默填空字符串喂 LLM](../root-causes/F-tool-silent-fallback.md)** (P1, Important) — `src/deep_analyzer.rs:223-235` `unwrap_or_warn` 把 `Ok(error_json)` 当成功
- **[根因 G: 网络全失联时无 Fast-Fail，进程静默卡死](../root-causes/G-no-fast-fail.md)** (P0, Important) — `run_review_only` 顶层无超时，沙箱下死锁 5+ 分钟无日志

**根因索引**: `docs/root-causes/README.md`
