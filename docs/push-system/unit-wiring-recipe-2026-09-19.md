# 把一个 Unit 接进正式循环：接线配方与已知陷阱

2026-09-19。本文把一次**已回退**的接线尝试（`MU-paper-sell`）换来的知识固化，使剩余 Unit 的接线成为机械执行。**不是设计文档**，是操作清单 + 陷阱表。

## 0. 适用对象

把一个「旧 push 路径」的 Unit 从裸 `push_governor_v3(...)` 接到 **counted 持久投递**（与 `T-10-paper-trade` / `holding_plan` 同级）。本文只覆盖"接线"，不含业务语义。

## 1. 七个触点（缺一个就不成立）

| # | 文件 | 动作 | 漏掉的后果 |
|---|---|---|---|
| 1 | `src/durable_delivery/model.rs` | `PushKind` 加变体 + `ALL` 计数 +1 + `as_str()` + `stable_template_id()`（`<kind>_v1`） | 编译失败（ALL 计数）或 `parse()` 反解失败 |
| 2 | `src/durable_delivery/model.rs` | **`compiled_policy_catalog()` 加一行** `(Kind, CooldownScope, Some(秒)/None, WindowMode)` | **阻断项 A：静默关停该 Unit 全部推送**（见 §2） |
| 3 | `src/bin/monitor/durable_delivery_runtime.rs` | kind 映射臂 `K::Kind => (D::Kind, DeliverySubKind::None)` | `is_counted_kind` 为 false，退回旧路径 |
| 4 | `src/bin/monitor/presentation_registry.rs` | 加 `descriptor("T-XX-...", PushKind::Kind, "producer_seam", "render_seam")` + **数组类型 2 处 +1** | 编译失败 |
| 5 | `src/bin/monitor/br196_test_delivery.rs` | `ACTIVE_PRESENTATIONS` 加元组 + 计数 +1 | 双射门失败 |
| 6 | 同上 | `ALL_PUSH_KINDS` 加变体 + 计数 +1；kind-cover 的 `64` 两处 +1；lifecycle 矩阵**两分支 × 2 组** +1；descriptor 计数 +1 | BR-196 单元门逐个失败（只能靠跑测试发现） |
| 7 | `src/bin/monitor/push_templates.rs` | 加 `prepare_*()`（出 `(text, CountedDeliveryBinding)`）与 `dispatch_*()`（`acquire_token` → `push_counted_with_binding`）；把旧调用点改过去 | 接线未生效 |

**触点 6 是维护陷阱**：为一个呈现单元要在 4 个位置改 ~7 个计数常量，且只能靠逐个测试失败发现。建议后续把它们收敛为从 `descriptors()` 派生的单一事实源。

## 2. 阻断项 A（本次尝试的实际死因）

`PaperSell` 被注册为 counted kind 后，`DeliveryEnvelope::new`（`model.rs:820-831`）**强制要求** `compiled_policy_catalog()` 里有一行匹配 `(push_kind, sub_kind)`：

```rust
let policy = compiled_policy_catalog().into_iter()
    .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
    .ok_or_else(|| DurableDeliveryError::PolicyMismatch(...))?;
```

**没有那一行 → `deliver_counted_binding` 必然 `Denied` → 不发卡、不写 durable 行**，而旧代码至少会尝试发送。**即：接线不完整会把"失败时才丢"变成"永远不发"。**

> 核对此目录时注意：`compiled_policy_catalog` 内部 `use PushKind::*;`，行里写**裸变体名**（如 `(PaperTrade, PerTicket, Some(300), Rolling)`）。用 `grep "PushKind::"` 会**零命中而被误判为"不存在"**——零命中不等于不存在。

## 3. 必须先做的两个决策（不能默认）

1. **是否计入 30 条/日预算**：`counts_against_daily_budget` 对不在 BR-237 豁免名单的 kind 默认 `true`。计入 ⇒ 该 Unit 新进入 30 槽竞争，可被 `DailyBudgetFull` 拒绝（**即 8/13 复盘被饿死那次的机制**）。涉及资金动作的卡不宜被无关预算饿死。
2. **occurrence identity**：`(business_date, code)` 还是别的？须与该 Unit 既有的"当日一次"不变式一致（例：`holding_plan` 用 `holding-plan:{date}:{code}`，与 `paper_sell::already_sold_today` 同形）。

## 4. Binding 的构造范式

镜像 `src/bin/monitor/holding_plan.rs:106-135`（已审同类实现）：

```rust
let canonical = serde_json::json!({ /* 该 Unit 的事实 */ });
let canonical_bytes = canonical.to_string().into_bytes();
let subject_hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&canonical_bytes));
let exchange = if code.starts_with('6') { Exchange::Shanghai } else { Exchange::Shenzhen };
let instrument = InstrumentId::new(exchange, code, AssetClass::Equity)?;
CountedDeliveryBinding::new(
    business_date,
    format!("<unit>:{business_date}:{code}"),
    canonical_bytes,
    CountedDeliveryScope::Ticket { instrument },
    subject_hash,
    CountedDeliveryOrigin::InternalDurable,
    None,
    true,   // retry_authorized
)
```

注意 Exchange 启发式（`starts_with('6')`）**不覆盖 BJ/ETF**；P-04 用的是权威的 `resolve_production_equity`。新接线应优先后者。

## 5. 验收路径（本次尝试的阻断项 B）

BR-196 有**两条**验收路径，别混淆：

| 路径 | 命令 | 是否覆盖严格矩阵 |
|---|---|---|
| 隔离审计 | `monitor --test --push-dry-run` | **不覆盖**——它 `bypass live acceptance gate` |
| Live opt-in | `BR196_LIVE_FEISHU_ACCEPTANCE=1` + Test 环境 | 覆盖，但**会真实外发飞书** |

新增单元还必须补 `T-XX` 的 **rendered preview** 与 `EXPECTED_CATALOG_TOTAL`，否则 `build_active_catalog` 会报 `BR-196 Active family missing rendered preview`。

## 6. 幂等陷阱（不要照直觉改）

**不要把 `modes.rs` 那类 `Ok(false)`/`Err` 直接改成 `Err` 来"让失败可重试"** —— 项目已有结论（`2026-09-11-chain-post-close-next-seam.md`）：`false` 不证明未发送（微信/飞书分片，**前片可成功**），且重试会重跑业务写、**不具幂等**。正确前提是"区分明确未尝试/可重试 与 可能已被接受/未决"。

## 7. 每片的交付纪律（本会话验证有效）

前提核实（**先确认能否拿到行为 RED**）→ 行为 RED → 实现 → 回归（`cargo check --all-targets` + 相关模块级套件）→ **独立复审** → 提交。

> 本会话 6 次自述被复审推翻，其中一次推翻的是我用来自证的**守侧测试本身**（它被自己的注释字串骗过）。复审不是仪式。

## 8. 边界

- 本文不含任何源码改动，是操作清单。
- 文档位于 `docs/push-system/`（`.gitignore` 白名单模式），是否纳管需加 `!` 规则。
- 未部署、未改生产闸、未运行 monitor。
