# 推送 Foundation W09 终态权威重查设计

**状态：** 已冻结设计，待实现与 fresh 门禁。W09 只建立 authority 端口、完整绑定校验和 finalize 前二次重查能力；不接生产 durable DB，不发送消息，不推进业务状态或通知游标。

**决策日期：** 2026-09-07

## 1. 目标与证据边界

W09 落实 WBS 的唯一验收句：构造 `VerifiedTerminalRef` 和 finalize 前都必须重新查询 authority，并验证 authority、decision、rendered bytes、subject 以及 `TerminalBinding` 的全部字段；兼容层弱 audit 永远不能冒充 receipt。

权威输入为：

- `docs/push-system/push-system-implementation-rfc.md` 的 `VerifiedTerminalRef`、`DeliveryResult`、`TerminalBinding` 与终态完成合同；
- `docs/push-system/push-system-wbs.v1.json` 的 W09 scope、依赖与验收句；
- `docs/Project_Architecture_Blueprint.html` 的 business DB / durable DB 物理隔离、跨库不原子和 finalizer 重验边界；
- W02 的不透明 `VerifiedTerminalRef`、W03 的注册完成策略、W05 的模板与渲染身份、W08 的已 attested immutable intent snapshot。

W09 不实现 W10 业务 CAS/finalizer，不实现 W12 的具体 durable adapter，不打开生产数据库，不改变 `src/bin/monitor`、notification、durable delivery、配置或生产 DDL。

## 2. 深模块与权限边界

新增 `src/push_foundation/terminal_authority.rs`。调用关系固定为：

```text
attested IntentSnapshot + TerminalTemplateBinding + registered CompletionPolicy
                                  |
                                  v
                    TerminalBindingExpectation
                                  |
                     private TerminalAuthorityPort
                        requery(decision_id)
                                  |
                                  v
                    AuthorityTerminalRecord
                                  |
       exact field checks + evidence hash + TerminalBinding hash
                                  |
                  opaque VerifiedTerminalRef

finalizer candidate:
  prior VerifiedTerminalRef + same immutable expectation
      -> authority requery again
      -> full verification again
      -> compare stable prior/fresh binding
      -> non-clone FinalizationTerminalRef capability
```

`TerminalAuthorityPort`、原始 authority record 和 finalization capability 仅在 crate 内可见。外部调用方不能提交任意 parts 构造 `VerifiedTerminalRef`。W12 的具体适配器必须位于受控库模块中实现此端口；兼容 NotificationService 不实现该端口。

## 3. Intent 预期绑定

W08 `IntentSnapshot` 每次生成 W09 expectation 时必须再次运行自身完整 integrity 校验，而不能信任缓存或调用方复制字段。只接受 `InitialDecisionKind::Ready`，并重建/核对：

- namespace；
- stable intent id 与 decision id；
- Unit、completion owner；
- occurrence 与 business date；
- subject 与 audience；
- persisted rendered bytes 的现场 SHA-256；
- persisted template binding SHA-256。

`TerminalTemplateBinding` 的规范材料为 `template_id`、`template_version`，domain 为 `TemplateBinding/v1`。其 canonical SHA-256 必须等于 W08 immutable `template_sha256`。这样即使 authority 返回了彼此一致但属于另一模板的 ID/版本，也不能与当前 intent 绑定。

完成策略必须来自已注册 `CompletionPolicy`：策略的 catalog Unit 和 completion owner 必须与 intent 精确一致，authority class 必须位于策略的 `allowed_authority`。W09 不允许调用方临时传入一个任意布尔值表示“已允许”。

## 4. Authority 端口与原始事实

端口按 exact `decision_id` 查询，返回且只返回三类事实：

- `Missing`：没有该 decision 的权威事实；
- `PendingSeal`：存在但尚未形成可验证终态；
- `Terminal(AuthorityTerminalRecord)`：完整终态候选。

端口 descriptor 固定 authority class 与 durable schema version。终态候选必须携带 RFC `TerminalBinding` 的全部字段，以及 exact authority evidence bytes、authority 声明的 evidence SHA-256 和 binding SHA-256。验证器必须：

1. 核对 query key、record decision 与 intent decision 三者相同；
2. 核对 record authority/schema 与端口 descriptor 相同且策略允许该 authority；
3. 逐项核对 namespace、intent、Unit、occurrence、business date、subject、audience、template ID/version、rendered SHA-256；
4. 现场计算 `SHA256(evidence_bytes)` 并核对 authority 声明值；
5. 按 `TerminalBinding/v1` 重新计算全字段 binding SHA-256，并核对 authority 声明值；
6. `Accepted`、`Rejected`、`Uncertain` 必须有 attempt；两个人工处置允许在已校验的尝试前路径中没有 attempt；
7. 仅在全部检查通过后构造不透明 `VerifiedTerminalRef`。

