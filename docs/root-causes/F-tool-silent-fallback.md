# 根因 F: 6 Tool 失败时静默填空字符串喂 LLM

> **优先级**: P1（Important）
> **违反**: AGENTS.md §2.1（生产路径不能降级到假数据）+ §2.8（假实现禁令）
> **影响面**: 7 条持仓深度研判 × 6 tool = 42 个数据获取点
> **位置**:
>   - `src/deep_analyzer.rs:223-234` (`unwrap_or_warn` 函数)
>   - `src/agent/tools.rs:54-58` (`FetchFinancialTool::call` 失败时返回 `Ok(error_json)`)
>   - `src/agent/tools_news.rs:54-58` (`FetchNewsTool::call` 失败时返回 `Ok(error_json)`)
>   - `src/agent/tools_research.rs`, `tools_sector.rs:80-94`, `tools_chip.rs`, `tools_money_flow.rs` (同模式)

---

## 一、复现路径

### 1.1 触发条件

任意一种 tool 网络失败时：
- HTTP 请求超时（沙箱 / 真实断网）
- JSON 解析失败（数据源格式变更）
- 数据源返回空（节假日 / 停牌）
- 数据源无该票数据（次新股 / 退市股）

### 1.2 实际日志（盘后 + 数据源部分失败时）

```
[复盘] ▶ 多 Agent 研判 600703 三安光电
[MultiAgent] 开始抓取数据：600703
[MultiAgent] 数据抓取完成 — 财务=true 研报=true 新闻=true 板块=true 筹码=true 资金=true
```

**6 个 tool 全部 "true"** — 但实际可能是**3 个 tool 失败、3 个 tool 成功**。**这条 log 完全是假的**。

### 1.3 根因链

```
run_multi_agent_analysis(code)
  └─> tokio::join!(fin_tool.call, research_tool.call, ..., flow_tool.call)
      └─> 6 个并行 tool，每个都返回 Result<String>
          └─> 失败时返回 Ok(json!({"error": ...}).to_string())  ← 假成功
              ↓
      └─> unwrap_or_warn(label, r: Result<String>) -> String
          ├─> Ok(s) => s    ← 把 {"error": ...} 字符串当数据传
          └─> Err(e) => ""  ← 真的 panic 才走空字符串
              ↓
      └─> 把 "数据" 拼装到 LLM prompt
          └─> LLM 看到 {"error": ...} 但被告知"这是有效数据"
              └─> LLM 编出 1500 字"基于上述信息的"研判
                  └─> 推送 1,500 字噪声给用户
```

**关键点**：tool 的"成功但内容是错误"被**两层降级**——先是 tool 内部把错误 JSON 当成功返回，然后 `unwrap_or_warn` 把错误 JSON 当数据传，最后 LLM 把错误 JSON 当真数据用。

---

## 二、代码定位证据

### 2.1 `unwrap_or_warn` 把 Result::Err 降级为空字符串

```rust
// src/deep_analyzer.rs:220-235
let unwrap_or_warn = |label: &str, r: Result<String>| -> String {
    match r {
        Ok(s) => s,  // ← 即使 s 是 '{"error": ...}' 也当数据用
        Err(e) => {
            log::warn!("[MultiAgent] {} 工具失败: {:#}", label, e);
            String::new()  // ← 静默填空，下游 LLM 拿到空上下文
        }
    }
};
let fin_str = unwrap_or_warn("financials", fin_res);
let research_str = unwrap_or_warn("research", research_res);
let news_str_raw = unwrap_or_warn("news", news_res);
let sector_str = unwrap_or_warn("sector", sector_res);
let chip_str = unwrap_or_warn("chip", chip_res);
let flow_str = unwrap_or_warn("fund_flow", flow_res);

log::info!(
    "[MultiAgent] 数据抓取完成 — 财务={} 研报={} 新闻={} 板块={} 筹码={} 资金={}",
    !fin_str.is_empty(),       // ← "财务=true" 是假的
    !research_str.is_empty(),  // ← "研报=true" 是假的
    !news_str_raw.is_empty(),  // ← "新闻=true" 是假的
    !sector_str.is_empty(),   // ← "板块=true" 是假的
    !chip_str.is_empty(),      // ← "筹码=true" 是假的
    !flow_str.is_empty(),      // ← "资金=true" 是假的
);
```

**问题**：
- `s.is_empty()` 判定"成功"——但 `s = '{"error": "..."}'` 长度为 19+ 字节，**不为空**
- `s` 是错误 JSON 时**不打 warn**（因为是 Ok）
- 下游 LLM 看到 `{"error": ...}` 但被告知"上下文完整"

### 2.2 6 tool 全部用 `Ok(error_json)` 模式

**模式 1** — `FetchFinancialTool` (`src/agent/tools.rs:54-58`):
```rust
if fin.any() {
    let result = json!({...});
    Ok(result.to_string())
} else {
    Ok(json!({"error": "No financial records found"}).to_string())  // ← 假成功
}
```

