# 会话交接 — 2026-09-19（推送可靠性 / 52 Unit 迁移）

## 0. 起点（先读这四条）

1. **工作目录是 worktree**，不是主检出：
   `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`
   分支 `codex/push-reliability-20260905`。**主检出 `master` 不含这 337+ 个提交**（`intent_store/` 在主检出里是 0 个文件）——别在错误基线上开发。
2. **有一条生产 monitor 在跑**（PID 763，从**主检出**的 `target/monitor-deploy-20260914-scan.*/release/` 起的 2026-09-14 制品）+ gRPC server。**不要碰**；本工作树的代码一行都没上线。
3. **"重构完成"= 52 Unit 逐个接进 counted 持久投递**（WBS 估算 828.99h）。本次会话交付的是 5 个独立缺陷修复 + 接线配方，**迁移认证仍是 0/52**。
4. **接线配方已固化**：`docs/push-system/unit-wiring-recipe-2026-09-19.md`。下一会话接 Unit 直接照它走。

## 1. 本次会话交付（8 提交，工作树干净）

| commit | 内容 |
|---|---|
| `259b9b8` | docs: Unit 接线配方与陷阱（7 触点 / policy 行阻断项 / 预算决策 / 幂等陷阱） |
| `99f5198` | fix(performance): 归因窗口标签改用实际天数（epoch 首月错标 30 天） |
| `8de757f` | test(review): R-13 复审覆盖缺口补齐 |
| `12b6be7` | fix(review): R-13 前排按身份选取（位置前缀致结论错位） |
| `176bdec` | fix(monitor): g5b 退避可测试化 + 真行为测试（替代无效守卫） |
| `8ea01a8` | fix(monitor): g5b 无 provider 分支不再绕过循环尾部 sleep |
| `139b12c` | fix(schedule): 定时输入越界优雅拒绝（`99:99` panic / 非法 weekday 死循环） |
| `f7d44c1` | fix(scheduler): 盘中窗口按秒判定（纳秒全等致三个手工窗口不可达） |

**另有两件非代码成果**：
- **仓库安全化**：97 项未提交 → 0（其中 `intent_store/` 的 **80 个文件此前 git 跟踪数为 0**）；23 份被 `.gitignore` 静默排除的文档纳管；删 2 个垃圾文件。
- **一次危险实现整体回退**：`MU-paper-sell` 接线会让**全部卖出通知静默消失**（见配方 §2 阻断项 A），已撤回并把正确设计记入 `mu-paper-sell-design-direction-2026-09-19.md`。

## 2. 在飞 / 未完成

| 项 | 状态 |
|---|---|
| `reconstruct_epoch_daily` 的 30 天窗口退化 | **已定性，未修**。工具只有"前一交易日投影 + 目标日 fills"，30 天范围被 BR-255 range gate 有意挡掉 ⇒ **乙（真取 30 天）≡ 改 BR-255 核心**；只剩甲（如实标真实跨度）。用户已表示此工具**不重要**，暂搁置 |
| `MU-cli-single` 定时分析丢 macro context | 前提**部分确证**（`schedule.rs:234` 丢弃 `_macro_ctx`；`modes.rs:53/59` 手工路径有）。`_limit_up` 那一半可能是有意（注释称 pipeline 内部自取）。**未修** |
| 52 Unit 接线 | 0/52。配方在手 |
| gRPC 双协议 S1–S6 | **未授权**（需用户明确授权） |
| 生产闸 / 上线 | `EffectBroker::production()` 恒 `ProductionRefused`；需发布决策 + 制品 + 回滚 + 启动对账 |

## 3. 下一步（按优先级）

1. **接一个 Unit**：照 `unit-wiring-recipe-2026-09-19.md` 的 7 触点走。**先做 §3 的两个决策**（是否计入 30 条/日预算；occurrence identity），**绝不跳过 policy 行**（§2 阻断项 A）。
2. 或修 `MU-cli-single` 的 macro context 半边（需先核实 `_limit_up` 那半是否真为有意）。
3. 每片交付纪律见配方 §7。

## 4. 环境坑（本次实测踩到，节省你半小时）

| 坑 | 事实 |
|---|---|
| `timeout` 命令 | macOS **没有**。用 `perl -e 'alarm N; exec @ARGV' …` |
| `--schedule` 属于 | **`stock_analysis` 主二进制**，不是 `monitor`（后者报 `unrecognized flag`） |
| `tests/` 目录 | 在 `.gitignore` 里；**新增测试文件必须 `git add -f`**，否则静默不进提交 |
| `docs/push-system/` | `.gitignore` **白名单模式**（整目录忽略 + 逐条 `!` 放行）；新文档要加 `!` 或 `-f` |
| `monitor --test --push-dry-run` | 跑得通（约 3 分钟），但它**绕过 live 验收门**；`BR196_LIVE_FEISHU_ACCEPTANCE=1` 那条会**真实外发飞书** |
| 编译耗时 | `--bin monitor` 全量约 50s；`--bin stock_analysis` 约 60s。跑二进制前先 `cargo build`，别让 alarm 杀掉编译 |

## 5. 纪律（本次会话用学费换的，照做）

1. **前提核实先于动手**——本次**6 次自述被独立复审推翻**，其中两次是"我以为的缺陷根本不存在"（盘前探针不可达 = 09:15-09:20 有真实窗口；Macro 超时静默 = 契约允许且原因已记录）。
2. **先确认"能否拿到行为 RED"再决定做不做**——有些候选（如依赖全局 DB 的）在仓库测试隔离纪律下拿不到 RED，做到一半才发现就白花。
3. **`grep` 零命中 ≠ 不存在**——`compiled_policy_catalog` 用 `use PushKind::*` 写裸变体名，我的 `grep "PushKind::"` 曾据此差点推翻一个**正确**的复审结论。
4. **独立复审不是仪式**——它证伪过我用来"自证"的守侧测试（该测试被我自己注释里的 "sleep" 字样骗过）；照抄 `§7` 的交付顺序。
5. **不要在未核实的情况下改纪元/持久化语义**——那类改动（如 §2 的乙）会牵动 BR-255 的边界规则。

## 6. 证据位置

- 各片 RED/GREEN/回归日志：`.superpowers/sdd/2026-09-14-chain-macro-recovery/*.log`
- 复审报告：由本会话的评审子代理产出（未落盘成文件；结论要点已并入各提交信息与本文）
- 缺口全清单（177 项，含严重度与复核后位置）：本会话产出的提取报告（`tool-results/` 下），另见 `docs/push-system/README.md` 与 `remaining-migration-evidence-2026-09-08.md`

## 7. 边界

- 本文不含源码改动；未部署、未改生产闸、未启停 monitor、未写生产库。
- 位于 `docs/push-system/`（白名单模式），已加 `!` 规则纳管。