错误只返回稳定分类和不含业务值的字段名，不泄露 receipt、渲染正文、subject、路径或数据库内容。

## 5. TerminalBinding canonical 合同

domain 固定为 `TerminalBinding/v1`。canonical object 使用现有 canonical-v1 编码，并包含：

`ref_id, authority_class, namespace, decision_id, attempt_id, intent_id, unit_id, occurrence, business_date, subject, audience, template_id, template_version, rendered_sha256, terminal_disposition, evidence_sha256, durable_schema_version`。

`verified_at` 与 `binding_sha256` 自身不得进入摘要。namespace、subject 复用 W01 的同源 canonical 表示；Option attempt 使用 string 或 `null`，不能用空串。枚举字符串大小写固定为 RFC 名称。

authority record 的 `binding_sha256` 不是被信任的快捷结果：验证器总是重新计算后比较。`verified_at` 变化不得改变 binding；其余任一字段变化都必须改变 binding 或在 expectation 比较时阻断。

## 6. 构造与 finalize 二次重查

首次构造流程每次调用端口一次，并在完整校验成功后返回 `VerifiedTerminalRef`。该引用可投影为：

- `Accepted` -> `DeliveryResult::TransportAccepted`；
- `Rejected` -> `DeliveryResult::TransportRejected`；
- `Uncertain` -> `DeliveryResult::TransportUncertain`；
- 两个人工处置 -> `DeliveryResult::AlreadyTerminal`。

人工接受不得投影成 TransportAccepted。

finalize 前不能直接消费首次引用。`reverify_for_finalization` 必须再次调用 authority，重复所有校验，并要求 fresh 引用与 prior 引用的稳定 binding/ref 完全相同；只有这样才产生不可 Clone 的 `FinalizationTerminalRef`。W10 只能消费该 fresh capability。authority 在两次查询之间变为 missing、pending、错误、换 ref、换处置或任一绑定漂移时，均 fail closed 且不产生 capability。

## 7. 弱 audit 隔离

`CompatibilityEvidenceRef` 保持公开的观察类型，但没有到 `VerifiedTerminalRef`、authority record 或 finalization capability 的转换接口。端口的查询结果类型也没有 weak variant。已有 rustdoc `compile_fail` 门禁继续保留，并增加 W09 API 形状测试，证明弱证据无法进入强校验入口。

W09 不以“本地返回成功”“日志出现成功字样”“TCP 已连接”“渠道计数大于零”推断 authority receipt。上述信息只能留在 compatibility/operational observation。

## 8. 失败与恢复矩阵

| 情形 | 结果 | 禁止行为 |
| --- | --- | --- |
| authority missing / pending seal / unavailable | typed blocked error | 构造引用、推进业务完成 |
| authority class 未被策略允许 | authority mismatch | 改用 compat 成功结果 |
| decision 或任一业务绑定不一致 | field mismatch | 只比较 disposition 后放行 |
| evidence bytes/hash 不一致 | evidence hash mismatch | 信任声明 hash |
| canonical binding/hash 不一致 | binding hash mismatch | 重新写 authority 事实 |
| transport disposition 无 attempt | disposition/attempt mismatch | 当作尝试前拒绝 |
| finalize 二查与 prior 稳定引用不同 | prior reference changed | 使用旧引用继续 CAS |
| 仅 verified_at 改变 | 允许 | 把审计时间纳入稳定摘要 |

## 9. 完成门禁

1. `TerminalBinding/v1` golden preimage/hash，且 verified_at 排除测试。
2. authority、decision、bytes、subject 以及所有 TerminalBinding 业务字段逐项变异均阻断。
3. evidence exact bytes/hash 与 authority 声明 binding hash 都必须现场重算。
4. transport attempt 必填、人工处置 attempt 可空；五种 disposition 的 DeliveryResult 映射正确。
5. finalize 成功路径确实查询两次；两次间任一 authority 漂移均无 fresh capability。
6. compatibility compile-fail 保持，production 无 parts constructor。
7. W01--W09 回归、rustdoc、check、Clippy/rustfmt、架构文档验证器和 diff check 全部 fresh 运行。
8. production wiring 相对 W08 零改动，线上 monitor PID 不变。

W09 完成后仍没有生产接线。W10、W11、W12、W13--W21、52 Unit 迁移、shadow/live promotion 与生产 migration 继续保留为未完成范围。
