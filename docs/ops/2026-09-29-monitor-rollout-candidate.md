# 2026-09-29 monitor 分批上线候选

状态：release 已构建并完成隔离验收；**尚未写入新 activation、尚未重启生产 monitor**。01:30 CST 已将生产磁盘上的源码和 monitor 恢复为当前旧进程对应的版本，避免 launchd 意外重启到未激活候选；新候选独立保存待切换。本批只接管已提交的本机源码改动，不宣称 M1–M7 完成。

## 版本与输入

| 项 | 当前生产 / 回退 | 候选 |
| --- | --- | --- |
| 本仓源码 | 生产运行根旧 `src/` 已从 `/private/tmp/stock-analysis-rollout-20260929-src.tar` 恢复；备份 SHA-256 `59172dea34c2e5f302139232003003d8388ed746df7072e836f7c7a973c93ec9`，恢复后逐文件 `rsync -anic --delete` 无差异 | `master@d7835cee` 的候选源码保存于 `/private/tmp/stock-analysis-rollout-20260929-candidate-src-live/`，与当前本仓 `src/` 逐文件无差异；候选 tar SHA-256 `0118b25c648ffab3e846b06a504e970edbb38fabd5472962ac8c870c5cf8a9ff` |
| monitor | 生产磁盘与当前运行进程均为旧 SHA-256 `a2589f0115d6f3ee89bf11d7714c7bd4bce58f6d3e5bcbc12b06831ffc5b8b34`，备份 `/private/tmp/stock-analysis-rollout-20260929-monitor` | 从生产根 `cargo build --locked --offline --release -j 2` 构建，SHA-256 `88c1ff0136f277fbdb2a963b3ddad04fcb478ec53cc7eba8053db6cda7ffebcb`，现保存于 `/private/tmp/stock-analysis-rollout-20260929-candidate-monitor` |
| gRPC bundle | manifest SHA-256 `cc3d97239da5bc224e487eef355b7154901739bafd76d64f78ab6337cad9f89f` | 同一 manifest，9/9 文件校验通过；VM Health 构建身份匹配 |
| activation | 旧 `expected_config_hash=ba4087dbd9d76d377a3804cd56738e91dd40e177dc131c3e5056650be660b158` | `selection_activation_prepare` 计算新 hash `3cff3b27c7b0f457949b1bafa1f281d686f4f6ccfc6cc55542b76ad05555513f`；候选 JSON 在 `/private/tmp/stock-analysis-rollout-20260929-activation.json`，未安装 |

