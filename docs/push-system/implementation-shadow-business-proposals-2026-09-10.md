# W17 同次完整业务提案比较：实施记录

日期：2026-09-10。BASE：`20f215d56387dc233b8496c5c5d69022363f1030`；初版SOURCE：`b142c9bdbaaafaa18664df049a540084d2c89532`；最终SOURCE：`9b2a5df7033cc490ffe9ed5749073eada3f09653`。状态：初审两项Important已处理，修复后26项定向测试及静态检查通过，独立限定复审无新增Critical/Important/Minor，**本Task关闭，过程例外保留**。仅隔离分支代码，不是完整W17、P-02接线或生产验收完成。

合同见[实施计划](../superpowers/plans/2026-09-10-shadow-business-proposals.md)。初版仅修改三份Rust文件，761行新增、7行删除；修复轮在其中两份文件137行新增、4行删除。没有改P-02选择器/采集/dispatcher、真实main、来源合同、模板、通知集合推进、激活权限或冻结数据。

## 实际增加了什么

旧影子入口只比较其已覆盖的决策/语义/渲染/完成提案，并且只返回报告。新增入口让两侧在**同一次执行**交回实际业务载荷，比较后还能取回旧侧产生的原对象，不需要再次准备或克隆。真实完整载荷必须由后续业务adapter提供，泛型相等不是完整业务认证。

