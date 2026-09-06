# 第三批 RFC/SQL/WBS 交接候选结果（2026-09-06）

状态：`PROVISIONAL`；第三批 RFC/SQL/WBS 规格与机器校验候选完成（独立整批审查前）。**独立整批审查待 Controller 执行**；下列 Task 1--5 的独立复审结论不代替最终整批审查，第三批尚未宣告最终完成。

本批把每条推送的身份、完成权威、跨库恢复、调度/readiness、shadow/activation/operator、保留期和逐 Unit 工期变成可失败的机器合同。它帮助实施者识别合同缺项、语义倒退和数值漂移，仍不证明线上推送问题已修复。

## 提交与独立复审证据

第三批共同计划基线为 `0bc6a2e`，Rust/Cargo 审计基线为 `07781bf386aafdf202851ae928efee8920387058`。各任务实现、定点修复及 Controller 状态提交如下；结论来自逐任务独立双轴审查记录及已纳管计划勾选，不是本实现者自评。

| 任务 | 实现 / 修复 / 状态提交 | 独立复审与已关闭发现 |
| --- | --- | --- |
| Task 1 输入冻结 | `bb3eccd` / 无修复 / `7143065` | Spec PASS / Quality PASS，Critical/Important/Minor 均 0；八输入字节、角色、路径安全、65/67 裁决已复核。 |
| Task 2 应用合同 | `1280baf` / `d6f516d` / `7e47af1` | 首轮 2 Important：中文要求与语义倒退漏检；补中文规范及身份、canonical、AlreadyTerminal、P01/N02 语义门禁后，限定复审 PASS/PASS，剩余 0/0/0。 |
| Task 3 持久化协议 | `5713b38` / `f226d0f` / `d763d5f` | 首轮 4 Important、1 Minor：非发送事实、SHA 身份、reason/edge 绑定、旧 schema 兼容与 NUL 字节漏洞；复审以 19 项/675 assertions 定点证据关闭，PASS/PASS，剩余 0/0/0。 |
| Task 4 调度与运维 | `824da74` / `507512a` / `246b931` | 首轮 1 Important：occurrence 无独立版本 CAS 模型；补 version、转换请求、零行无副作用和提交证据合同后，限定复审 PASS/PASS，剩余 0/0/0。 |
| Task 5 精确 WBS | `846fa1b` / `edbb509` / `d45dada` | 首轮 1 Important、2 Minor：CLI-chain 未获晋级授权、Float 舍入、非法 ID 诊断不稳定；修复后限定复审 PASS/PASS，剩余 0/0/0。CLI-chain 保持未排序，等待单独产品裁决。 |
| Task 6 候选验证 | checker `4a7ff33eb4136b6008359bb95428b1b5d062762f`；本交接候选另行提交 | checker 补齐四项 strict 发布原因码和 catalog 显式模式；最终独立 Spec/Quality 审查待 Controller 执行，最终审查及完成提交勾选保留未完成。 |

Task 6 产品范围严格为七个路径：本结果、README、第三批计划、`rfc_spec.rb`、`rfc_spec_test.rb`、`check-catalog.rb`、`catalog_test.rb`。计划先补列漏记的四个 checker/test 路径；先提交 checker，再从该 clean HEAD fresh 验证，最后仅更新交接文档和候选状态。没有修改 RFC 正文、WBS、SQL、catalog/source/input、Rust/Cargo、HTML、CI/workflow、模板或 assets。

## 输入与规格制品

八份冻结输入总计 1,221,011 字节；实际 manifest 门禁逐项验证原始尺寸、SHA 和普通文件路径，不重生成或规范化：

| 输入 | 字节数 | authority 边界 |
| --- | ---: | --- |
| `docs/Project_Architecture_Blueprint.md` | 165130 | 冻结架构/拟议设计输入，不直接证明本分支或部署。 |
| `docs/Project_Architecture_Blueprint.html` | 540856 | 派生视觉快照，不是新建离线 HTML 或独立语义权威。 |
| `docs/push-system/comprehensive-reanalysis-2026-09-05.md` | 47302 | 历史风险与验收样本。 |
| `docs/push-system/recent-push-evidence-2026-09-05.json` | 405754 | 脱敏聚合历史证据，不是实时生产快照。 |
| `docs/push-system/all-push-kinds-2026-09-05.md` | 27905 | 原 67-kind 非基线视图。 |
| `docs/push-system/push-documentation-hardening-plan.md` | 9426 | 验收要求及后续发布边界。 |
| `docs/push-system/reanalyse-recent-pushes.rb` | 20408 | 程序快照，仅语法检查，未执行。 |
| `docs/push-system/verify-reanalysis.rb` | 4230 | 程序快照，仅语法检查，未执行。 |

