# 10月9日：goal-first 本地交付

**已完成：** 三档周报与人工registry、独立本地watchdog、只读三臂助手、SELL/连板预览、全前缀分页与SLI。包含五个release bin的工具包在Desktop外候选目录，**待安装**。旧monitor、bridge、数据库与日程没有改变。

**怎样运行：** 先打开[固定工具包runbook](../ops/goal-first-delivery-runbook.md)，以默认check检查bundle；离线助手给同一时钟的private `review.json` / manifest / registry，默认0模型calls。SELL/连板给显式observed pack，只产生中文预览，不发送/下单。实际包：`~/.local/share/stock-analysis-candidates/goal-first/v20261009-f3761320c`；可直接运行的保留示例、source身份及容量结果在[证据索引](2026-10-09-goal-first-evidence/INDEX.md)。

**下一必要边界：** 装包与加载job需单独运行验收；SELL缺独立close/真实lots/fee与ATR单位裁定，连板缺PIT/可执行历史，family缺版本归因，手机备用channel与真实model价格/账单未验收。历史money/Unknown处置仍需人类裁定。至少20个completed-session记录省时/遗漏才能说明用户价值；fixtures不证明收益。

性能证据仅debug同fixture：大样本peak RSS下降79.639%；小样本latency上涨31.8705%（约2.855ms）。优化release Criterion、three-way构建、生产启动/WAL压力和真实日级counted replay均未测。新report上限64MiB会增加有限整文件/JSON树内存；input65536/request131072增加reservation，cash ceiling保持原显式值，可能拒绝。

架构已按[实际/staged边界](../Project_Architecture_Blueprint.md)窄更新，HTML draft生成因冻结current-audit file-set mismatch被拒绝（exit1），旧HTML保持历史/provisional；没有绕过guard或扩大冻结平台工具。完整各任务报告、检查/失败历史、独立review/rereview、八项取舍与成本、原v2方案原件SHA，见[证据索引](2026-10-09-goal-first-evidence/INDEX.md)。源码实现、本地产物、已安装观察、来源资格、人类验收分别记录；任何SHA都不升级source authority。
