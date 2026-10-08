# 收敛范围后的首轮开发（2026-10-08）

执行依据：[当前范围裁定](2026-10-07-active-scope.md)。本轮仅修改现有 paper 扫描及 H08 历史观测入口，并形成周报基线；没有恢复冻结的平台工程。

## 本轮改动

- `paper_sell::intraday_session_open_at`：将 UTC 时刻转为上海时间，只在已核验交易日的连续交易时段放行；拒绝节假日、周末、未知日历年份、午休及 11:30/15:00 边界后的扫描。
- `IntradayMonitor::tick`：与卖出路径共用上述判断，休市时在读取账户 binding、数据库和行情前返回零处理数。开市时仍执行既有资格、账本、价格带、T+1、费用、幂等和风险规则。显式盘后扫描保持原入口。
- `historical_observed_probe --candidate-db`：复用既有只读快照会话，按指定证券原推送及尚待验证预测的最早日期生成 from，仍要求显式 to；与手填 `--from` 互斥。不存在的源库不会被创建。
- 原 `outcome_data` 分页查询复用同一实现，并增加可选 exact-code 过滤；既有 latest-N 采集路径仍保持其原合同，不能据新入口声称它已经变成显式区间回填。
- [首份周报基线](../ops/2026-10-08-weekly-outcome-review.md)：记录原库缺口、待结果、未成交/退出/费用证据与下一周动作，不据不完整记录计算有效命中率或自动调整策略。

历史探针仍是单证券、已完成起止日、ObservedOnly / NotAdmitted / Unknown coverage / NotCertified PIT，观测原件进入独立私有目录。新候选日期接线不写生产日线、交易状态、结果、receipt 或订单。

## 验证记录

节假日回归先执行原逻辑，实际 1 项失败：2026-10-02 10:00 CST 被当作交易时段。原日志保留为 `paper-session-red.log`，失败原因是断言，不是构建失败。

最终验证为 41 个不同方法通过，0 ignored：卖出模块单独进程 12 项；买入监控与 outcome 模块同进程 26 项；`historical_observed_probe` CLI 3 项。验证覆盖扫描边界、既有执行/恢复、候选分页、参数互斥和只读源库不变/缺文件不创建；没有将 debug 测试认作生产制品、合格数据或真实投递验收。

首次把三个模块合跑为 30 通过、8 失败：买入夹具向进程共享测试库 Seed，清理器仅清除候选、成交及旧 ledger，不清除 seed/head；随后旧库存测试要求 legacy unbound scope，触发 `seeded database requires explicit bound economic scope`。按不同进程重跑上述两组后全部通过，复用同一未改源码/harness；没有为通过测试放宽生产经济范围或清除生产 seed。该既有共享夹具限制仍在，不能宣称整库同进程测试全绿。

库原件 SHA-256：`paper-isolated.log` 为 `6d2bef17ceb50663f582304719ea2c261f5d10447d56c497198e456f0eef7731`；`buy-outcome-isolated.log` 为 `7499eb98184b79c8995f77bbeb0d0c735d515361442c3a8b4c80bb02d19ea637`。首失败日志及节假日 RED 一并保留。4 个改动 Rust 文件与 Cargo.toml/lock/build.rs 共 7 件相关输入在验证间未变；这不是完整 release 输入证明。

CLI 原件 `probe-final.log` SHA-256 为 `a3dd0e9aed8d7354a0860e219f798b3e625f903961bdbab4cbedaebea13087f4`。`targeted-validation.json` 保存上述日志、7 件相关输入及两个 harness 的哈希。未跑全量测试、release、生产投递或同版真实 RPC；既存库测试 152 项/普通库 886 项 warning 保留，未追加 clippy 或放宽校验。

本地原件位于开发工作树 `.planning/2026-10-08-active-scope/`，不推送原始日志或私有数据库。tracked 交接只记录摘要及其身份。

## 接续与实际边界

1. Windows 原 chat 已收到本轮入口接线和 Shenzhen 000001、2026-08-21..=2026-09-30 的具体验收要求；复用 1076 证券输入，继续交付真实逐日状态、生命周期、价格资格及修订来源。原 critical 95% 门槛保留；任务消息和内部上下文汇总不算 RPC 结果。
2. outcome 三项前序修复已在 master；本轮沿 `codex/platform-roadmap-implementation-20261002` 提交并推送，保留开发分支，未合入 master。本轮源码提交/推送与生产安装分别核验。新数据合格前不将原 76 行 T+1/3/5 填成有效收益。
3. 2026-10-08 01:06:58 CST 核对正式 monitor PID 3643，monitor SHA 仍为 `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，bridge 文件 SHA 为 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`。源码定向验证未触发重启；新候选的具体 release、兼容回退及 activation 材料另按当前上线要求准备。
4. paper 缺 binding 或独立同日价格资格仍保持拒绝；本轮仅去掉休市时无意义的执行尝试，没有制造资金、持仓或成交。
5. 本周无新成交不能验证止损退出效果；情报稳定性仍需实际接纳、失败及恢复证据，日志恢复次数不等于送达次数。

为下一次窄观测准备了 `original-000001-candidates.db`：只读事务复制生产原表结构及该证券的 2 条原推送，0 条预测，16 KiB；SHA-256 `6dac265b5b0eec58fd155ff74921532e9709487bad992afe2a42d5693c819412`。SQL、原列、行内容哈希和观察时刻存于同目录 manifest。它只是有界原行诊断副本，不是完整生产备份或合格行情。私有输出目录 `historical-observed/` 已准备；Mac 实际 SDK/同版 RPC 尚未验收，不把准备材料算成回填完成。

## 晨间接续

08:38 CST Mac 通过既有隧道完成旧 4e 合同的四项原始只读 RPC，回传 Windows 示例/manifest 勘误；结果、旧报价时效和旧窄候选 SDK 元组不匹配详见 [晨间接续](../ops/2026-10-08-active-scope-readiness.md)。本轮核对 7 件相关输入与原日志哈希未变，沿已有授权将本首轮源码合入 master，实际本地/远端 OID 以新发布 receipt 为准。前述“未合入 master”是首轮提交时的历史状态；没有生产安装或合格历史回填。