完整原始 SHA 见 [输入 manifest](rfc-input-manifest.v1.json)。本分支源码目录仍为 **65 kind / 102 producer / 52 Unit / 195 evidence**；26 个 monitor kind 映射到 23 个 durable kind，其余 39 个按真实状态保留 adapter/INACTIVE/STARVED/OPT-IN 边界，14 个 durable 状态不充当业务完成状态机。PaperBuy/Watchdog 不并入该基线。

[RFC](push-system-implementation-rfc.md) 固定应用类型、variant、身份与 reverified authority、业务 CAS/append-only、七步跨库恢复、调度/readiness、晋级/回滚、shadow、operator、retention 和样本合同。[SQL](push-system-foundation.v1.sql) 是 SQLite CLI 规格脚本，含 `.bail on`，不是可直接交给任意库 `execute_batch` 的迁移代码；独立 SQL 与 RFC 唯一嵌入逐字节匹配。[WBS](push-system-wbs.v1.json) 是唯一估算/依赖事实源，RFC 摘要由 renderer 校验 freshness。

| 制品 | SHA-256 |
| --- | --- |
| catalog | `0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3` |
| WBS | `d56486ebbb0bba116d7ffae5f428860544c441157db5e4978c67811c825d8d0f` |
| SQL | `1da5cce2beb9baf21a6863893736a5bc070c319eb562eb9b3bf52954f5d8247d` |
| RFC | `a3e109f45c60b73e96bc070e1b17c3496e3ba542336bce2a0ad003457883cd67` |

## 从 clean checker HEAD fresh 验证

验证根为隔离 worktree `push-reliability-20260905`，HEAD=`4a7ff33eb4136b6008359bb95428b1b5d062762f`；开始及全部内容验证结束时 `git status --short` 均为空。下面五个入口分别运行，未复用 Task 5 的 312/4011 输出，未运行旧 archive-writing 测试。

| 完整命令 | runs | assertions | failures | errors | skips | exit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `ruby scripts/architecture-docs/test/rfc_inputs_test.rb` | 12 | 238 | 0 | 0 | 0 | 0 |
| `ruby scripts/architecture-docs/test/source_catalog_test.rb` | 12 | 110 | 0 | 0 | 0 | 0 |
| `ruby scripts/architecture-docs/test/catalog_test.rb` | 33 | 393 | 0 | 0 | 0 | 0 |
| `ruby scripts/architecture-docs/test/rfc_spec_test.rb` | 194 | 3002 | 0 | 0 | 0 | 0 |
| `ruby scripts/architecture-docs/test/wbs_test.rb` | 66 | 409 | 0 | 0 | 0 | 0 |
| 合计 | **317** | **4152** | **0** | **0** | **0** | 全部 0 |

| 完整命令 | fresh 结果 | exit |
| --- | --- | ---: |
| `ruby scripts/architecture-docs/check-rfc-inputs.rb --root .` | `rfc_inputs_valid` | 0 |
| `ruby scripts/architecture-docs/check-sources.rb --root .` | `source_catalog_valid` | 0 |
| `ruby scripts/architecture-docs/check-catalog.rb --root . --draft` | `push_catalog_valid`，保留 NOT CHECKED 边界提示 | 0 |
| `ruby scripts/architecture-docs/check-rfc.rb --root . --draft` | `rfc_spec_valid` | 0 |
| `ruby scripts/architecture-docs/render-catalog.rb --root . --check` | `markdown_current`，保留 NOT CHECKED 边界提示 | 0 |
| `ruby scripts/architecture-docs/render-wbs.rb --root . --check` | `wbs_current` | 0 |
| `ruby scripts/architecture-docs/check-rfc.rb --root . --check` | 精确四项发布 blocker，见下文 | 1，预期 |
| `ruby scripts/architecture-docs/check-catalog.rb --root . --check` | 精确两项 provisional，见下文 | 1，预期 |

严格 RFC 的完整错误输出固定为：

```text
rfc_status_provisional
wbs_status_provisional
rfc_html_missing
ci_rfc_gate_missing
```

严格 catalog 的完整错误输出固定为下列两行；随后的 NOT CHECKED 说明不是第三个错误码，无 `worktree_dirty` 或内容错误：

```text
provisional path=docs/push-system/push-capability-catalog.v1.json
provisional path=docs/push-system/push-evidence-manifest.v1.json
```

