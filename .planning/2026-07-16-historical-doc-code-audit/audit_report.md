# 历史文档功能与 Bug 全量代码兑现审计

审计日期：2026-07-16
固定代码快照：`c1e53321b2f4fb5d1f21cc0baf7ff4ade1ffcb7b`
历史起点：`3c7fad274a462a972bc5f6ef183d2119ef30a708`
最终状态：**In Progress / Blocked，不是 Done**

## 结论

**否。过往文档描述的功能和 Bug 并未在代码层面全部解决。**

该结论来自全量路径对账与逐条声明账本，不是抽样反例：

- 243/243 个项目文档路径已覆盖，包括 181 个当前可见路径和 62 个 Git 历史删除路径。
- 430/430 个实现区域路径已覆盖：固定快照 402 个；工作区独有、并发改动或二进制资产 28 个，逐个记录 skip 理由。
- 242 个文本型文档共 85,132 行、约 4.93 MB，全部读取并校验 SHA-256；唯一 vendor PDF 共 60 页，另行提取 77,146 字符全文并确认属于供应商 API/版本参考，不是本项目验收承诺。
- 高召回扫描得到 12,311 条候选行，人工/独立审查整理为 5,423 条可追溯来源声明。该数量保留跨设计、计划、验收和复盘的重复来源，不等于 5,423 个唯一功能。
- 文档 coverage missing=0，代码 coverage missing=0，ledger ID/列/状态校验无错误。

## 声明账本结果

| 状态 | 数量 | 判定含义 |
|---|---:|---|
| verified_complete | 19 | 在固定快照中找到实现、测试及失败路径证据 |
| partial | 1,542 | 有代码，但生产调用、测试、失败处理或验收不完整 |
| unresolved | 1,933 | 当前代码直接显示未实现或缺关键闭环 |
| contradicted | 13 | 文档的完成声明被当前代码或更新文档反证 |
| unverifiable | 1,740 | 缺生产/实盘/外部证据，不能判为完成 |
| superseded | 176 | 已被后续设计或同 SHA 文档取代 |

仅 19 条来源声明达到完整证据标准；其余绝大多数不能支持“全部完成”。对早期自动化高召回条目采取保守判定：候选符号映射本身不算完成证据。

## Standards 审查结果

全量代码规范审查形成 63 个仍开放 finding：

| 严度 | 数量 |
|---|---:|
| BLOCKER | 10 |
| CRITICAL | 20 |
| HIGH | 29 |
| MEDIUM | 4 |

最关键的未闭环项：

| 规则 | 当前代码证据 | 判定 |
|---|---|---|
| 2.1 / 2.8 数据源与假实现 | `src/broker.rs:92-198` 默认 mock 报价返回 0；多个 broker `push_*` 仅日志；`src/broker/ib.rs:41-60` 返回零值/空行业 | unresolved |
| 2.2 / 2.3 坏数据 | Sina、Baostock、GTIMG 与 repository 多处把解析失败/NULL 转成 0 或今天日期 | unresolved |
| 2.4 新鲜度 | freshness 脚本在 DB/sqlite 缺失时成功 SKIP，且按日历日而非交易日；Grafana 仍标 60s TTL、30s 刷新 | unresolved |
| 2.5 测试/实盘隔离 | `src/bin/monitor/main.rs:2080-2120` 的 `--test` 刻意保持 prod；E2E 使用非 `TEST_CODE` 标的 | contradicted / unresolved |
| 2.6 下单安全 | 模拟网关只做 60 秒内存去重；缺完整资金、100 万、百股、涨跌停和 ≥50 万二次确认；paper 风控伪造 100 万总资产与 30% 现金 | unresolved |
| 2.7 审计 | AuditDispatcher 只 stdout/内存计数；JSONL 可修改、写错被吞、仅保留 7 天 | unresolved |
| 2.8 fake gate | 检查脚本仅匹配窄正则，未拦住日志-only broker、mock health 与 T-14/T-15 stub | ineffective gate |
| 2.10 BR | 工作区检查仍报告 16 个 PENDING BR 与 21 个 warning，但返回成功 | partial / ineffective gate |

此外，v16 的三总线/策略内核缺生产消费者，v17 的默认 L6 主路径、持久化、查询、回放及旧路径清退仍未闭环；历史 QMT 计划没有真实 provider/fallback/E2E。

## Gate 验证

命令在 `git archive c1e53321...` 的隔离快照执行，没有启动 monitor、券商连接、真实推送或下单。

| Gate | 命令 | 结果 |
|---|---|---|
| B | `cargo fmt --all -- --check` | **FAIL**，exit 1，大量格式差异 |
| B | `cargo clippy --all-targets --all-features -- -D warnings` | **FAIL**，exit 101，335 errors |
| B | `cargo test --all-targets --all-features --quiet` | **FAIL**，exit 101；`v14_e2e.rs:285` 缺参数，`rule_filter_benchmark.rs:616` match 不完整 |
| C | 固定快照 `bash tools/compliance/check.sh` | **FAIL**；提交中缺两个被入口调用的合规脚本 |
| C | 当前工作区同一命令 | 脚本 exit 0，但含 16 PENDING BR、21 warnings、155 个 `unwrap_or(0.0)`，模板层缺失仍 SKIP/PASS |
| D | coverage/live evidence | **FAIL / UNVERIFIABLE**；CI 只要求 60%，不是全局 80% / 核心 95%；无 auditor sign-off，未执行危险实盘动作 |

所以 Gate A–D 均不能支持 Release Ready，按 AGENTS Part 4 必须保持 **In Progress / Blocked**。

## 覆盖边界

- 本报告完整覆盖本地工作区项目文档、Git 可达历史文档和固定快照代码；第三方 EMQuant 手册也完成全文提取，但按外部参考资料记为 0 project claim。
- `.github/copilot-instructions.md` 与外部 issue tracker 配置缺失；外部 issue 正文无法凭空恢复，相关声明保持 unverifiable。
- 固定快照后无新提交；审计结束时 `src/event/bus.rs`、`src/event/mod.rs` 和 `src/event/jsonl_writer.rs` 存在并发工作区改动，未作为固定提交的完成证据。
- 未运行可能触发真实通知、账户访问或订单的 monitor/live 验证，遵守数据与资金安全红线。

## 审计产物

- `doc_manifest.tsv`：全量文档路径、来源提交、hash、行数和跟踪状态。
- `code_manifest.tsv` / `code_scan.tsv`：全量实现路径与逐文件扫描指标。
- `candidate_counts.tsv`：每份文档的高召回候选统计。
- `agents/early_claims.tsv`：v9-v14、pre-v9 与历史别名的 5,242 条来源声明。
- `agents/late_claims.tsv`：其余项目文档的 181 条规范化声明。
- `agents/standards_findings.tsv`：63 个 Standards finding。
- `agents/*_coverage.md`：逐文件覆盖及 skip/0-claim 理由。
- `scripts/check_coverage.py` / `scripts/validate_ledgers.py`：可复现的无遗漏和结构校验。
