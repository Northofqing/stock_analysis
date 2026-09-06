# 推送文档硬化实施计划

> **面向执行代理：** 必须使用 `subagent-driven-development`（推荐）或 `executing-plans`，逐项实施本计划。各步骤使用复选框（`- [ ]`）跟踪状态。

**目标：** 产出可复现的当前架构审计，并单独产出达到实施就绪标准的推送 RFC；补齐目录、证据、估算、离线 HTML 和 CI 校验。

**架构：** Markdown 和纳管的 JSON 是规范事实源。使用仅依赖 Ruby 标准库的工具校验来源、证据和目录不变量，并通过独立模板与本地 Mermaid 资源生成两份 HTML。

运行时 Rust 代码和生产配置保持不变。

**技术栈：** Markdown、JSON、Ruby 标准库（`json`、`digest`、`erb`、`base64`）、Git、GitHub Actions。

**规格依据：** `docs/push-system/grill-decisions-2026-09-02.md`

## 全局约束

- 当前来源工作树不干净，因此所有产物保持 `PROVISIONAL`；在干净基线替换当前基线前，严格发布检查必须失败。
- 不得修改 `src/**`、`config/**`、数据库、生产状态或密钥。
- 本工作树包含用户无关的产品改动时，不得暂存或提交。
- 兼容仓库已安装的 Ruby 版本；不得使用 `filter_map`、`tally` 或非标准 gem。
- v18/v19 原始来源文档必须保持逐字节不变。
- 四个交易时段是规划 Epic；原子 Unit 身份为 producer + occurrence family + completion owner。

---

### 任务 1：来源治理与文档状态

**文件：**
- 修改：`.gitignore`
- 修改：`docs/README.md`
- 修改：`docs/v18.x/README.md`
- 修改：`docs/v19.x/README.md`
- 新建：`docs/push-system/design-source-catalog.v1.json`

**接口：**
- 输入：现有九份 v18/v19 来源文件。
- 输出：不再被忽略的不可变来源集，以及供 RFC/校验器使用、以 SHA 支撑的设计裁决。

- [ ] 显式放行 `docs/push-system/` 和九份来源文档，同时不得放行其他无关文档。
- [ ] 从 v19 README 移除已退役的活跃规则指针，将“文件放错位置”改为有证据边界的“版本标签冲突”。
- [ ] 使用合法 JSON 记录每份来源的路径、SHA-256、自声明版本/状态、裁决、冲突及取代它的文档。
- [ ] 运行 `ruby -rjson -e 'JSON.parse(File.read("docs/push-system/design-source-catalog.v1.json")); puts "ok"'`；预期输出 `ok`。
- [ ] 对所有受治理来源运行 `git check-ignore`；预期无输出且退出码为 1。

### 任务 2：精确实施 RFC

**文件：**
- 新建：`docs/push-system/push-system-implementation-rfc.md`

**接口：**
- 输入：已批准的决策记录，以及当前蓝图 §24.10--§24.19/§25.9。
- 输出：供目录和校验器使用的规范类型、存储、协议、门禁、WBS 和验收标准。

- [ ] 定义范围、非目标，以及 Foundation → 原子 Unit → 清理的拓扑。
- [ ] 精确定义 `RunContext`、`PreparedFacts`、`PreparedPush`、`JobDecision`、`VerifiedTerminalRef`、`DeliveryResult`、`CompletionPolicy` 和带命名空间的 `ReasonCode` 合同。
- [ ] 为 intent、transition event 和 promotion journal 定义可执行的 SQLite DDL，包含身份、不可变哈希、CAS generation 和约束。
- [ ] 定义 intent、activation 和 authority 状态转换表、跨库顺序，以及每个崩溃边界的恢复/重发规则。
- [ ] 定义 PhaseScheduler/readiness、shadow 比较、兼容性、操作员 CLI、保留期和安全约束。
- [ ] 恢复 W01--W21，列出首批原子 Unit、三点估算/依赖，以及工程人日、交易周和日历跨度的计算公式。
- [ ] 定义每个 Unit 及全项目的验收门禁，不得保留占位项。
- [ ] 运行 `rg -n 'TBD|TODO|implement later|待补|待定' docs/push-system/push-system-implementation-rfc.md`；预期无输出。

### 任务 3：机器可读目录与稳定证据清单

**文件：**
- 新建：`docs/push-system/push-capability-catalog.v1.json`
- 新建：`docs/push-system/push-evidence-manifest.v1.json`

**接口：**
- 输入：monitor 内部的 65-kind 枚举、当前业务审计、Git/源码 symbol。
- 输出：精确的 phase/status/Unit/policy/evidence 覆盖，以及可复现的证据锚点。

- [ ] 精确添加 65 个唯一 PushKind 条目，包含 phase、status、Epic、Unit、producer/trigger/source/authority/policy 和 evidence ID。
- [ ] 在目录元数据中记录临时 HEAD、脏路径和源文件 SHA。
- [ ] 为每个 evidence ID 定义仓库相对路径、symbol/kind、symbol locator、symbol SHA-256 和派生的起止行号。
- [ ] 使用当前、与 symbol 绑定的证据替换过时的 Candidate/BR-232 锚点。
- [ ] 使用 Ruby 解析两个 JSON 文件；预期 JSON 合法且恰有 65 个唯一条目。

