# 根因索引

> 父文档: `docs/v9.4-review-output-诊断-codex-2026-06-30.md`（Codex 视角完整诊断）
> 关联: `docs/v9.4-review-output-诊断-2026-06-30.md`（Claude 视角诊断）
> 创建: 2026-06-30

本目录收录 Codex 在诊断 `monitor --review` 输出质量问题时挖出的、**Claude 诊断未覆盖**的根因。每个根因文档独立成文，含复现路径、代码定位、影响面、修复方案、合并 Gate。

---

## 根因清单

| ID | 标题 | 优先级 | 违反条款 | 文档 |
|----|------|--------|---------|------|
| **E** | 多 Agent 研判绕开 Freshness 门禁 | **P0 (Critical)** | §2.1, §2.3, §2.4 | [E-multi-agent-bypass-freshness.md](./E-multi-agent-bypass-freshness.md) |
| **F** | 6 Tool 失败时静默填空字符串喂 LLM | P1 (Important) | §2.1, §2.8 | [F-tool-silent-fallback.md](./F-tool-silent-fallback.md) |
| **G** | 网络全失联时无 Fast-Fail，进程静默卡死 | **P0 (Important)** | §2.1, §2.7 | [G-no-fast-fail.md](./G-no-fast-fail.md) |

---

## 优先级与依赖关系

```
P0-G (Fast-Fail)
   │  5min timeout 阻止下游连锁失败
   ▼
P0-E (Freshness Gate)
   │  阻塞路径后才能修数据质量
   ▼
P1-F (Tool data_inventory)
   │  LLM 不再编造
   ▼
P1 推送去重 + 模板裁剪（Claude P1-4/5）
P2 6 分析师真独立化（Claude P2-6）
```

**依赖关系**：
- **G 必须先做**：没有 fast-fail 时，沙箱/CI 跑 review 永远 5+ 分钟不退出，掩盖所有下游问题
- **E 紧接 G 之后**：multi_agent 是产生 1.1 万字噪声的主路径，freshness gate 立刻让 review 推送量下降
- **F 在 E 之后**：tool 失败标注是 LLM 输入质量的基础，否则 freshness 拒绝后 LLM 拿到空上下文会编造

---

## 工作量与 ROI

| 根因 | 推荐方案 | 工作量 | 立即收益 |
|------|---------|--------|---------|
| G | 顶层 5min timeout | 30min | CI/沙箱不再 5min 卡死，exit 2 显式失败 |
| E | multi_agent 加 freshness gate | 2h | 7 条持仓研判在数据过期时**直接拒绝推送**，节省 1.1 万字噪声 |
| F | data_inventory 显式标注 | 3h | LLM 输出含 `[数据缺失]` 而非"信息不足"模板话术 |

**P0 全部完成**：30min + 2h = 2.5h，立即让 review 推送从 14 条噪声变成可读内容。

---

## 与 Claude 诊断的差异

| 维度 | Claude 诊断 | Codex 补充 |
|------|------------|-----------|
| 横幅文案 | P0-1 已识别 | 补充根因：F20 修复引入的回归 |
| Freshness | P0-2 提议加 `is_post_close` | 补充根因 E：multi_agent 走 service 缓存**绕开**了现有 freshness gate |
| Tool 失败 | 未识别 | 根因 F：`Ok({"error": ...})` 假成功 + `unwrap_or_warn` 静默填空 |
| 网络失联 | 未识别 | 根因 G：沙箱 / 真实断网时无 fast-fail |
| 北向资金 | P0-3 提议改 warn | 补充：必须 `Option<f64>` 类型兜底 |
| 6 分析师独立化 | P2-6 | 补充：选项 (2) 温度差可能**加剧**同质化 |

---

## 验证命令

```bash
# 根因 G 验证：沙箱跑 review，5min 内必须 exit
cargo run --bin monitor -- --review 2>&1 | tail -5
echo "exit code: $?"
# 期望: exit code = 2, log 含 "5 分钟超时未完成"

# 根因 E 验证：grep multi_agent 路径无 freshness
rg -B2 -A5 "get_kline" src/deep_analyzer.rs | head -30
# 期望看到 (修复后): validate_quote_freshness 调用

# 根因 F 验证：mock 3 tool 失败，输出含 [数据缺失]
# (需要 mock 测试代码, 详见 F 文档 §5.3 单元测试)
```

---

## 相关文档

- `docs/v9.4-review-output-诊断-2026-06-30.md` — Claude 视角诊断
- `docs/v9.4-review-output-诊断-codex-2026-06-30.md` — Codex 视角综合诊断
- `docs/monitor-runs/2026-06-30/sandbox-blocked.log` — Codex 沙箱跑 review 的 543 行原始 log
- `docs/业务规则清单-registry.md` — 业务规则登记（BR-010/007/008 待新增）
