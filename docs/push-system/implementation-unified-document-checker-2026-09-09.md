# 统一文档门禁：实施与验证记录

日期：2026-09-09。状态：本批产品实现、受影响验证和限定复审完成；首次TDD顺序偏差明确作为一次过程例外保留，不代表全部TDD合规、当前审计对齐或项目全部完成。BASE=`aef7972965f610ed418049593dfff1d55341772e`，首版源码=`1a105863f7e380e3b7f5fa916ea9226d0e498e95`，最终源码=`7a150b24128d7b664620cc42f8402446d1454f2e`。合同见[实施计划](../superpowers/plans/2026-09-09-unified-document-checker.md)。

## 已实现的范围

正式入口为 `ruby scripts/architecture-docs/check.rb --draft|--check [--root ROOT]`。[入口源码](../../scripts/architecture-docs/check.rb#L10)拒绝缺模式、重复模式/root、未知或缩写参数及多余位置参数；参数错误退出2，内容/发布阻断退出1，成功退出0。缺省root相对脚本仓库，显式相对root相对调用目录。

[组合调用](../../scripts/architecture-docs/check.rb#L64)独立校验八份冻结输入，再执行一次Catalog校验、目录Markdown新鲜度、RFC/WBS合同和RFC HTML新鲜度。保留具体错误及path/id并去重；strict中的PROVISIONAL和工作树dirty不遮住目录Markdown陈旧。当前明确只报告 `html_targets=rfc`，没有把蓝图HTML算作已检查。

正式checker只读，不生成/修复文档，不执行生产脚本；子进程内部设置 `GIT_OPTIONAL_LOCKS=0`。CI仅在本地增加完整历史checkout和Rust步骤前的精确strict命令，见[工作流](../../.github/workflows/ci.yml)。没有推送或触发远端CI。

## 性能修正与完整性

真实组合正例从408.770351秒降到72.523631秒，约5.6倍；仅指这一场景，且后一轮新增只读断言，不是整体开发或所有测试提速比例。改动是[同一源字节的词法视图复用](../../scripts/architecture-docs/rust_evidence.rb)和[批量读取Git blob](../../scripts/architecture-docs/catalog.rb#L316)，不跳过文件、符号或当前源码校验，不采用跨运行缓存。

审查前发现并修复一个真实遗漏：若只用blob映射推导历史文件集，mode 160000的 `src/linked.rs` 会消失。真实临时Git反例先失败1项/1断言，再修复为完整路径集合与blob请求分离，通过1项/5断言。Git批协议另覆盖截断、缺帧、类型/ID错误、坏终止符和多余帧，以及含空格/换行的文件名。

## 首版1a10586对应的验证

| 验证 | 实际终态 | 范围 |
| --- | --- | --- |
| Catalog整套 | 39项/445断言，62.444311秒，零失败/错误/跳过 | 正式校验语义和批量读取 |
| Checker关键组 | 4项/48断言，230.893389秒，零失败/错误/跳过 | 完整组合、strict只读、综合漂移、独立WBS陈旧 |
| Checker其余组 | 11项/142断言，308.822385秒，零失败/错误/跳过 | 与上一组互斥，合计覆盖最终15个方法；不是一次最终全套运行 |
| 既有CI定向回归 | 2项/18断言，零失败/错误/跳过 | strict产物门禁及现有CI映射触发格式 |
| 语法及接线 | 六份Ruby语法、CLI可执行位、限定diff检查通过 | 实际 `RfcSpec.ci_rfc_gate?` 可识别本地声明 |
| 实际当前树draft | 退出1，107条内容错误，44.531714秒 | 全部错误与优化前一致，没有过滤旧证据漂移 |
| 实际当前树strict | 退出1，112条错误，44.327718秒 | 107条内容错误加5条发布/工作树阻断 |
| 实际当前树只读 | 包装验证退出0，598项文件/索引前后SHA、字节数、mtime不变 | 两模式stderr均为空；被测checker仍为退出1 |

strict增加的是两份历史JSON的 `provisional`、`worktree_dirty`、`rfc_status_provisional` 和 `wbs_status_provisional`。这不是实际当前draft通过，更不是原硬化任务6或项目全部完成。

原始记录位于隔离树 `.superpowers/sdd/2026-09-09-unified-document-checker/`：实施者 `task-1-report.md`、实际输出与源码hash的 `current-tree-final-verification.json`、固定 `review-aef7972..1a10586.diff`。完整组合正例使用真实历史源码、固定输入/来源依赖闭包及正式生成的Markdown/HTML；不是简化enum或mock组件返回成功，也不替代当前分支验收。

## 明确保留的过程偏差

首条测试命令虽先于CLI实现启动，但慢fixture尚未完成便开始实现，没有取得真正的实现前“缺CLI”RED。事后在自有合法fixture移走CLI得到Ruby退出1/LoadError，并恢复文件，只能证明反例有效，不能补造TDD先后顺序。独立审查将其列为Important；主线接受一次已披露的过程例外，保留原要求和未满足事实，不通过删除重写源码假装改变历史。后续修复仍须实际RED→GREEN，功能与发布标准不豁免，本Task不能声称全部TDD过程合规。

唯一一次完整checker运行曾因测试夹具二进制/UTF-8替换错误退出；其精确footer未保存，旧15/182及“其余14项通过”的转述均撤回。旧单例会话95867的7/8断言转述同样撤回。修正夹具后最终两组覆盖如上，不将分组相加冒称重跑全套；原WBS单例另有5559的真实1项/8断言记录。

## 首轮独立审查与处置

固定范围初审返回Needs fixes：0 Critical / 3 Important / 1 Minor。其中 `--root` 把后续选项吞成路径、返回错误退出码已由源码确认；另有首次TDD过程偏差和未声明的 `-h` 别名。

enum项的初审描述已更正：固定源码第95–96行已有外层 `RustEvidence::Invalid` 捕获，不会因此产生stacktrace或跳过RFC/HTML。原reviewer已撤回原描述；真实反例64.458086秒、1项/7断言/1失败证明的是遗漏strict的 `worktree_dirty`，且重复追加不带证据ID的错误。RFC/HTML检查仍正常执行。原审查文字、更正及反例均保留，不把审查结论本身当作行为证据。

参数反例同样先真实失败：`-h`为1项/16断言/1失败，root吞选项为1项/6断言/1失败。最小修复仅涉及catalog.rb、check.rb及check_test.rb，已提交7a150b2：enum分支局部捕获后带ID返回稳定诊断、继续strict并统一去重；root拒绝选项形状的值；只保留精确 `--help`。

两项缺陷及别名均经真实反例再修复。上表数字对应首版1a10586，不冒称修复后全套重跑；受影响验证如下。主线保留固定初审包、`review-verdict.md`和8933字节修复包 `review-1a10586..7a150b2.diff`，只复审修复范围。

| 修复版验证 | 实际结果 |
| --- | --- |
| 参数/help/root | 2项/65断言，3.061114秒，零失败/错误/跳过 |
| 损坏enum＋strict＋RFC/HTML聚合 | 1项/17断言，65.123014秒，零失败/错误/跳过；带ID错误、dirty及独立组件错误共同保留，stderr空 |
| Catalog相邻定向 | 3项/28断言，3.366652秒，零失败/错误/跳过 |
| 三文件语法与暂存检查 | 三份Syntax OK；精确三文件/53增10删，限定暂存diff检查通过 |
| 真实当前树draft / strict | 33.606025秒 / 32.213416秒；退出1，分别107 / 112条，与首版完整诊断相同 |
| 真实当前树只读 | 598项文件/索引前后SHA、字节数、mtime不变；包装验证退出0，两个checker的stderr空 |

实际修复版证据为 `fix1-current-tree-verification.json`，59736已终态；主线把其中21份代码/资源快照及三份实施者冻结hash与提交前文件逐项匹配。期间没有Git写入；随后三文件本地提交是单独动作，不属于checker副作用。未推送远端、未重跑Rust或未受影响长套件。

独立限定复审最终Approved：I1/I2/M1全部ADDRESSED，I3明确为ACCEPTED PROCESS EXCEPTION；本轮无新Critical/Important/Minor。产品合同符合，不能把过程例外写成首次TDD要求已经满足。完整结论存于 `review-fix1-verdict.md`，没有重复全分支审查。

## 剩余工作与并行边界

- 本批限定复审已关闭；首次TDD过程偏差单独保留，不冒称完全符合原过程。
- [强制当前审计版本](../superpowers/plans/2026-09-09-current-source-audit.md)尚未实现；[107条漂移分析](current-code-audit-delta-2026-09-09.md)不是机器目录已对齐。
- 当前架构蓝图、第二HTML目标、兼容wrapper、两目标统一检查和真实CI/发布证据仍待。
- 完整W01–W21及52 Unit的生产接线、迁移、shadow、晋级与回滚继续按总目标执行。本批没有操作monitor、生产数据库、provider或真实消息。

实施时由一个agent独占源码；测试补证与蓝图只读准备并行，固定源码后独立审查与父线文档整理并行。同文件交接必须先关闭前置审查，不把准备完成算作实现完成。
