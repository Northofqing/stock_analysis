# 2026-09-29 monitor 分批上线候选

状态：release 已构建并完成隔离验收；**尚未写入新 activation、尚未重启生产 monitor**。本批只接管已提交的本机源码改动，不宣称 M1–M7 完成。

## 版本与输入

| 项 | 当前生产 / 回退 | 候选 |
| --- | --- | --- |
| 本仓源码 | 生产运行根旧 `src/` 已归档于 `/private/tmp/stock-analysis-rollout-20260929-src.tar`，SHA-256 `59172dea34c2e5f302139232003003d8388ed746df7072e836f7c7a973c93ec9` | `master@d7835cee`；`rsync -anic --delete src/ <运行根>/src/` 无内容差异 |
| monitor | SHA-256 `a2589f0115d6f3ee89bf11d7714c7bd4bce58f6d3e5bcbc12b06831ffc5b8b34`，备份 `/private/tmp/stock-analysis-rollout-20260929-monitor` | 从生产根 `cargo build --locked --offline --release -j 2` 构建，SHA-256 `88c1ff0136f277fbdb2a963b3ddad04fcb478ec53cc7eba8053db6cda7ffebcb` |
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

## 待执行的单实例切换

`selection_activation_prepare print-activation` 明确要求**人工 review 后**才写入 `config/selection/selection_activation.v1.json`。当前候选的 `reviewed_by=codex-platform-production-20260929` 仅用于预览，不能代替人审。完成复核后应重新生成实际 reviewer、未来 `effective_from` 的文件，核对新 hash 与字节，再同步到仓库和生产运行根。切换前再次核对 launchd PID、数据库/投递锁与 Uncertain 水位；78 条既有 Uncertain 保持隔离，不能因本次重启自动裁定。按单实例顺序重启 monitor，桥接保持原 PID；等待 DB 初始化及 gRPC 重连。新 PID、binary hash、activation、实际公告批次、来源健康和投递/账本状态必须逐项验收。

如启动失败，先停止新实例，保持同一运行根和数据库；只回退二进制与匹配源码/activation，并核对已发生的写入和 Uncertain，不能覆盖生产 DB 或盲重发。VM 身份如变化，须与本地 bundle 同步切换，不做单侧回退。
