# Goal-first 使用闭环收尾（2026-10-09）

## 当前范围

在用户“开发吧”以及既有直接发布授权下继续目标优先开发。本次交付独立工具包，不修改既有交易monitor/bridge运行输入、activation、持仓导入或PaperLedger期初。代码在 `codex/goal-first-delivery-20261009`，起点 `5d28374beb4c5f9d77d92310b33a0d028726ea2c`。

## 已实现

- 新模拟账户到周报：精确私有binding，原数据库path/device/inode对照不可变导入回执，同一只读SQLite backup/时间/原registry生成JSON与Markdown。新当前账户与旧期初前历史独立；缺/错绑定、复制原库、缺证明、未来截面不回退旧账本。
- 周报后自动生成一次Phase A三臂JSON/Markdown和状态收据。共用同一账户/字节/时点，默认零模型调用；错误保留周报与部分产物，不重复模型工作。账户观察是共同上下文，非历史策略结果或可执行报价。
- 盘后producer：准备/消费时间窗、读取预算、账户30秒时效、过期粘性、opaque candidate及durable admission复查。公开原始观察始终无发送资格；真实来源未齐时NotReady。没有重用paper lots或T-21确认卡冒充真实卖出建议。
- 可选独立手机适配器：本地状态先持久化，有限超时，明确受理/Unknown/退避，跨重启与事故世代去重。可变凭证路径在 `runtime/data/private_config/`，不破坏交易config activation。模板保持disabled，不复制到启用路径。
- SQLite微基准：真正public预测到期页读取（8192行、页256）与行情日线批量写入（每轮独立DB、64行）；构造、DDL、池/WAL初始化及清理不计入测量。

## 复审与验证

独立复审发现并已修两处问题：手机配置若落config会破坏activation身份；旧事故的retry事件会在同类新事故时复活。已移私有路径并匹配generation，相关Python29项通过。周报/assistant脚本24项、打包8项通过。Rust和正式制品只读验收正在执行，正式结果在发布完成后补本节，不提前宣称通过。

构建基线在冻结提交5d28374、独立fresh target、monitor dev、incremental=0、jobs4上完成：cold563.86秒、unchanged2.09秒、单文件注释修改43.17秒；target4798712362bytes。Cargo复用仍有效；尚不能据此声称开启incremental更快。详证在私有 `stock-analysis-candidates/closeout-performance-20261009-nonincremental/report.json`。

## 外部待验收

- 合格实时行情，Windows资源耗尽修复及市场时段业务探针。日资金流不能代替实时MoneyFlow，普通订单簿不能代替strictT0。
- 真实可卖批次/预留/费用、15:00证券状态及数量来源合同；独立真人提醒的counted语义/owner接线、2分钟实测及正式发送。
- 手机通道/凭证及实际手机显示；HTTP成功只证明服务受理。
- 真实family/version/known-by/PIT历史，模型价格与账单合同、三臂人工评分和≥20完成交易日效果；模板生成不证明模型有效。
- 完整真实日事件capture/count receipt join后的回放。四条Rolling人工请求与两笔争议金额仍保留，无自动裁定。

E5 facade/workspace与平台工程仍按新方案冻结；真实券商执行不在本次范围。
