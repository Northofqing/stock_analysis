# 手动推送：健康准备与失败结果实施记录

日期：2026-09-10。状态：本局部任务完成；初审重要问题已修复，限定复审通过，保留既有验证噪声 Minor。基线 `43f48933de599acd9c8207d72d6610ded2a08ba4`，初版 `de990c8ca07262765fae454c1e23562ad35a0827`，最终源码 `41e7762301498302266b8780866e9fa47c5315a5`。以下源码行号对应最终源码。范围见[实施计划](../superpowers/plans/2026-09-10-manual-push-bootstrap-outcome.md)，原问题见[HoldingPlan 调用链](holding-plan-call-chain-2026-09-10.md)。本项不启动生产 monitor，不读写实际数据库、不调用真实 provider/sink，不代表上线。

## 修改合同

| 当前窗口 | 健康准备与执行 | 最终结果 |
| --- | --- | --- |
| [Preopen](../../src/bin/monitor/manual_push.rs#L44) | 不新增健康/投递动作；P-01 保留常驻 scheduler 或专属补偿 owner | 拒绝，由原 CLI 错误分支退出 2 |
| [Intraday](../../src/bin/monitor/manual_push.rs#L50) | 先真实 refresh，再读取新 banner；失败不能复用旧缓存。成功后依序 I-01、I-02、I-03、D-01、I-04 | 未确认项保留并继续后续任务；存在失败即 Err |
| [Evening / Outside](../../src/bin/monitor/manual_push.rs#L88) | 为 A-01 准备 banner；失败记录 A-01 并跳过它，仍执行独立的 A-10 | 任一失败即 Err；全部确认才 Ok |

[真实 adapter](../../src/bin/monitor/manual_push.rs#L134)复用 [refresh_banner_state](../../src/bin/monitor/main.rs#L2023)，不调用会发 T-01/T-02 或写状态的健康通知 hook。原 refresh 在数据缺失时保留账户 incomplete 与真实 DataMode；不存在“填个正常 banner 就保证发送”的承诺。[generic CombinedAccount gate](../../src/bin/monitor/v14_adapter.rs#L438)读取 banner，[counted gate](../../src/bin/monitor/v14_adapter.rs#L493)则按 requires_banner 选择来源。A-01 用前者、A-10 属于后者的 CountedSourceOnly，这是两条盘后故障路径必须分开的原因。

实际 [run_daily_pushes](../../src/bin/monitor/main.rs#L1479) 只捕获一次时刻并转发到同一手动 runner。[批尾结果](../../src/bin/monitor/manual_push.rs#L110)直接由[原 CLI 分支](../../src/bin/monitor/main.rs#L5193)消费，保留 JSONL flush 后退出 2/0。新 module 集中编排和失败聚合，真实 adapter 沿用原 dispatcher、[HoldingPlan token/逐项 counted 投递](../../src/bin/monitor/manual_push.rs#L158)与失败明细。测试使用自有内存 adapter，不复制第二份业务编排，不运行整个 main 的启动/reconcile/webhook 路径。

## 已取得的过程证据

- 保持行为提取基线：1 项通过，原“没有 refresh”和“批尾 Ok”行为仍保留；结合真实入口接线核对，确认测试与生产使用同一 runner。
- 第一 RED：编译成功后，新进程测试实际得到 `banner unavailable` 错误；随后健康准备修复的 3 项测试通过，含 A-01 健康失败后继续 A-10。
- 第二 RED：I-02 返回未确认时，实际 runner 仍给出 `Ok(())`，`expect_err` 断言失败。不是编译失败或猜测结果。
- 冻结 RFC 输入校验通过，未改八份原输入。

## 最终源码验证

| 命令/证据 | 结果与边界 |
| --- | --- |
| `cargo test --offline --bin monitor manual_push::tests::` | 修复后重新运行，10 passed，738 filtered；覆盖两个原缺陷、两个盘后窗口、部分/多项失败、HoldingPlan 原明细、P-01 拒绝与旧 banner 不复用；不是完整 monitor suite |
| `cargo test --offline --lib opportunity::scheduler::tests::` | 7 passed，3366 filtered；原有默认/自定义时间合同保持。接口修正不改该源码，沿用有效证据，不声称再次运行 |
| `cargo clippy --offline --bin monitor --message-format=json` | exit 0；190 warning、0 error，与改前警告签名及计数相同，无新增/移除；monitor artifact fresh=false |
| `cargo fmt --all -- --check` | exit 1；6 个未改文件共 20 处既有格式差异，逐名与 BASE 比较无改动；不是全仓格式通过 |
| 两修改文件 scoped rustfmt、`git diff --check` | 均 exit 0；main 同文件一处 overlay_net_yi 为机械换行，无业务改动 |
| `check-rfc-inputs.rb --root .` | exit 0，`rfc_inputs_valid`，仅原始输入一致性 |
| 验证前后 SHA | 两源码摘要完全相同；没有验证中改写源码或为提交重复测试 |

最终 SHA-256：

```text
main.rs         3e3623363f19111cebb0dc8795851cb8d49f1d3a3b2f58af99a170f63cc1ae03
manual_push.rs  d4613908df2f24ed241d5d528cb20032be96b786877554afd3cc587c94a1d7e2
```

过程日志保存在本任务 `.superpowers/sdd/2026-09-10-manual-push-bootstrap-outcome/`：`health-red`/`health-green`、`batch-red`、初版 `final-*`、`scheduler-regression`；修正后为 `fix1-runner`、`fix1-rustfmt`、`fix1-diff-check`、`fix1-clippy`、`fix1-diagnostics`、`fix1-before-sha`/`fix1-after-sha`，均保留命令、原输出及真实退出码。Rust 测试的 lib 非 test 目标保留 113 条旧 warning，不能与 Clippy 190 条口径混合。一次 fmt 未结束时后续两个命令曾被队列锁以 75 拒绝，未实际执行；确认 fmt 结束后才运行，没有并发 Cargo。

## 独立审查与接口修正

初审 Spec compliant、Quality Needs fixes，发现 1 项 Important：A-01 的测试 seam 传 banner，而真实 adapter 丢弃参数。修复仅将 A-01 seam、真实与内存 adapter 对齐为 date-only；测试验证 refresh/read 必须先成功及允许调用顺序，不再声称 banner 作为参数进入 A-01。原 dispatcher 未改，不能把这组测试当作真实治理 gate/sink 的验收。修复后的 10 项测试与静态检查重新通过；限定复审判定原 Important 为 ADDRESSED，修复差异新增 Critical/Important/Minor 均为 0。既有 warning/全仓格式差异为一项已记录的非阻断 Minor。

审查记录为本任务 workspace 的 `task-1-review.md` 和 `task-1-fix-1-review.md`。原审查的两项跨范围说明已处理：本中文实施记录、README、剩余清单和历史审计状态同步完成；真实初始化、生产投递及完整迁移明确不作为已完成验收项。5 份文档的 122 个本地引用和 41 个数字行号范围检查通过，未将该检查说成标题 anchor 或历史语义验证。

## 不在本项内完成的部分

- [时间窗口合同](../../src/opportunity/scheduler.rs#L88)原样保留：`push_window` 对 Intraday 使用 `NaiveTime` 全等匹配，真实时钟的纳秒可使其落入 Outside。因此不能宣称实际盘中时窗已整体修通，也没有擅自放宽到整分钟/全天盘中。
- HoldingPlan 的同批持仓/行情证据、日表与 durable 完成权、有效修订的再次发送资格仍未接通。既有 RFC 已否定“可读日级 occurrence 等于每日唯一”，但不授权 timestamp-only 变化自动重发；[裁决补核](holding-plan-call-chain-2026-09-10.md#既有频次与恢复裁决补核)保留依据。
- 不修改 HoldingPlan 来源/建议阈值/冷却/日预算/恢复，不改变其他 CLI 模式；A-01/A-10 原 bool 的未确认与无数据含义保持，不把 false 换成送达。
- [真实账户成交同步水位](../../src/bin/monitor/main.rs#L2252)仍未接通，源可用性与生产发送另需验收。52 Unit 完整迁移、受保护来源/owner 切换、六门禁、远端 CI 和上线观察仍不能由本任务替代。