**模式 2** — `FetchNewsTool` (`src/agent/tools_news.rs:54-58`):
```rust
if news_str.is_empty() || news_str.contains("未找到相关结果") {
     Ok(json!({"error": "No recent news found for this stock."}).to_string())  // ← 假成功
} else {
     Ok(news_str)
}
```

**模式 3** — `FetchSectorTool` (`src/agent/tools_sector.rs:80-94`):
```rust
let resp = self.client.get(&url)...send().await;
let body: Value = match resp {
    Ok(r) => match r.json().await {
        Ok(v) => v,
        Err(e) => return Ok(json!({"error": format!("板块接口 JSON 解析失败: {}", e)}).to_string()),  // ← 假成功
    },
    Err(e) => return Ok(json!({"error": format!("板块接口请求失败: {}", e)}).to_string()),  // ← 假成功
};
```

**模式 4** — `FetchChipDistributionTool`, `FetchFundFlowTool`, `FetchResearchTool` 全部同模式（已确认）。

### 2.3 tool 的 LLM 视角

LLM 拿到的 prompt 拼装（`src/deep_analyzer.rs:248-300`）：

```rust
// 拼装 extra_context（资金面分析师读取）
let mut extra = String::new();
if !flow_str.is_empty() {
    extra.push_str(&flow_str);
    extra.push_str("\n");
}
if !chip_str.is_empty() {
    extra.push_str(&chip_str);
    extra.push_str("\n");
}
// ... 同样处理 news_ctx, sector_ctx 等

// 最后注入到 LLM prompt：
let user_prompt = format!(
    "请基于以下数据对 {code} {name} 进行多角色分析：\n\
     \n\
     【财务数据】\n{fin_str}\n\
     【研报】\n{research_str}\n\
     【新闻】\n{news_ctx}\n\
     【板块】\n{sector_ctx}\n\
     【筹码】\n{chip_str}\n\
     【资金流】\n{flow_str}\n\
     ..."
);
```

**当 `fin_str = '{"error": "No financial records found"}'` 时**：
- LLM 看到 `[财务数据]\n{"error": "No financial records found"}`
- LLM 的训练数据让它**倾向于"补全"上下文**——它会编"该公司 ROE 较低、毛利率一般"等
- 用户看到的就是"信息不足" + 因子归因数字（Claude 观感 2）

---

## 三、影响面分析

### 3.1 直接违反

- **§2.1**: "数据源失败显式报错，不降级到假数据" — **降级了**（错误 JSON → LLM 编造）
- **§2.8**: "任何'写数据/验证/通知/同步'类函数... 必须真实操作目标数据源。仅写日志不操作数据的实现视为假实现" — **精神违反**（tool 假装"获取数据"，实际返回错误包装）
- **§2.7**: "关键数据流与每一笔订单留痕：来源、时间、决策依据" — **决策依据被污染**

### 3.2 数据流污染

| Tool | 失败时返回 | LLM 看到 | LLM 输出 |
|------|----------|---------|---------|
| `fetch_financials` | `{"error": "No financial records found"}` | 错误 JSON | 编造财务数字 |
| `fetch_news` | `{"error": "No recent news found for this stock."}` | 错误 JSON | 编造新闻摘要 |
| `fetch_sector` | `{"error": "板块接口请求失败: ..."}` | 错误 JSON | 编造板块归属 |
| `fetch_research` | `{"error": ...}` | 错误 JSON | 编造研报观点 |
| `fetch_chip` | `{"error": ...}` | 错误 JSON | 编造筹码分布 |
| `fetch_fund_flow` | `{"error": ...}` | 错误 JSON | 编造资金流 |

**6 个 tool 全部失活** → LLM 看到 6 段错误 JSON → 输出**完全编造的 1500 字研判**。

### 3.3 业务后果

- **用户观感 2 "有用信息不多"** 的根因：6 tool 失败率高 → LLM 拿到的都是错误信号 → 输出"信息不足 / 评分中性 / 信号分歧大"等模板话术
- **用户观感 3 "数据准确性不对"** 的部分根因：LLM 编造的财务数字可能与真实数据**对不上**
- **用户观感 4 "同质化严重"** 的根因：6 个 tool 输入全是 `{"error": "..."}` 同模板 → LLM 输出 6 份"信息不足"同模板

---

## 四、修复方案

### 方案 α: 最小侵入 — `data_inventory` 显式标注（**推荐 P1**）

**位置**: `src/deep_analyzer.rs:223-235` + 后续 prompt 拼装处

