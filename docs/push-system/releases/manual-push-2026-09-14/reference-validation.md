# 首批手动推送：参考验证与切换缺口

更新日期：2026-09-14（北京时间）。结论：两文件首包已通过干净参考树的17项定向测试和非测试可执行文件构建；未部署、未启动monitor，也未完成实际切换验收。首批不等待盘后完整恢复或全部52个Unit，但仍须满足本批自身的安全切换条件。

## 精确范围与身份

- 源差异：43f48933de599acd9c8207d72d6610ded2a08ba4→41e7762301498302266b8780866e9fa47c5315a5，仅main.rs与manual_push.rs，补丁34007字节，SHA443625e605903d86b737d3b7eefdd2da3265b2160764b115baf3f817977f7954。
- 实际参考基线：a673043acb9390605d2a43fc3ee2ad01488f633e；工作树/private/tmp/manual-push-reference-build.prqW42，仅应用该两文件差异，没有盘后v10、新Foundation DDL或后续竞价/持仓来源改动。
- 候选main SHA af706bcccf577a1b16e566de19e7e3500eaf172ed226556d875ac4e853e59a0f；manual SHA d4613908df2f24ed241d5d528cb20032be96b786877554afd3cc587c94a1d7e2。历史最终main的整文件SHA与参考main不同，不能混用；[manifest](manifest.json)分别保存。
- Cargo.toml、Cargo.lock、build.rs保持参考版本。Git忽略但构建必需的client-bundle/market.proto从本地既有合同原字节补入，原根/开发根/参考根SHA相同：8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332。
- 原release的源码身份未认证；原目录HEAD及二进制修改时间不能证明构建输入。原目录160个未合并路径及全部用户修改保留。

## 实际验证终态

命令工作目录均为上述参考根；命令级--target-dir指向隔离开发树target，不指向原checkout的target。唯一Cargo队列，验证期间参考源码冻结。

| 捕获 | 实际命令（省略相同target-dir参数） | 终态与范围 |
| --- | --- | --- |
| reference-manual-tests | cargo test --offline --locked --bin monitor manual_push::tests:: -- --test-threads=1 | exit0；10 passed，0 failed，713 filtered；编译2m07s，测试0.00s |
| reference-scheduler-tests | cargo test --offline --locked --lib opportunity::scheduler::tests:: -- --test-threads=1 | exit0；7 passed，0 failed，2840 filtered；编译2m19s，测试0.00s |
| reference-monitor-build | cargo build --offline --locked --bin monitor | exit0；2m10s；标准dev配置，非测试monitor可执行文件；未执行 |

三次捕获各自834项输入的before/after及核查时current摘要一致，日志摘要独立核对通过。lib编译保留84条告警，scheduler的lib-test目标43条；不同目标的计数不是告警增减对比，也不宣称全仓零告警。原两文件差异的独立Spec/Quality与限定复审见[原实施记录](../../implementation-manual-push-bootstrap-outcome-2026-09-10.md)，本次另核参考树14个相关源码摘要及真实adapter依赖；不是重新认证全部未改业务。

日志SHA（完整命令/时间/834项摘要保存在开发树.superpowers/sdd/2026-09-14-first-incremental-release/同名JSON与log）：

```text
reference-manual-tests     c84ca497ae98dffb285e166c825ab0e7a45481d216432002c16d89814ecbeacf
reference-scheduler-tests  563b602cc78e85f8aeb37139dc69ba7eb0b460f2cd10e30e0a5a399286673823
reference-monitor-build   1c890358689382ddc4494dd6e5dbbf5a29dcabee60da9b4822f8093a63f4ddac
```

构建终态UTC：2026-09-13T16:39:49Z。参考二进制97835952字节，Mach-O x86_64，SHA3d4e494ffaf94114418d282e8ee27d326d555a59ac024083cf5c393434d6967f；已固定副本reference-monitor。构建前隔离开发树的旧dev制品已备份为reference-monitor-before-build，SHAc7cd5ad33da53bdff2a373a4fd63ebc0a70f3b467880a47d2e8f131171d2e3da。后者不是原生产制品备份，也不构成生产回滚验收。原checkout的release/debug均未改。

## 这些验证证明什么

真实run_daily_pushes实例化RealManualPushEffects并调用同一run_manual_push；编译验证真实adapter与参考依赖兼容。内存用例覆盖刷新后读banner、健康失败不复用旧banner、盘中原顺序、失败继续后续任务并返回全部失败项、A01健康失败仍运行A10、P01拒绝且无新增效果。CLI现有Err分支经JSONL drain后退出2的接线已静态核对。

