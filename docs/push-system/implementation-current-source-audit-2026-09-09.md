# 历史规范与强制当前源码审计：实施记录

日期：2026-09-09。状态：Task1及后续Task2均已完成本地验收与独立审查，当前制品为047b4ab；两次不同层级的摘要遗漏及修复证据分别保留。全部current材料保持PROVISIONAL，strict仍有真实发布阻断，本记录不批准生产切换或Unit晋级。

后续发现更新：蓝图核对指出MU-auction-volume.note仍有旧“外层insert/两次数据读取”，该行未被ff94eca的kind修复触及。新增[计划Task2](../superpowers/plans/2026-09-09-current-source-audit.md#task-2-修正当前migrationunit竞价摘要的残余历史语义)已在047b4ab修正，正式生成、真实树及独立Spec/Quality均通过；下游蓝图现在可以使用最终SHA继续。以下Task1证据保留原时点，顶部材料表已更新至Task2当前字节，不能混用两个版本的SHA。

实施依据：[已批准实施计划](../superpowers/plans/2026-09-09-current-source-audit.md)。唯一开发树为 `.worktrees/push-reliability-20260905`，分支 `codex/push-reliability-20260905`；Task BASE为`6e58f1e2d79bd183451c96d0dbfc9ab1db9f20f3`，初版源码提交为`c2e33a2508761835288c0a421d874ecb9dbdf358`（10文件，20604行新增/51行删除，主要为机器JSON/生成目录），最终摘要修复为`ff94eca98961d63d13691658fe5fb2d9a088fc56`（仅三份current制品，4行新增/4行删除）。不能把Rust/Cargo源码pin当本Task提交。

## 本次交付的行为

- [Catalog.validate](../../scripts/architecture-docs/catalog.rb#L20)同一入口强制执行历史pair、current pair及SourceCatalog。历史规范只匹配固定历史Git树；current必须同时匹配自己的Git树和当前工作树。任一current JSON缺失，draft和strict都失败，没有历史-only跳检开关。
- [CurrentAudit](../../scripts/architecture-docs/current_audit.rb#L5)是当前事实域，不是新运行时目录。原RFC/WBS/SQL及include_bytes使用的历史材料不换路径、不换身份；current manifest按原始字节SHA绑定current catalog和两份历史JSON，JSON空白改动也会破坏绑定。
- 业务kind、producer总引用及trigger/source/authority/policy的证据依赖各自闭合；architecture组、supporting文件及反向architecture_ids另行闭合。架构引用不能替业务缺失引用补账，同一真实声明可分别被两域引用，但不能复制locator制造伪证据。
- [renderer](../../scripts/architecture-docs/render-catalog.rb)默认仍输出历史Markdown；新增显式`--current`固定输出新Markdown。[总入口](../../scripts/architecture-docs/check.rb)分别检查两份派生文档；provisional或另一个pair出错，不会遮住独立有效pair的新鲜度、RFC/WBS或HTML错误。
- 诊断保留reason code并区分`pair=historical/current`；baseline错误继续保留origin。strict保留全部真实发布阻断，正式检查不修复输入或刷新派生文件。独立Catalog strict的Git status同样禁用可选索引写锁，不只保护总入口。

## 当前材料与身份

| 制品 | 角色 / SHA-256 |
| --- | --- |
| [当前机器目录](push-current-capability-catalog.v1.json) | current-source-audit；`b3c04218e3548f80c026db905e3d0ac2eed59d7ce24efeefa8e69a20b417de93` |
| [当前源码manifest](push-current-evidence-manifest.v1.json) | current-source-audit；`309319f13b599d400f72f9b17ac607e5f6aaa9f8e9ee9c24511f02629d34f13c` |
| [当前四时段目录](push-current-capability-catalog.md) | 派生Markdown；`e4d12fa44a7da92e79a0229e69581e2d6d90cbea9ddf78d1bc3d14b326352303` |
| [历史机器目录](push-capability-catalog.v1.json) | 原字节保持；`0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3` |
| [历史源码manifest](push-evidence-manifest.v1.json) | 原字节保持；`54dc705961da7a6deb458009d2125ee612257d82bad3c14b65d25642e09b64fa` |

历史Rust pin=`07781bf386aafdf202851ae928efee8920387058`；当前Rust/Cargo pin=`aef7972965f610ed418049593dfff1d55341772e`，并非要求HEAD始终等于pin。文档/工具提交不改变完整Rust/Cargo字节时，pin可以保持；不允许靠刷新hash掩盖新源码变化。

当前manifest覆盖548文件：546个src Rust文件与Cargo.toml/Cargo.lock，包含79新增文件（46生产源码、33测试源码）。250个声明证据由正式RustEvidence定位，9个架构组另列库能力/测试/生产未接线边界；文件级SHA不能替代每个调用边的语义证明，也不覆盖全仓594个Rust文件或外部部署。

65 kinds、102 producers、52 Units身份/状态/owner/occurrence保持。kind主归属为盘前10、集合竞价6、盘中21、盘后28；状态为ACTIVE 36、INACTIVE 22、STARVED 5、OPT-IN 2。主归属不是跨时段触发次数，ACTIVE不是Ready/部署/接收；PaperBuy与Watchdog仍为排除项。对每个producer的触发、输入、完成权威及策略，阅读当前生成目录，不能用上述总数计算迁移完成率。

## 不只是刷新行号

本批复用[107项漂移分类](current-code-audit-delta-2026-09-09.md)，但正式定位全部当前证据。历史195个声明重新绑定当前主体，另补55个必要声明，包括原9+29候选之外的prepare/execute/helper依赖。

八项实质变化的当前证据关系涉及auction来源观察及冻结准备、一次采集同批消费、Foundation envelope/template身份、全局/单decision恢复scope、过期attempt恢复、summary与hydration同scope等。独立初审发现AuctionVolume producer层虽更新，kind.note仍保留旧“双次采集”说明；此遗漏已在ff94eca替换，并同步原字节绑定和正式生成Markdown，限定复审I1已关闭。当前明确一次采集、同一snapshot形成消息/records/notified_codes，sink与全部逐票记录成功才推进通知集合；部分记录失败不回滚、非原子和bool不证明TransportAccepted的边界保留，不能将整个MigrationUnit标为完成。

Foundation production固定拒绝、测试专用effect绑定、测试专用startup，以及未找到生产caller的readiness/metrics/SLA等边界仍明确存在。新声明与对应测试文件进入审计，不意味着真实main已接线，也不替代W15–W21和各Unit验收。

## 验证结果与失败记录

Ruby环境为2.6.10，Git为2.50.1。本批没有运行Cargo、生产CLI、monitor、模型、provider、消息发送、数据库迁移或远端CI。

| 验证范围 | 实际终态与版本口径 |
| --- | --- |
| 首条B→C行为反例 | 产品修改前1项/1断言真实失败；实现后1项/13断言通过，逐份移除current材料在两模式都失败 |
| Catalog整套 | 首轮47项/615断言有1个hardlink诊断失败；修复后48项/620断言全通过，175.165171秒 |
| 最终renderer | 后续缺失root/parent保护更改后，4项/58断言通过；不将前一整套结果冒充后续所有新增用例同次全跑 |
| 新增current边界 | 非祖先/缺源码及architecture-only两项/18断言通过；原字节、闭包、symlink、索引只读等各有独立真实反例与修复记录 |
| 总入口整套 | 20项/270断言有1个新增测试错误码名称失败，1112.437846秒；其余19项通过，原失败保留 |
| 总入口最终定向 | 修正测试预期后1项/10断言通过，45.889830秒；实际到达RFC旧metadata拒绝与WBS Unit快照防绕过断言。产品代码无需因该预期错误修改 |
| RFC/WBS定向兼容 | RFC 2项/84断言、WBS 2项/7断言通过；历史规范身份不随current更新 |
| 两份Markdown与静态 | 两个renderer `--check` 均输出markdown_current；最终7个Ruby语法通过，限定diff无空白问题；主线另核7个最终源码SHA与实施报告一致 |

以上不是“最后一次整套20项全绿”或“全部Rust测试通过”的声明；整套与修正后定向分别绑定其实际版本和覆盖。详细命令、seed、session、失败及源码SHA向量保留在本Task `.superpowers/sdd/2026-09-09-current-source-audit/task-1-report.md`。

### 主线真实工作树验收

初版父线session18046已终态wrapper exit0（draft33.019570秒、strict32.608358秒），原记录保留。I1修复后在最终七个Ruby和三个current制品上再执行，session92625已终态wrapper exit0；下表绑定ff94eca的制品字节：

| 命令 / 范围 | 实际结果 |
| --- | --- |
| `ruby scripts/architecture-docs/check.rb --draft --root .` | exit0，28.340874秒；`architecture_docs_valid html_targets=rfc`，stderr为空 |
| `ruby scripts/architecture-docs/check.rb --check --root .` | exit1，28.595672秒；仅历史/current四项provisional、RFC/WBS两项provisional和提交前worktree_dirty，stderr为空 |
| 只读证明 | 610项相关源码/测试/模板/资产/冻结输入/current制品及Git index的SHA、尺寸、mtime前后相同；changed_files为空 |
| 原字节边界 | Rust/Cargo相对aef7972、全部历史冻结材料相对Task1 BASE的限定diff均为空；该时点三制品SHA保存在fix1-current-tree-verification.json，不与顶部Task2当前表混用 |

严格模式仍失败是有效发布阻断，不是内容错误；未过滤或修改状态。前置checker 7a150b2的107/112内容漂移已不再出现在这次实际树结果中。初版记录保留在本Task `current-tree-verification.json`；修复后的核心结果、610项摘要、三制品与索引快照保存在 `fix1-current-tree-verification.json`，逐checker文件快照明细保留在92625终端输出。这不是远端CI、生产数据或完整运行时验收。

独立初审覆盖6e58f1e..c2e33a2，结论C0/I1/M0；唯一I1为上述kind摘要遗漏。原实施者修复后，原审查者仅复核c2e33a2..ff94eca：I1 ADDRESSED、无新增问题，Spec通过、Quality Approved。初审无法仅从diff确认的实际树和冻结边界由上述父线证据闭合；生产caller否定结论仍限定既有审计范围，不扩张为包外不存在。报告分别保留在 `review-verdict.md` 与 `fix1-review-verdict.md`。本Task通过不等于整个项目完成。

## Task2：Unit层残余说明纠正

Task2 BASE=`b531b51fdee7040303fa9a21538e6012445bcac9`，SOURCE=`047b4ab0ac1133a828efe2052eb60a541611e335`。仅三份current制品4行新增/4行删除；[Unit摘要](push-current-capability-catalog.v1.json#L10574)明确同一snapshot、dispatcher内部推进通知集合，并保留非原子/部分写失败不回滚/bool不证明接收。依据为[main一次采集与调用](../../src/bin/monitor/main.rs#L9705)、[prepare](../../src/bin/monitor/push_templates.rs#L6132)、[execute](../../src/bin/monitor/push_templates.rs#L6189)。历史Unit身份/owner/occurrence/phase不变。

实施者用真实BASE对象逐项比较，证明解析内容仅Unit.note与catalog原字节绑定改变；正式locator对应四声明、65/102/52计数、renderer生成及单独--current --check通过，限定diff检查通过。生成和检查最初在同一shell执行，整体exit0、两个成功输出均保留，后续单独check亦exit0；不伪造当时未单独捕获的第一个进程退出码。工具/测试/Rust/Cargo/历史JSON限定diff为空，未为中文文案增加validator或重跑未变套件。准确命令及输出保留task-2-report.md。

父线session34566已终态wrapper0：draft exit0/41.405351秒，strict exit1/39.326052秒，仅历史/current四项provisional、RFC/WBS两项provisional与提交前dirty；610项SHA/尺寸/mtime和索引不变，源码pin与冻结材料diff为空。完整输出保留task2-current-tree-verification.json；其implementation_task_base是Task2 BASE，harness原task_base字段仍表示冻结材料6e58f1e基线。当前三SHA与顶部表一致。

独立审查范围固定b531b51..047b4ab，仅本Task三制品：Spec符合、Quality Approved，Critical/Important/Minor均无。审查者实际核对main→snapshot→prepare→execute及真实sink的is_pushed来源；父线自己的终态34566闭合其无法独立重跑的快照/严格模式要求。完整结论保留task-2-review.md。Task2已关闭，不重审整个Task1或运行时，不把这项完成扩张为全目标完成。

## 使用与剩余工作

```bash
ruby scripts/architecture-docs/check.rb --draft --root .
ruby scripts/architecture-docs/check.rb --check --root .
ruby scripts/architecture-docs/render-catalog.rb --current --check --root .
```

检查命令只读。需要刷新current派生Markdown时，显式使用renderer的`--current --write`，不手改生成区、不覆盖历史JSON或冻结蓝图。

当前HTML仍只有rfc目标。[完整当前蓝图与第二HTML计划](../superpowers/plans/2026-09-09-current-blueprint-offline-html.md)随后消费已验收材料，补新蓝图、兼容入口、双目标门禁和真实浏览器验收；[蓝图输入核查](current-blueprint-inventory-2026-09-09.md)已区分当前调用链、默认值与未验证生产状态。

整个目标仍包括完整W01–W21、52个Unit的shadow/单owner晋级/观察/回滚/清理、生产来源及身份认证、operator/approval、Q39/SLA与留存安全、旧v5–v9审计兼容，以及实际CI/生产验收。本Task及本地draft通过均不能替代这些交付。
