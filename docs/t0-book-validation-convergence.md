# 盘口校验收敛（T0Quote / OrderBooks 一个出口）

状态：**方案已定，待执行（2026-10-19 盘后）**
决策人：operator（2026-10-19 盘中「可以」）

---

## 要解决的问题

盘口数据现在有**三套校验、两个来源**：

| # | 位置 | 规则 | 谁用 |
|---|---|---|---|
| 1 | `convert.rs:744` `book_levels` | 普通解析 | OrderBooks |
| 2 | `convert.rs:772` `live_book_levels` | **要求恰好 5 档，否则报错** | OrderBooks（live） |
| 3 | `convert.rs:3914` `validate_t0_live_quote` | **每档 price 必须 > 0** | T0Evidence.T0Quote |

业务消费者只有 **T0Evidence**（做 T 挂单参考：`push_templates.rs:1691` 的「五档卖/买」「五档买/卖」）。
`OrderBooks` 只被 `grpc_local_readiness_probe` 用到，**不进任何业务路径**。

### 造成的实际故障

涨停时卖一为空、跌停时买一为空：

- `validate_t0_live_quote` 的 `require_positive_live_value` 直接拒绝整个 T0Evidence 批次
- 结果：**涨停/跌停的票完全没有做 T 建议**
- 而这恰恰是最需要判断「还能不能追、要不要挂单」的票

---

## 收敛方案

### 一个数据出口

业务侧盘口**只从 `T0Evidence.T0Quote` 出**。`OrderBooks` 保留但只服务 readiness 健康检查，
在 `main.rs:4816` 的能力清单处注明「不供业务」。

### 一条校验规则

新增唯一实现，三处调用：

```rust
/// 五档合法性的唯一实现。OrderBooks 和 T0Quote 都调它。
///
/// 空档：price == 0 && volume == 0 → 允许（涨停卖侧 / 跌停买侧）
/// 有量无价：price == 0 && volume > 0 → 拒绝（真异常）
/// 有价无量：允许（挂单被撤）
fn validate_book_side(
    side: &str,
    levels: &[(f64, f64)],   // (price, volume)
    capability: &'static str,
) -> Result<(), GatewayError>
```

改动：

| 文件 | 改什么 |
|---|---|
| `convert.rs:744` `book_levels` | 内部改调 `validate_book_side` |
| `convert.rs:772` `live_book_levels` | **删掉独立的「恰好 5 档」检查**，改调 `validate_book_side` |
| `convert.rs:3914` `validate_t0_live_quote` | 档位部分改调 `validate_book_side`（现价/开高低收的校验保留） |
| `t0_advisor.rs:579` | `bid_ask_ratio` / `ask_bid_ratio` 分母为 0 时改 `Option<f64>` |
| `push_templates.rs:1695` | 两个 ratio 为 None 时渲染 `—` |
| `main.rs:4816` | 能力清单处注明 OrderBooks 不供业务 |

### 不做的

- 不改 `T0BookLevel` 的结构（`price: f64` 保持）。
  理由：改成 `Option<f64>` 要动结构体、解析、校验、模板和三处 fixture，
  收益只是语义更干净，成本是全链路 + 序列化格式变。`price=0 && volume=0`
  已经能唯一表达空档，且不构成「补 0」（0 是「没有这一档」的标记）。
- 不删 `OrderBooks`（readiness 还要用）。

---

## 验收

1. `cargo test --lib convert` —— 三处校验的用例都要过
2. 找一只 **涨停票**跑 `T0Evidence`，确认：
   - 不再整批失败
   - `asks` 侧全是 `price=0, volume=0`
   - 推送模板显示「五档卖/买: —」而不是 `inf`
3. 找一只 **正常票**，确认五档数值与改前一致（不能有回归）

---

## 为什么这条比 D3 优先

| 项 | 影响 |
|---|---|
| D3 板块归属 | 排序里 20% 权重的一个维度，缺了只是分数低 |
| **T0Quote 拒空档** | **涨停/跌停票的做 T 建议直接消失** |

按 operator 明确的目的「做 T 的挂单参考」，这条是正路。