**实现**:
```rust
// 替换 unwrap_or_warn
struct ToolResult {
    name: String,
    ok: bool,
    data: String,
    err: Option<String>,
}

let collect = |label: &str, r: Result<String>| -> ToolResult {
    match r {
        Ok(s) if s.contains("\"error\"") || s.is_empty() => {
            // 即使 Ok 也可能是错误 JSON（"假成功"模式）
            let err = s.lines().next().unwrap_or("unknown").to_string();
            log::warn!("[MultiAgent] {} 工具返回错误: {}", label, err);
            ToolResult { name: label.into(), ok: false, data: String::new(), err: Some(err) }
        }
        Ok(s) => ToolResult { name: label.into(), ok: true, data: s, err: None },
        Err(e) => {
            log::warn!("[MultiAgent] {} 工具失败: {:#}", label, e);
            ToolResult { name: label.into(), ok: false, data: String::new(), err: Some(e.to_string()) }
        }
    }
};

let results = vec![
    collect("financials", fin_res),
    collect("research", research_res),
    collect("news", news_res),
    collect("sector", sector_res),
    collect("chip", chip_res),
    collect("fund_flow", flow_res),
];

// 构造 data_inventory 段
let data_inventory = results.iter()
    .map(|r| format!("[{}] {}", r.name, if r.ok { "OK" } else { "MISSING" }))
    .collect::<Vec<_>>().join("\n");

// 拼装 prompt
let user_prompt = format!(
    "请基于以下数据对 {code} {name} 进行多角色分析。\n\
     \n\
     【数据可用性清单】（MISSING 表示该数据源失败，禁止编造）\n\
     {data_inventory}\n\
     \n\
     【财务数据】\n{fin_data}\n\
     【研报】\n{research_data}\n\
     ...\n\
     \n\
     强制规则：\n\
     1. data_inventory 标 MISSING 的字段，在结论中必须写 [数据缺失]，禁止编造\n\
     2. 至少 N 个数据源成功才给出综合分；不足 N 个时综合分标 N/A\n\
     ..."
);
```

**工作量**: 3h
**收益**:
- LLM 收到明确的 data_inventory，不会编造
- 用户看到 `[数据缺失]` 而非假数据
- "信息不足" 高频出现问题解决

### 方案 β: 中等侵入 — tool 接口改为 `Result<Data, ToolError>`（**P1 推荐 + 方案 α**）

**位置**: 6 个 tool 文件

**实现**: 改 `Tool::call` 签名为 `Result<ToolOutput, ToolError>`，区分"成功"和"成功但内容是错误"。

**工作量**: 1d（影响 6 个 tool + Tool trait + 所有调用方）

### 方案 γ: 彻底重构 — 错误码标准化（**P2**）

引入统一错误码 `enum ToolError { Network, Parse, Empty, RateLimit }`，LLM 可以基于错误码做差异化处理（如网络失败 → 标注 `[网络异常]`，空数据 → 标注 `[无新闻]`）。

**工作量**: 1w

### 推荐路径

**先做方案 α**（3h P1）— 改 `unwrap_or_warn` + 加 `data_inventory` 注入 prompt，**立即可见**：
- 用户看到 `[数据缺失]` 而非"信息不足"
- LLM 不会编造
- "同质化"问题缓解

**同步做方案 β**（1d P1）— 让 tool 真正能区分成功/失败，长期根除。

---

## 五、合并 Gate Checklist

### 5.1 修复完整性

- [ ] `src/deep_analyzer.rs:220-235` `unwrap_or_warn` 替换为 `collect`（返回 `ToolResult`）
- [ ] `data_inventory` 段注入 LLM prompt
- [ ] prompt 加强制规则："MISSING 字段必须写 [数据缺失]，禁止编造"
- [ ] log 改用 `data_inventory` 显示真实成功/失败（不再用 `!str.is_empty()`）

### 5.2 业务规则登记（§2.10）

- [ ] `docs/business_rules.md` 新增 **BR-011: Tool 失败时禁止静默填空字符串，必须显式标注 data_inventory**

### 5.3 单元测试

- [ ] 测试 1: mock 3/6 tool 失败 → 输出 markdown 里**至少 3 处** `[数据缺失]`
- [ ] 测试 2: mock 全部 tool 成功 → 输出不含 `[数据缺失]`
- [ ] 测试 3: mock `Ok(json!({"error": ...}))` → 视为失败，log warn
- [ ] 测试 4: 测试 LLM prompt 拼装：data_inventory 段必须出现

### 5.4 集成测试

- [ ] CI 加场景：`cargo run --bin monitor -- --review`，grep 输出 "数据缺失" 字符串应出现至少 1 次（因为总有几个 tool 会失败）
- [ ] 沙箱跑（无网络）：所有 tool 应全部标 MISSING，输出大量 `[数据缺失]`

### 5.5 旧模块接入检查表

- [ ] 列出所有调用 6 tool 的位置：`rg "fin_tool|research_tool|news_tool|sector_tool|chip_tool|flow_tool" src/`
- [ ] 对每个调用点：是否有 `data_inventory` 拼装？
  - 已接入 → 记录 PR
  - 未接入 → 记录未接入理由

### 5.6 Review 检查（§2 红线）

- [ ] review 第 5 步：grep `src/deep_analyzer.rs` 确认 prompt 拼装含 data_inventory 段
- [ ] review 第 5 步：检查 `docs/business_rules.md` BR-011 登记完整
- [ ] review 第 5 步：检查单元测试覆盖 mock 失败场景

### 5.7 数据红线（§2）

- [ ] §2.1: 不静默降级到假数据 ✅（LLM 收到明确 MISSING 标记）
- [ ] §2.8: 不允许假实现 ✅（tool 失败时显式标注）
- [ ] §2.10: 业务规则登记 ✅（BR-011）