测试未执行main初始化、真实CLI退出、账户DB、provider或sink。原scheduler测试只证明原合同未被该补丁改变；它仍以完整NaiveTime判断Intraday，10:30:00.123和10:30:05会落入Outside并走A01/A10，不能把手构Intraday的测试视为真实盘中路由验收。各原dispatcher对Deduped/无数据的返回语义仍不一致；批次退出2不授权整批自动重发。这些保留问题仍在全量目标中，不因为首包测试通过而销项。

## 为什么参考二进制不能直接替换

以下证据行号对应精确参考树，不把当前开发main的整文件身份冒充参考版本：

| 资源 | 参考源码证据 | 临时构建的实际绑定 |
| --- | --- | --- |
| 生产实例锁 | src/bin/monitor/main.rs:3387 | 编译根/data/locks/production/monitor-delivery.lock |
| 核心业务DB | src/bin/monitor/main.rs:3528、3542 | 编译根/data/stock_analysis.db，并覆盖DATABASE_PATH |
| durable DB | src/durable_delivery/coordinator.rs:2267 | 以编译根解析repository-relative路径 |
| 两类生产audit | src/event/dispatcher.rs:287；src/event/durable_delivery_append.rs:216 | 均绑定编译根 |
| activation材料 | src/selection/activation_gate.rs:43 | 编译根内的发布资格材料 |
| event-bus JSONL | src/bin/monitor/main.rs:3332 | 启动CWD下data/event_bus，与上述编译根可能分离 |

env!(CARGO_MANIFEST_DIR)是编译期常量，复制文件或改变启动CWD不会重定位。临时制品若运行，可能访问另一份DB/审计、与旧程序锁不互斥，或在初始化时失败。不能靠.env伪装编译根，也不为首包默认放松身份、任务锁和数据完整性规则。

## 原release追溯结果

只读核查已结束，主控复核了原release、对应deps制品、dep-info和fingerprint摘要。原release与deps/monitor-54f374361d106c1c的SHA均为d27b52f532b86ee393124b3d944949279c7ceedadd5ee29f9ec05461e86e78ed，仍未改动。monitor.d的442个绝对路径全部位于原目录；结合二进制内受限路径提取，本地记录一致指向原目录编译，不能由此证明当时源码内容。

monitor.d SHA为def6bed04b4c55443875da51eada386575f3cc20d7daf670c4fb443637957a5c；对应bin-monitor.json SHA为23db6726b527411305dc6880de59f5797b585a1720fe84275d8677feea48ee97。该JSON的CheckDepInfo明确checksum:false，没有完整源码内容摘要或Git提交；精确构建身份搜索未找到旧release回执。因此旧制品字节可以保留用于后续回滚准备，但不能冒称a673043就是它的构建源。无需引入外部可信身份平台来解决此问题，需要的是明确正式源码基线并保存后续实际构建回执。

独立报告保存在本任务私有目录deployment-baseline-report.md，SHA6f3594dc4a7f442cf95a6859abe1b0f689513810fb583e81502ff0e6143f0be3。报告中的公开文档摘要/行号对应其读取时版本，文档随后已更新，不能作为当前文档身份。主控只复用仍与真实文件相符的制品/构建输入事实。

推荐后续先实现“运行数据目录与构建目录分离”，统一保留原数据库、实例锁和审计目录，源码在独立稳定目录构建；正式源码基线另行核对，不能默选a673043后声称原有业务全部等价。这比原两文件首包增加了启动/路径合同改动，须先确认该范围；当前仅提出建议，未实现、未部署，也未修改原路径或跳过锁。

## 首批仍需完成的切换步骤

1. 确认要集成的完整源码基线及预期稳定数据/锁/审计根；原冲突不自动丢弃。旧制品构建记录追溯已结束但未找到精确源码快照，仍未将a673043声明为正式部署基线。后续明确所选基线与保留业务范围，不能伪造无法恢复的历史证明。
2. 在确定的正式构建布局上应用两文件包并生成最终候选，固定真实制品及依赖身份；布局需要改动运行根政策或原冲突目录时，先明确相应变更范围和授权。临时工程构建不能替代这一项。
3. 保留可恢复的旧正式制品与启动信息，核同一任务只有一个发送执行者，确认在途/Unknown处置；运行回退不删除事实、不降低schema，也不撤销已发生的外发。
4. 取得对应生产操作授权后受控替换、启动并验收实际推送。不能用源码反向检查、进程存在或普通成功日志代替回滚和真实投递证据。

本批工程验证完成不等于正式交付完成；后续竞价/持仓候选及完整Task2–4/W15–W21/52Unit范围继续保留，见[首批计划](../../../superpowers/plans/2026-09-14-first-incremental-release.md)。
