# 新闻组合修复部署（2026-10-05）

已部署到 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，完成启动、版本和新闻采集验收；实际新闻投递仍待后续合格窗口。源码提交 `d9b4aabb7445c01148c2306d848b178866053768`。

## 部署范围

在生产基线 `1f0fc6a7` 的独立热修 `365ca5ba` 上，仅迁入 `63ae676d` 的 Main 刷新差量。相对旧生产735项输入，只改连接缓存恢复、人工审核通知词汇及测试夹具三个原热修文件，再加 Main 概念索引后台刷新；未迁入 DEV 平台或诊断改动。新闻完整批次、代码集合一致安装、失败保旧、单 pending worker 及300秒刷新条件保持。原桥接二进制和实例保持，数据库、凭据和历史投递记录未替换或重放。

旧热修63项定向验证复用；组合版本新增 monitor2项回归退出0，编译43.83秒。release monitor退出0，构建2分42秒；同制品 info dry-run退出0，61家族、失败0、外部进程尝试0、投递回执追加0。首次继承WARN日志的dry-run也退出0，但未显示验收摘要，所以保留该记录并显式开启INFO完成上述有限确认。独立 Source `8a408a83` 通过，测试仅覆盖刷新 helper 的 pending及一次消费，不等于真实跨窗口发送已验证。

## 实际制品与启动

新 monitor SHA256 `49518b30d35570bbe300f7c7adccd8639105d9d169e3bbefe02b9fe142dcdd20`，编译绑定真实生产根。最终 activation expected_config_hash `eab56ef23b876c4b2049b08e4340aa85da00ab3377f162389cbe2aa31d5f0b1f`，SHA256 `96db29bff923dae6bf0424e3cf9a6d2047f707180cac27e4672370540d473e5e`，20:56:48 CST生效。reviewed_by记录 Codex根据本聊天用户“部署 继续开发”的授权操作，不冒称旧14:00候选已获精确批准。

最终受控重启的新唯一PID81292，桥接仍PID56417；cwd、打开的主库和持久投递库均为原生产根。20:57:27开始，启用分支实际注册4个新闻feed；20:59:31主库初始化完成，20:59:33桥接连接，20:59:40四源首次接纳 Jin10=19、Eastmoney=20、CLS=20、ThePaper=12，共71条。实际735输入、5公共编译输入、当前二进制与activation均与候选核对。

健康CLI退出1：进程和心跳新鲜，但账户Frozen、数据Unsafe、摘要不完整和缺失能力仍存在；该结果不升级为全面健康、真实券商或完整Financial资格。当前夜间已经超过9:30/11:30/13:00/15:00的300秒汇总窗口，未强制补发测试消息或裁定Uncertain，未宣称NewsFlash/NewsAI已新送达。

## 保留的部署问题与回退

首次部署前的完整stat比较在停止服务前退出；该比较把读取访问时间也纳入稳定性要求，后续诊断没有定位到具体访问时间变化项。检查修正为设备、inode、类型、owner、nlink、size、mtime、ctime等实际身份字段，并核对完整内容哈希。首次切换的activation又使用了准备工具从旧生产输入算出的17a3哈希，启动门正确拒绝。准备工具的生产根是编译时绑定，候选目录作为cwd不会改变它；必须在生产输入安装后从实际生产根重算，或使用明确绑定候选根的准备工具并核对。该次旧候选与拒绝日志保留，未升级通过。

安装精确735项候选后，同工具实际重算出eab56，重新生成未来activation，替换后在生效时刻之后受控重启一次。最终4feed启用分支与后续采集事实支持门已通过；未绕过任何门禁。回退仍使用同一运行根和现有数据库，只还原六项已备份源文件/activation/二进制，不覆盖数据。

完整候选、两个失败检查点、更正记录、启动快照及回退材料位于 `/Users/zhangzhen/.local/share/stock-analysis-news-rollout-20261005/`；生产验收副本位于运行根 `ops/news-rollout-20261005/`。原 `/Users/zhangzhen/.local/share/stock-analysis-news-hotfix-20261005/` 冻结包保持原样。

非作者独立部署 DATA 复核通过，Spec/Standards C0/I0/M0；报告 `validation/independent-deployment-data-review.md` SHA256 `8ea64d24bbbf0a54475972d39fda82590dd70ebb89c16aa08fb43a2952561090`。报告独立核对冻结候选、实际生产输入、制品、activation与闭合启动采集日志；健康拒绝、首轮错误激活和投递窗口未验收的边界原样保留。