RFC HTML 只检查已批准目标 `docs/push-system/push-system-implementation-rfc.html` 的存在与合法普通文件边界；目录、路径组件链接、悬空链接不算交付，不发明 HTML freshness。CI v1 解析 `.github/workflows/ci.yml` 的 jobs/steps，识别独立 `run: ruby scripts/architecture-docs/check.rb --check`（可用单命令 block scalar）；注释、prose、echo、其他 checker、here-document、无效 uses/run 组合或布尔 false step 不算门禁。本批没有创建统一 checker 或修改 workflow，也没有执行 GitHub Actions。

内容错误保留原原因码并排在发布原因码之前，不被过滤、改名或归入预期失败；公开 CLI 回归直接证明 `rfc_counts_invalid` 可与四个 blocker 同时返回。draft 不追加上述四码。catalog 的 `--draft|--check` 互斥，重复模式/未知参数 exit 2；无 mode 的既有 strict 行为保留兼容。

Task 6 的真实 TDD 记录：RFC 四码测试 RED 为 1 run / 2 assertions / 1 failure，旧实现仅两码；GREEN 为 1/8/0。catalog 显式模式 RED 为 1/1/1，实际 exit 2；GREEN 为 1/38/0。发布制品负例组进一步发现无效 CI step 被接受，RED 为 4/96/1，修复后 GREEN 为 4/103/0；这些均走公开 CLI，不使用测试内复制 validator 或 setup failure 冒充缺陷。

## SQLite、语法和数值复算

fresh DDL 使用 `mktemp -d /private/tmp/task6-ddl.XXXXXX` 创建新目录，实际 DB 为 `/private/tmp/task6-ddl.q1ijHL/foundation.sqlite3`；下面两次 `/usr/bin/sqlite3 DB < 文件` 均 exit 0，未打开 `data/**` 数据库：

```sh
set -e
task6_dbdir=$(mktemp -d /private/tmp/task6-ddl.XXXXXX)
/usr/bin/sqlite3 "$task6_dbdir/foundation.sqlite3" < docs/push-system/push-system-foundation.v1.sql
/usr/bin/sqlite3 "$task6_dbdir/foundation.sqlite3" < docs/push-system/push-system-foundation.v1.sql
/usr/bin/sqlite3 "$task6_dbdir/foundation.sqlite3" 'PRAGMA foreign_keys=ON; PRAGMA foreign_keys; PRAGMA integrity_check; SELECT count(*) FROM push_foundation_objects; SELECT type,count(*) FROM sqlite_master GROUP BY type; PRAGMA foreign_key_check;'
```

输出：foreign_keys=`1`，integrity_check=`ok`，`push_foundation_objects` 登记数=`25`；sqlite_master 总计 table=6、index=11、trigger=18；foreign_key_check 无违规行。fresh RFC 全套测试还实际执行以下 SQLite 公开事务/约束证据，不以 DDL 关键词存在代替行为：

| fresh RFC 测试 | 所证边界 |
| --- | --- |
| `test_sql_schema_is_executable_versioned_and_repeatable_without_data_loss` | schema version=1、表/index/FK、带现存行重跑 DDL 不丢数据。 |
| `test_sql_rejects_illegal_states_hashes_reasons_times_and_foreign_keys` | 非法状态/hash/reason/time/FK 拒绝。 |
| `test_intent_identity_and_material_cannot_be_replaced_or_updated` | intent 身份/不可变材料及 REPLACE 覆盖拒绝。 |
| `test_finalization_cas_and_transition_commit_together_and_zero_cas_appends_nothing` | 本地 CAS 与 transition 同提交，CAS 零行不追加事件。 |
| `test_transition_failure_rolls_back_the_prior_cas_and_committed_events_are_immutable` | 事件失败回滚前置 CAS；已提交 transition 禁止 UPDATE/DELETE。 |
| `test_activation_is_versioned_and_journal_is_an_independent_append_only_fact` | 非法 activation 跳转拒绝；新 generation 回滚，journal/manifest 不可改写。 |
| `test_raw_sqlite_cli_guards_do_not_repair_or_change_incompatible_databases`、`test_sql_guard_oracles_detect_real_weakened_schema_mutants` | 原始 CLI 对不兼容 schema fail closed；真正移除 guard 的 schema mutant 使对应行为断言失败。 |

语法命令为 `ruby -c FILE`，逐个覆盖 `scripts/architecture-docs/*.rb`、`scripts/architecture-docs/test/*_test.rb` 及两份导入 Ruby 快照，共 19 文件，全部 Syntax OK/exit 0。`JSON.parse(File.binread(path), decimal_class: BigDecimal)` 逐个解析 `docs/push-system/*.json` 共 5 文件，全部成功。`git diff --check` 通过。