运行根为 `/Users/zhangzhen/.local/share/stock-analysis-runtime`。`config/` 实际内容无差异；新 hash 来自 `src/` 更新。构建前源差异为 16 个文件，涉及公告 ExternalV1 接线、启动阶段计时、T-14/T-15 来源门、预测恢复、费用 v2 研究纯函数、定时分析上下文和新闻错误码。没有修改数据文件、凭据或 VM bundle。桥接二进制仍是原 SHA-256 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`，本批无桥接源码改动。

## 候选验收

- `cargo build --locked --offline --release -j 2 --bin monitor --bin grpc_bundle_probe --bin selection_activation_prepare` 退出 0。
- 新探针 `--opening`：Health `live=true/ready=true`、部署身份 matched、静态路由 9/9 ready，四家 GlobalNews 均 `ADMITTED`。
- 新探针 `--market-announcements-date 2026-09-28 --limit 300`：`ADMITTED/complete/Cninfo/300`，上游 batch 报 `total=744`。
- 新 monitor `RUST_LOG=info --test --push-dry-run` 退出 0：`mode=test`、独立 `TEST_CODE` namespace、59 个 dry-run family，`external_process_attempted=0`、`receipt_audit_appended=0`。旧 activation 使 selection-v2 `activation_not_effective`，符合切换前状态。
- 代码定向验证：公告 7/7、transport 8/8、methods 3/3，费用 v2 2/2、策略 realistic 8/8、push readiness 7/7；本轮定时管线 3/3、新闻 raw_v2 18/18、数据库隔离 1/1。各组结果仅对应相关源码与测试，未做全量测试。

01:04 CST 的只读生产预检：旧 monitor PID `17089` 仍占用同一运行根的主库与 durable 库，桥接 PID `56417` 未变。`monitor --health --json` 报 `monitor_running=true`、快照新鲜，但 `Frozen/Unsafe`、四项能力缺失；该命令只覆盖 banner/account/data，夜间结果不能代替盘中健康。durable `delivery_decisions` 聚合为 `Delivered=841`、`RejectedDurable=3987`、`ManualResolvedRejected=6`、`UncertainManualReview=78`；其中 73 条是 2026-09-24 的 `DataMode`，其余 5 条分属 WatchlistTracking、CloseCall、T0Advice。只读统计未查看外部渠道结果，不授权裁定或重发这些不确定投递。四家 GlobalNews 于 01:03:33 CST 的旧进程日志均有 `available` 样本；PaperLedger 仍报未激活。

当晚 00–01 时日志按 `[DataGateway]` 聚合：四家 GlobalNews 与 SecurityIdentity 各 7 次 `available`，`board-memberships` 有 366 次 `available`；旧进程的 `R-08-announcements` 有 8 次 `invalid_request`，最后一条在 01:10:55 CST。该旧 adapter 缺口已在候选源码修复并由只读真实 RPC 证明，切换后须确认新进程不再产生同一请求错误。

01:30 CST 恢复磁盘启动输入后，`launchctl` 仍显示旧 monitor PID `17089`，未重启；磁盘二进制 SHA 与旧备份一致，生产 `src/` 与旧 tar 解包目录逐文件一致。候选二进制及源码已分别保存在上表的 `/private/tmp` 路径。复核通过后需重新将这组候选精确同步到运行根，再重算/核对 activation hash 并按生效时刻单实例切换；不能直接使用当前旧源码目录生成新 activation。

## 待执行的单实例切换

`selection_activation_prepare print-activation` 明确要求**人工 review 后**才写入 `config/selection/selection_activation.v1.json`。当前候选的 `reviewed_by=codex-platform-production-20260929` 仅用于预览，不能代替人审。完成复核后应重新生成实际 reviewer、未来 `effective_from` 的文件，核对新 hash 与字节，再同步到仓库和生产运行根。切换前再次核对 launchd PID、数据库/投递锁与 Uncertain 水位；78 条既有 Uncertain 保持隔离，不能因本次重启自动裁定。按单实例顺序重启 monitor，桥接保持原 PID；等待 DB 初始化及 gRPC 重连。新 PID、binary hash、activation、实际公告批次、来源健康和投递/账本状态必须逐项验收。

候选源码与生产运行根分别核对封存输入清单时，运行 `python3 scripts/verify_executable_input_manifest.py /path/to/manifest.json /path/to/root --activation-ready --allow-extra config/selection/selection_activation.v1.json`（替换清单和根目录路径），并单独核对 activation 文件的批准 SHA-256。activation 会枚举 `src/` 和 `config/` 中每个普通文件；若有 `.DS_Store` 等清单外文件，先移出输入树并重算 hash，不得用额外的 `--allow-extra` 宣称激活就绪。

`selection::process_bootstrap` 在启动时计算 activation gate；`activation_gate` 在 `now < effective_from` 时返回 `activation_not_effective`。因此必须等新文件的未来生效时刻到达后再重启 monitor，不能先重启、事后仅等时钟越过生效点。等待期间旧 PID 继续提供服务。

18:00 UTC 后，`/private/tmp/stock-analysis-rollout-20260929-activation.json` 中的预览 `effective_from=2026-09-28T18:00:00Z` 已过；该文件的 `reviewed_by` 也是占位值。它只能供比对旧候选哈希，不能直接安装。实际切换需在候选源码和二进制重新放回运行根并核对哈希后，使用真实复核人和新的未来生效时刻生成 activation，再按上述顺序重启。

如启动失败，先停止新实例，保持同一运行根和数据库；只回退二进制与匹配源码/activation，并核对已发生的写入和 Uncertain，不能覆盖生产 DB 或盲重发。VM 身份如变化，须与本地 bundle 同步切换，不做单侧回退。