| 接点 | 当前行为与证据 |
| --- | --- |
| [ShadowBusinessObservation](../../src/monitor/push_job/shadow.rs#L295) | 原ShadowObservation与拥有的Option<P>同次返回；P只要求PartialEq，不要求Clone/Copy/Debug |
| [新增执行入口](../../src/monitor/push_job/shadow.rs#L439) | 同context/facts初检通过才各调用一次；一侧失败不跳过另一侧；与[原入口](../../src/monitor/push_job/shadow.rs#L414)共用[私有执行内核](../../src/monitor/push_job/shadow.rs#L532) |
| [存在性检查](../../src/monitor/push_job/shadow.rs#L516) | Ready缺提案或非Ready带提案产生路径明确的InvalidObservation；[修复后的遍历](../../src/monitor/push_job/shadow.rs#L473)对每个成功callback检查，不再因基础观察已无效而漏记新增事实；独立限定复审已确认 |
| [业务差异类型](../../src/monitor/push_job/shadow.rs#L230) | 保留原差异，另记录存在性错误和BusinessProposal内容差异；两个有效提案仅内容不同时，路径仍Completed而整体不Match |
| [完整业务报告](../../src/monitor/push_job/shadow.rs#L319) | statuses、counts、differences、reasons与is_match统一覆盖原证据及新增比较；私有基础ShadowReport不暴露给新调用方单独判成功 |
| [原旧提案取回](../../src/monitor/push_job/shadow.rs#L381) | 有效旧侧提案保留原所有权，可take一次；新侧失败/不同/尝试被拒效果不自动丢弃旧数据；旧侧未执行/失败/观察无效时没有旧提案 |

新报告的Debug只显示状态、计数与固定差异，载荷包装只显示类型和存在性，不调用P的Debug。旧execute_shadow、ShadowReport及三个原公开闭集enum的合同保持不变；新增类型由[公开导出](../../src/monitor/push_job.rs#L62)提供。

**取回旧数据不是发送许可。** 新报告Match也不授予激活、完成或晋级权限；真实dispatcher仍须使用原合法owner及当前fence。P::eq的覆盖范围也必须在实际接线时审查，不能传空载荷、漏字段替代类型或把旧结果克隆两份来证明两个adapter一致。提供的拒绝能力不沙箱化任意全局I/O。

## 初版固定源码验证（b142c9b，不是修复后证据）

`cargo test --lib monitor::push_job::shadow_tests -- --test-threads=1`：**25 passed / 0 failed / 0 ignored / 3338 filtered，真实exit 0**。保留17项旧接口测试，新增8个business_proposal_测试组。主线独立核对了完整stdout中25个唯一成功用例、新组8个、精确汇总和meta退出码；不是全项目测试通过。

- [同实例/各一次/原对象所有权](../../src/monitor/push_job/shadow_tests.rs#L176)：无Clone/Debug载荷，用旧侧堆地址证明原对象移动，第二次取回为空。
- [完整字段差异](../../src/monitor/push_job/shadow_tests.rs#L1118)：消息本身，以及消息相同但价格bits、隐藏指标、记录顺序、通知集合不同，均有固定预期。
- [存在性](../../src/monitor/push_job/shadow_tests.rs#L1157)、[回调失败及旧输出保留](../../src/monitor/push_job/shadow_tests.rs#L1308)、[输入与Ready/NoData绑定](../../src/monitor/push_job/shadow_tests.rs#L1381)：逐路径区分，不用载荷相同掩盖无效观察。
- [八类拒绝能力](../../src/monitor/push_job/shadow_tests.rs#L1489)：每侧逐项请求先计数后拒绝，不因忽略错误而Match，合法旧数据仍保留。
- [合法非Ready](../../src/monitor/push_job/shadow_tests.rs#L1529)、[Debug/错误安全及取回后报告](../../src/monitor/push_job/shadow_tests.rs#L1543)：无发送业务数据及无敏感载荷泄露。

以上链接采用修复后当前定位，初版具体源码身份仍以b142c9b及当时SHA为准；新增组合异常用例不倒计入初版25项。

测试stdout为3090字节，SHA-256 `9410f7b739b652e265b3b768e7896f1e483e31546a7cd4bb8630f52a1bd86fd5`；stderr为9986字节，SHA-256 `122da4c1bf017bde38f04206f846743d385687c0e0dc72ad5fb8b2336b283fb5`。stderr保留43条lib-test警告；编译2分14秒，测试本身0.16秒，不把会话等待和纠错时间算作测试耗时。

`cargo clippy --lib --message-format=json`：真实exit 0。主线解析完整809条JSON，188条warning、0 error，三个目标文件在任何诊断span中均未命中；实际lib制品及build-finished成功。没有BASE Clippy对照，不能把所有非目标警告认定为既有或声称全库零警告。stdout SHA-256 `5c3b571e722f75ea482bd191d4b50af5b2ebd68d2c7994a628980b193ba33a23`；stderr SHA-256 `37c7200df6c3a17827fba64641b42ed4d9d59f80a562b18e5cc9f125146b35fb`。

`rustfmt --edition 2021 --check src/monitor/push_job/shadow.rs src/monitor/push_job/shadow_tests.rs`及三文件定向`git diff --check`均exit 0、stdout/stderr为空。没有递归格式化push_job.rs子树；其改动仅公开导出块，不宣称整棵模块树已通过格式验收。

源码SHA-256在最终验证前、测试后及实现者结束时保持一致：

| 文件 | SHA-256 |
| --- | --- |
| src/monitor/push_job/shadow.rs | f117095fee89ac5ea59e8212ff00e3eba0cb2f5e36ce1606d1e194533890fc5b |
| src/monitor/push_job.rs | 6d1c208ae465acd897cb7d1b7d3c6e9f95bdb9c97aab4a0f71463ab593a5479a |
| src/monitor/push_job/shadow_tests.rs | fe4afde9ca8b50c3a5492e55220e150eb43536c238d00d39f2aa135f475332ed |

最终完整输出及cwd/命令/退出码分存本地`.superpowers/sdd/2026-09-10-shadow-business-proposals/final-*.stdout/.stderr/.meta`，完整报告为task-1-report.md。原始材料未纳入Git，本记录不是远端CI或生产证据。

## 修复轮1：组合异常的真实反例与验证

[新增公开接口回归](../../src/monitor/push_job/shadow_tests.rs#L1215)覆盖三组：双侧Ready同时缺投影和业务提案；双侧非Ready同时有非法reason和不应存在的业务提案；合法old面对组合无效的new，仍取回原旧对象。每条相关路径保留基础和业务两类固定差异，不以已有基础错误为跳过条件。

产品修复前，`cargo test --lib monitor::push_job::shadow_tests::business_proposal_presence_is_reported_alongside_base_invalidity_and_retains_valid_old -- --exact --test-threads=1`真实exit 101，0 passed / 1 failed / 3363 filtered；在第一组ReadyProposalMissing缺失断言运行期失败。三组在同一测试内，不能声称三组各自跑出了RED。完整fix1-red-combined-presence.stdout SHA为`a3a3feaaf1fa40a01379398a1022bbe90d71f96a079bcde8923873c126572118`。

修复将“基础status不是Completed就跳过”改为“没有成功callback输出才跳过”，每个实际输出都检查提案存在性；只有基础有效且存在性有效的old才保留提案。未改原验证内核、语义合同或发送路径。

修复后整组`cargo test --lib monitor::push_job::shadow_tests -- --test-threads=1`：**26 passed / 0 failed / 0 ignored / 3338 filtered，tool/wrapper/meta exit 0**，包括17旧项与9个业务测试组。主线独立核对全部stdout；编译2分48秒，测试0.22秒，43条库test warning。捕获前缀fix1-shadow-tests，stdout为3222字节，SHA `7594804be16a8e693a70137599c5a111109b828c35a84b6fe1b97476a3602547`；stderr为9986字节，SHA `62c9c0a250566eda11fe54df9cf1282be017f6f4ace480b0437f3f14a3d7366a`。

修复后Clippy完整809条JSON / 188 warning / 0 error，递归检查诊断中的文件span，无三个目标文件命中；实际lib artifact及build-finished成功，meta exit 0。fix1-clippy.stdout SHA `c83e213c99f712420d5d71af906dc26eb7643a2bd6701c70c554f711bc248f97`，stderr SHA `3d0ca368d0afe5449758687fd3cf3a01a368e2f4410edaa06a6b11aaf35b3647`。仍无BASE对照，不把库警告标成已证明全部既有。

两个shadow文件的定向rustfmt --check与三文件diff-check均空输出、meta exit 0。本轮测试后记录并在静态检查结束后核对的源码SHA为：

| 文件 | SHA-256 |
| --- | --- |
| src/monitor/push_job/shadow.rs | 90a2c3d63e65a1e755579c84cb7ca812f5853820ae4fccacac918db490140824 |
| src/monitor/push_job.rs | 6d1c208ae465acd897cb7d1b7d3c6e9f95bdb9c97aab4a0f71463ab593a5479a |
| src/monitor/push_job/shadow_tests.rs | dc2ce3050f31cb7381b491f4daac7dbe9340a1acccee0377f593c8bcf65a9c12 |

**修复后整组测试启动前漏记SHA，不能声称有该次测试前后摘要对照。** 父代理接受的限定验收输入是完整单写捕获、实施者对执行期间无源码变更的确认、测试后固定SHA及后续SOURCE绑定；文件mtime早于捕获创建只作辅助，不是认证。该过程证据缺口进入限定复审，不补造pre-hash，也不为恢复不存在的历史记录重跑同源码测试。

## 开发验证失误及证据限制

首个公开接口RED记录缺类型/函数的E0433/E0425、Cargo状态101；这只是编译RED，不是运行期业务反例。开发中另外修正了局部变量遮蔽和测试辅助闭包生命周期错误。早期合并日志保留命令内状态，但没有保存外层工具返回对象；不伪造运行期RED或事后补造工具退出证据。

**实现者曾误启动两次相同Cargo命令，同时重定向到同一开发日志，违反单队列约束。** 受损文件有残缺test行及两个退出标记，两个底层工具退出码均未知；此前“不存在第二会话”的说法已撤回。主线确认这是真实字节交错而非显示截断，该日志不计入通过数，保持原样、没有手工修复或删除。

主线随后暂停新Cargo，增加本任务捕获器：原子互斥锁拒绝第二命令、同名前缀禁止覆盖、stdout/stderr分开单写、meta独立保存真实退出。一次合成诊断验证了并发拒绝、受控exit 7、精确分流、拒绝覆盖且原文件摘要不变、正常释放锁；未运行生产或Cargo。以上最终测试、Clippy及检查均通过该捕获器串行执行，最终完好证据不抹去开发期违规。

**父代理显式验收例外：历史开发过程不符合单Cargo约束，此事实成立且永久保留。** 仅接受后续互斥捕获、固定源码、独立核对的有效证据作为代码验收输入；旧两次退出未知，不计入任何通过主张。这个例外不是历史过程合规结论，不豁免代码缺陷，也不批准今后放宽单队列要求；过程失败继续进入最终全分支审查。

修复轮1又发生会话记录错误：实现者已启动单例GREEN，却误称尚未启动，并尝试重复启动。捕获器以exit 75拒绝第二命令，没有启动第二Cargo或覆盖日志；“未知/外部会话、队列为空”等错误陈述已撤回。主线再次暂停验证、只读核对任务捕获并要求后续从工具实际返回值存储session ID，轮询不得猜数字。原始GREEN启动映射无法直接恢复，外层tool退出未知，只有完整捕获的命令exit 0；不能反推session归属。实现者报告初步GREEN至整组之间只做定向格式化、未增删断言，但没有保留格式化前摘要，不能事后还原精确字节差异；最终26项覆盖后来的格式化版本。此处与初版真实发生过的并发违规分别保留。

## 独立审查与后续

独立Spec/Quality审查范围为20f215d..b142c9b，结论Needs fixes：

1. 初版新增入口在基础路径已InvalidObservation时提前跳过存在性检查。因此同一路径同时发生ReadyProjectionMissing与ReadyProposalMissing，或非Ready基础绑定错误与NonReadyProposalPresent时，只记录前一种事实。原有用例分别测试两类错误，未覆盖组合；修复要求每个成功回调都检查存在性，只有取回旧提案才同时要求基础和存在性有效。
2. 开发期间违反单Cargo约束，要求父代理显式记录验收例外。处理见上一节；不把后续测试成功写成历史违规未发生。

修复轮1已先运行组合异常RED再修复，26项整组测试、Clippy与定向检查完成，过程局限见上文。原实现者已释放Rust/Cargo；独立限定复审范围为b142c9b..9b2a5df：Finding 1为ADDRESSED；Finding 2为ADDRESSED（显式验收例外，不是历史合规），修复无新增Critical/Important/Minor。结论为“All findings addressed, no new Critical/Important breakage”；本Task关闭，不重开初版未改范围或把过程限制抹去。

完整[真实P-02接线](p02-shadow-integration-handoff-2026-09-10.md)仍需实际完整PreparedAuctionVolumeDispatch、两个真实adapter及同次捕获、可信注册/来源/交易日/构建身份、真实效果端口、合法owner/fence与dispatcher消费。本次没有补真实量比，也不解决[连板](limit-boards-call-chain-2026-09-10.md)或[盘后链分析](chain-post-close-call-chain-2026-09-10.md)的新发现；这些不能被计成已修复或已迁移。