另用 Ruby 标准库 `json/digest/bigdecimal` 只读复算：JSON 数值直接转精确 Rational，每行按 `(O+4M+P)/6`、半入两位检查；逐行求和、20% 汇总缓冲、依赖 DAG 递归最长路径及首批 rank 1--3 分别重算，SHA 用 `Digest::SHA256.file` 读取。没有修改 JSON 或调用 renderer 写模式。

| 复算项 | 结果与解释 |
| --- | --- |
| 工作包 / Unit | W01--W21=21；Unit=52，与 catalog 双向闭合。 |
| 工程基线 | Foundation 281.34h + Unit 547.65h = **828.99h**，103.62 个 8h 工程日。 |
| 一次 20% 缓冲 | 165.80h；合计 **994.79h / 124.35 工程日**。 |
| 交易场景 | 42 owner-changing Unit，42 promotion + 76 observation = **118 conservative sessions**；10 个 conformance-only Unit 不消耗此晋级配额。 |
| 条件日历 | 外部等待 63 business days；125 向上取整工程工作日 + 118 session + 63 = 306；假设周一开始且只有周末休息，`7*floor((306-1)/5)+(306-1)%5+1` = **428 natural days**。 |
| 依赖最长路径 | **208.01h**，不同于单工程师 828.99h 资源总时间。 |
| 首批 | 全部 Foundation 加已批准 rank 1--3，**318.66h / 10 conservative sessions**；不包含未获排序批准的 CLI-chain。 |

428 天是串行无重叠、无额外休市的条件场景，不是承诺日期。交易所休市、样本不足、人工批准仍增加未知时间，上界为 null；保留期至少 90 天及更严格规则的等待、inactive 能力未来激活不计入该场景。同波次和未排序 Unit 的生产顺序仍需批准。旧 98--142h 只是历史对照；W01--W21 使用 `reconstructed_2026-09-06` lineage，没有伪称找回旧逐项估算。

## 工作区与样本保护

隔离区执行 `git diff --exit-code 07781bf386aafdf202851ae928efee8920387058 HEAD -- src Cargo.toml Cargo.lock`：exit 0、零 diff。Task 6 相对 `d45dada` 的产品 diff 只含上述七文件；没有 SQL、`src/**/*.rs`、Cargo、生产状态或 CI 变更。

原 root 工作区只读执行 `git diff --name-only --diff-filter=U | sort -u | wc -l`，前后均为 **160**；`git diff --binary HEAD -- src tests Cargo.toml Cargo.lock | shasum -a 256` 前后均为 `3b0f746129bcdd108680af75da354fe087c00f300416483fdcab01705b48064e`。没有暂存、解决或修改原工作区。

以下四份样本只读取 SHA，开始与结束一致；其中 `data/g5b` 是 JSONL 保护样本，只计算字节摘要，没有解析或连接生产数据库：

| 工作区 / 样本 | 前后相同 SHA-256 |
| --- | --- |
| root `reports/alerts/20260831.jsonl` | `6639c0eb6bc236970952881f8753537840c65d6841b5556157c0d584c618f5c9` |
| root `data/g5b/2026-08-31.jsonl` | `cd5f702d1947fa1fc86af39aae22694e1867b3121068e0b8cb649fad4e6c0914` |
| 隔离区 `reports/alerts/20260905.jsonl` | `7b065405571d49bb898ec997cf7fe92d43e68c5dbbfe6b3d4f9de0b32aee5470` |
| 隔离区 `reports/alerts/20260905.md` | `69265ebc6172a80cbf47cb2beade4420f10186e72915da32d170386dc2427519` |

本次稳定性不撤销首批开发样本曾被改坏的事故结论，不能把当前不变误写为历史上从未受损。

## 未完成边界与下一次交接

- W01--W21 运行时未实现，52 Unit 未迁移，未 shadow/live promote。
- Rust、生产 DB schema、配置、消息模板和业务行为未改；SQL 仅是经临时 DB 验证的规格制品。
- 蓝图 §24/§25 未去拟议化；通用 offline HTML builder、RFC HTML、统一 `check.rb` 与 CI 接线未交付。
- 未部署，没有 `TransportAccepted`、用户已读、交易结果或收益改善证明；没有 provider/LLM/message/order 调用。
- PaperBuy/Watchdog 仍是原混乱工作树排除项；原工作区 160 conflicts 未解决。
- Task 1--5 定点独立复审已通过；独立整批审查待 Controller 执行。最终需核 Q1--Q108、硬化计划 Task 2 与本计划验收，关闭全部 Critical/Important，Minor 修复或明确记录接受理由与成本。

本提交只准备候选交接。Controller 返回整批独立 Spec/Quality 结论后，原实现者才更新最终状态、剩余计划勾选并提交 `docs: complete push RFC and WBS batch`。未 merge、未 push、未 deploy，保留隔离 worktree。
