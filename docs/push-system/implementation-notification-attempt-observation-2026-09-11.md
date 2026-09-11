# 真实通知入口逐目标弱结果：实施与验证

日期：2026-09-11。初版源码：`bd143fe7812875b76215c95fca79d3dbd0c07b4f`，最终修复：`de3887624e97fcacd43bf089a36cc9a2f98d2aea`，BASE=`6ddbf51`。状态：本Task完成；初审唯一Important已修复并经限定复审Approved，无开放的重要问题。本任务不等于盘后误封日已修复，也不关闭完整Unit迁移。

## 实际交付

[真实NotificationService](../../src/notification/service.rs)新增send_report；旧send现在只调用该入口一次，再按“任一弱成功”投影原Result<bool>。不是另造未被旧调用使用的模拟发送器。

[只读报告](../../src/notification/send_report.rs)逐目标保留channel、本次调用内从零开始的序号、已有WeakOutcomeKind。Custom每个URL配置项各保留一次调用，重复URL不去重；列表为空不伪造尝试。渠道方法true→Accepted，false/Err→Unknown，不提前退出后续渠道，也不新增重试。

这里的Accepted只是旧渠道方法报告的弱成功，不是TransportAccepted，不证明完整原报告已送达、用户已读、必达集合完成或重启幂等。序号不是跨配置/跨重启的目标身份。报告和Debug不存正文、URL、token、SMTP凭证或原始错误。旧渠道日志不是本任务的整体脱敏交付。

旧send_with_image、文件保存、底层微信/飞书/邮件实现、配置检测以及W02强/弱完成策略均保持原源码；主控逐字核对了相关方法/文件，不把这种保留证据称为新的SMTP/图片运行验收。

## 实际验证

所有配置和内容均为显式合成；HTTP只访问自建随机127.0.0.1端口，client关闭代理。没有读取真实.env/生产数据库、启动或观察生产monitor，没有调用真实渠道、模型、PAM或交易。

| 检查 | 结果与限制 |
| --- | --- |
| 原协议基线 | 5 passed / 0 failed / 0 ignored；编译3m56s、运行0.00s |
| 首个接口RED | exit101，唯一E0599缺少send_report；是接口RED，不是生产误封日复现 |
| 相同过滤器首个GREEN | 1 passed / 0 failed / 0 ignored；编译3m38s、运行0.00s |
| 首轮16项合批 | 6 passed / 10 failed；10项均在绑定loopback时被沙箱EPERM拦截，未进入业务断言；未忽略、删测试或削弱断言 |
| 获批同范围重跑 | 16 passed / 0 failed / 0 ignored，3380 filtered；编译2.03s、运行3.20s；源码前后摘要一致 |
| 初版Clippy | exit0，1m36s；188条warning/0 error；对同配置基线诊断多集比较新增0、移除0，不是零告警项目 |
| 修复后同组回归 | 实际session32960 exit0，16 passed / 0 failed / 0 ignored，3380 filtered；编译3m10s、运行3.29s；13个源码输入前后摘要一致 |
| 修复后Clippy | 实际session96403 exit0，1m16s；基线188、最终188，新增0、移除0；13个源码输入前后摘要一致 |
| 格式与差异 | 5个变动Rust文件定向rustfmt --check、暂存/非暂存diff --check均exit0；不代表全仓fmt通过 |

最终行为命令：

```bash
cargo test --offline --lib -- notification::send_report_tests:: notification::service::tests::br111_ --test-threads=1
```

其中11项新测试走真实发送入口，5项为原纯协议校验。覆盖Custom部分成功/全部未知/断连/重复地址/空配置、六种本地可配置渠道的顺序与协议成功、微信两片后false/解析Err、飞书回退/多片边界/普通长报告、旧bool真值矩阵及报告Debug不泄露合成敏感标记。测试保留43条既有warning；测试数不计算迁移完成率。

fixture完整读取Content-Length，限制请求长度1MiB，同一8秒期限用于accept/read/write检查；有实际线程handle和停止/join逻辑。Telegram/邮件/Pushover/ServerChan的固定远端或SMTP未执行真实集成；对应穷尽路由保留，不能说十类渠道均取得真实远端接收证明。

## 审查、修复与证据边界

初审：规格Issues found / 质量Needs fixes，0 Critical、1 Important、1 Minor。Important是新增观察路径把Unknown记成“失败”，与已接受前片但后片结果未知的事实冲突。原实施者在de38876仅改一文件10+/6-：false/Err日志明确“投递状态未知”，汇总改为“弱成功／状态未知”，计数变量对应更名；不改协议、正文、路由或重试。

修后16项测试验证原发送行为和结果投影没有回归，并不直接断言logger文案；该日志语义由bd143fe..de38876的精确差异独立复审确认。限定复审结论Approved，唯一Important已处理，fix未引入新问题。既有43条测试warning、188条Clippy诊断保留到整体审查，不扩大本次改动。

初审CannotVerify是执行历史/命令授权不可能由源码diff独自证明。主控以本任务ledger、原实施者所有权记录、实际Cargo会话及loopback审批记录补核；主控独占Cargo/Git/公开文档，没有生产运行验收。修后两组捕获的13个源码输入及原始日志SHA均已重新匹配当前de38876字节。原始记录保留于本计划私有证据目录的fix1-tests.log/json、fix1-clippy.log/json和task-1-fix1-review.md；不把历史running标签当活跃进程，两会话均已终态。

## 飞书分片：已纠正的分析前提

最初根据formatter会转换标题/分隔符，曾推断公开入口完全不能产生多片。随后用末尾空标题的边界输入推翻了这个推断：[真实测试](../../src/notification/send_report_tests.rs)通过send_report得到两片，首片成功、后片card及text回退失败，共3个HTTP请求，最终保留一个Unknown目标。

另一个真实测试确认普通长报告仍会走单片截断，尾部合成标记未进入请求；本任务不修改该旧算法。因此“普通长报告完整性问题”保留到后续，不再写成“任何输入均不能多片”，也不以内部chunked方法名当运行证明。

## 本轮裁决与代价

- 沿用已验证隔离树，只跑已审计定向基线；代价是不提供全仓健康证明。
- 先把真实弱结果保留下来，再接盘后持久流程；代价是本Task结束仍不能宣称误封日已修复。后续任务不能缩成内存缓存。
- false/Err保守归Unknown；代价是某些实际明确拒绝也不能仅凭本观察自动重试，需补强证据/授权。
- 一名Rust写入者、主控单Cargo队列与固定差异独立审查；整体结束才做整分支收尾，当前不删除证据工作目录。
- 飞书先修正普通报告的测试前提，再由空标题公开边界反例纠正过度推断；不为满足测试改发送算法，完整原文分片仍待解决。

## 后续不变

[实施计划](../superpowers/plans/2026-09-11-notification-attempt-observation.md)与[盘后持久恢复设计](chain-post-close-recovery-design-2026-09-11.md)保留下一纵向工作：固定原采集/模型/报告、同业务库准备与保存进度、网络前持久开始记录、逐目标弱观察与重启默认不盲发，再接真实盘后timer/独立身份/强完成cursor。

没有借用R03或Magiclaw强回执，没有开放生产身份、创建生产注册、迁移schema、切换owner或部署。实际必达渠道已异步询问，未确认前不改变旧发送路由和任一成功规则；这不阻止独立代码工程继续。
