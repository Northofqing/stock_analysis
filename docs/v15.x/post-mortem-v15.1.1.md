# v15.1.1 Post-mortem — 推送静默事故

> 日期: 2026-07-12
> 严重度: **P0** — 生产推送完全中断
> 影响: 一整天所有 DailyReport / ReviewMarket / NewsCatalyst 等 non-critical 推送都没发出去
> commit: `4394962` (A2.4 默认 stage Shadow → Gray) → revert in `4679d1f`

---

## 事故时间线

1. `4394962` (A2.4) 把 `current_stage()` 默认从 `Shadow` 改为 `Gray`
2. 注释写："更安全, 默认只推 critical alert"
3. 我的 commit msg 没提醒"会中断默认部署推送"
4. 用户当天收不到推送，反馈"今天所有的消息都没有推送出来"
5. 诊断：Gray + `is_critical_alert=false` ⇒ `launch_gate_check` 拦掉所有 non-emergency
6. 回滚 commit `4679d1f`

## 根因分析 (5 Whys)

**Why 1**: 默认 stage 改成 Gray 让所有 non-critical 推送被拦
**Why 2**: Gray 的 `should_push_user(stage, is_critical_alert=false) = false` 是 by design
**Why 3**: 我把默认从 Shadow 改成 Gray 想要"理论安全"
**Why 4**: 我认为"默认只推 critical"比"默认推全量"风险低
**Why 5**: **我把"理论安全"和"实操可见性"混淆了** — 静默的失败比多推的失败难发现得多

## 核心教训

> **默认值必须是"可见、可测、可恢复"的状态，不能是"沉默失败"的状态。**

应用到生产系统：
- ❌ 默认静默（Gray 默认）— 失败不可见
- ✅ 默认全量（Shadow 默认）— 默认行为可见，多推可被人工忽略
- ✅ 静默/灰度必须用 **env var 显式强制开启**（`STAGE=Gray`）

## 通用规则（写进 CLAUDE.md）

1. **默认值原则**：所有"行为开关"的默认值必须是"出声"的状态（推全量、记录详细日志、显示警告）。任何"静默"的默认值都需要 env var 显式声明才能生效。
2. **修改默认值前**：先在 commit msg 标注 "⚠️ BREAKING: 默认行为变更" 并写明回滚方法。
3. **测试覆盖默认值**：新增默认值必须有测试断言（如 `test_current_stage_default_shadow`），不允许"默认值无测试覆盖"。
4. **静默路径可见**：任何会跳过推送/告警/任务的路径，必须在启动时打印一次当前 mode + 在每次跳过时 warn 一行，方便监控发现。

## v15.x 相关 commit

- `4394962` — ⚠️ A2.4 引入 (Gray 默认) — 已 revert
- `4679d1f` — ✅ v15.1.1 revert 回 Shadow 默认

## 后续任务影响

Phase D（v15.2 全量新闻分析）涉及新增大量 PushKind、entity extractor、stock mapper，必须遵守上述规则：
- 新增"aggregation_pass"或"filter_mode"等参数，默认必须关闭并打印
- 新增"silent_skip"路径必须 log warn
- 新增"默认过滤某些新闻类型"必须有显式 opt-in 配置

---

## Checklist (给后续 contributor)

- [ ] 修改任何默认 stage / mode / level 时，先看 `docs/v15.x/post-mortem-v15.1.1.md`
- [ ] 在 commit msg 里搜 `BREAKING` 关键字
- [ ] 测试覆盖新默认值
- [ ] 验证生产部署行为变化（不只是 `cargo test --lib` 通过）
- [ ] 在 `docs/v15.x/` 写 post-mortem 如果新默认值再次引起事故