### 任务 4：拆分当前审计与拟议设计

**文件：**
- 修改：`docs/Project_Architecture_Blueprint.md`
- 修改：`docs/v19.x/v19.3-push-workflow.md`

**接口：**
- 输入：任务 2--3 产出的 RFC 和目录。
- 输出：只包含当前事实、状态为 PROVISIONAL 且有代码证据支撑的快照。

- [ ] 将全局状态从 `Implementation-Ready` 改为 `PROVISIONAL`（有代码证据的当前架构快照）。
- [ ] 保留 §24.1--§24.9 的当前审计；将 §24.10--§24.19 替换为简洁的 RFC/目录链接和当前状态边界。
- [ ] 保留 §25.1--§25.8 的覆盖审计，将过时证据修正为 evidence ID/symbol，并用 RFC 链接替换 §25.9 的重复计划。
- [ ] 更新文档导航和 v19.3 历史指针，使其指向新的规范文档。
- [ ] 在蓝图中搜索拟议 DDL、CompletionPolicy 和 MigrationUnit 排期的重复内容；预期只保留链接和当前状态讨论。

### 任务 5：可复现的离线 HTML 构建器

**文件：**
- 新建：`scripts/architecture-docs/build.rb`
- 新建：`scripts/architecture-docs/templates/document.html.erb`
- 新建：`scripts/architecture-docs/assets/mermaid.min.js`
- 新建：`scripts/architecture-docs/assets/mermaid.LICENSE`
- 新建：`scripts/architecture-docs/test/build_test.rb`
- 修改：`scripts/render-architecture-blueprint-html.rb`
- 重新生成：`docs/Project_Architecture_Blueprint.html`
- 新建：`docs/push-system/push-system-implementation-rfc.html`

**接口：**
- 输入：任一规范 Markdown 文件、独立模板和本地固定版本的 Mermaid 字节。
- 输出：提供 `build TARGET`、`build --all`、`--check` 和 `--draft` 命令，并保证字节级确定性产出。

- [ ] 先写失败测试，证明目标 HTML 不存在时构建器能创建产物、产物不含 `http://`/`https://` 运行时依赖，且能检测陈旧产物。
- [ ] 运行 `ruby scripts/architecture-docs/test/build_test.rb`；构建器存在前预期测试失败。
- [ ] 从已正常工作的 Markdown 子集 renderer 中提取最小通用实现，并且只读取 Markdown、template 和本地资源。
- [ ] 在每份 HTML 中嵌入 Mermaid 字节及许可证/版本元数据；保留源 Markdown、图表源码、来源哈希、搜索、主题、折叠和打印行为。
- [ ] 保留旧 renderer 路径，作为新构建器的兼容 wrapper。
- [ ] 运行 `ruby scripts/architecture-docs/test/build_test.rb`；预期所有测试通过。
- [ ] 只删除测试临时目录内生成的临时产物；不得删除项目交付物。

### 任务 6：统一校验与 CI

**文件：**
- 新建：`scripts/architecture-docs/check.rb`
- 新建：`scripts/architecture-docs/test/check_test.rb`
- 修改：`.github/workflows/ci.yml`

**接口：**
- 输入：决策记录、RFC/WBS、各类目录、evidence manifest、source catalog、Markdown、模板、资源和 HTML。
- 输出：`ruby scripts/architecture-docs/check.rb --draft`，以及严格的 `--check` 发布门禁。

- [ ] 先写失败测试，覆盖 kind 缺失、kind 重复、来源 SHA 漂移、symbol 漂移、Q1--Q55 行缺失、W01--W21 行缺失、HTML 陈旧，以及严格模式拒绝临时产物。
- [ ] 实现可直接指导修复且以非零码退出的校验；`--draft` 只能跳过干净 commit/发布状态要求，绝不能跳过内容、哈希或覆盖检查。
- [ ] 在 Rust 格式检查前增加 CI 步骤：`ruby scripts/architecture-docs/check.rb --check`。
- [ ] 运行校验器测试；预期所有测试通过。
- [ ] 运行 draft 检查；预期成功。
- [ ] 运行 strict 检查；预期只出现已记录的临时/脏基线失败，不得有其他错误。

### 任务 7：最终验证与交接

**文件：**
- 修改：`task_plan.md`
- 修改：`findings.md`
- 修改：`progress.md`

**接口：**
- 输入：此前所有任务产物。
- 输出：有证据支撑的完成报告，以及明确的剩余发布阻断项。

- [ ] 运行 `git diff --check`；预期退出码为 0。
- [ ] 运行两个 Ruby 测试文件和 `ruby scripts/architecture-docs/check.rb --draft`；预期零失败。
- [ ] 运行 `ruby scripts/architecture-docs/build.rb --all --check --draft`；预期两份 HTML 都是最新且可离线使用。
- [ ] 运行 `cargo test --test unified_data_architecture -- --test-threads=1`、`cargo test --test tool_binary_process_isolation -- --test-threads=1` 和精确的 gRPC catalog 单元测试；预期分别为 15/15、8/8 和 1/1。
- [ ] 确认 `git status --short` 中没有由本计划造成的意外产品/配置改动。
- [ ] 记录严格发布仅受用户所有的脏来源基线阻断；获得干净 commit 前，不得声称这是正式的当前快照。
