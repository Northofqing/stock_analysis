# BR-135 Unsafe 心跳去重实施计划

> 使用 `executing-plans`、`tdd` 与 `verification-before-completion` 依次执行；每个行为先 RED，再 GREEN。

**目标：** 同一 Unsafe 事件不再每 30 分钟重复外发，但每 30 分钟保留权威内部心跳审计。

**架构：** `monitor::data_mode` 的纯状态机输出外部状态通知、内部心跳或静默三种动作；`main` 负责执行副作用并仅在权威成功后提交状态；模板层只保留真实状态变化的 T-02 外发。

## 受影响文件

- `docs/business_rules.md`：先注册修订后的 BR-135。
- `docs/superpowers/specs/2026-08-22-br135-unsafe-heartbeat-dedup-design.md`：Gate A 设计、失败模式与回滚。
- `src/monitor/data_mode.rs`：稳定指纹与纯状态机。
- `src/bin/monitor/main.rs`：外发/审计编排和失败重试。
- `src/bin/monitor/push_templates.rs`：删除持续异常外发分支，支持同模式事实变化外发。
- `src/bin/monitor/presentation_registry.rs`、`src/bin/monitor/br196_test_delivery.rs`：退休 reminder presentation 并同步闭集计数。

## Task 1：纯状态机

- [x] 写测试：首次 Unsafe 返回外部通知；确认后 1799 秒静默；1800 秒返回内部心跳。
- [x] 写测试：相同 missing 集合顺序变化不改变指纹；missing 集合变化返回外部通知。
- [x] 写测试：内部审计失败不推进时间；恢复确认后清空；时钟回退报错。
- [x] 运行 RED：`cargo test --lib monitor::data_mode::tests::br135 -- --test-threads=1`
- [x] 最小实现并运行 GREEN。

## Task 2：模板外发边界

- [x] 写测试：同一 Unsafe 指纹无变化不生成外发计划；指纹变化用 `T-02-data-mode`。
- [x] 删除 `PersistentUnsafeReminder` 外发 reason、renderer 与 catalog preview。
- [x] 运行：`cargo test --bin monitor br135 -- --test-threads=1`

## Task 3：主循环内部心跳

- [x] 写可注入审计闭包测试：只有 `InternalHeartbeat` 调用审计，成功后提交，失败后保持 due。
- [x] 使用 `event::publish_delivery` 写 `data_mode_unsafe_heartbeat_v1 / Deduped / internal_audit`。
- [x] 保留唯一 60 秒调度和现有 BR-225c 恢复防抖。
- [x] 运行 monitor 定向测试。

## Task 4：presentation 闭集同步

- [x] 从 registry 和 BR-196 descriptor 清单删除 `T-02-data-mode-reminder`。
- [x] 按实际闭集同步 family/catalog 计数，不改变 PushKind 清单。
- [x] 运行 `cargo test --bin monitor br196 -- --test-threads=1` 与 dry-run catalog 测试。

## Task 5：验证与证据

- [x] 执行 `cargo fmt --all -- --check`：被无关既存格式漂移阻断；BR-135 直接文件 scoped rustfmt 通过。
- [x] 执行 `cargo clippy --workspace --all-targets --all-features -- -D warnings`：被无关 `performance` 四个既存 lint 阻断；没有 BR-135 诊断。
- [x] 执行 `cargo test --workspace --all-targets --all-features -- --test-threads=1`：lib 2752/2752 通过；monitor 682 通过，仅两个基线静态字符串计数测试失败。
- [x] 执行 `bash tools/compliance/check.sh`：本次相关检查通过；Gate C 被隔离库与仓库既存文档/脚本缺失阻断。
- [ ] `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1`：Gate B/C 未完成，按流程不得进入 Gate D。
- [ ] `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`：同上。
- [x] `cargo build --release --bin monitor`：通过；新鲜缓存复跑 1.87s、exit 0。
- [x] 记录隔离工作区改动前已存在的 2 个 monitor 静态字符串计数失败；不得把它们声明为本次回归或擅自修复。

## PR 证据字段

- `Refs: docs/superpowers/specs/2026-08-22-br135-unsafe-heartbeat-dedup-design.md §4`
- `Data-Redlines: [2.1, 2.4, 2.7, 2.8, 2.10]`
- `OldModules:` 采用真实 health、T-02 与既有 hash-chain；退休 reminder presentation。
- `Threshold-Proof:` 未改 `config/*.toml`；30 分钟为已登记 BR-135 常量。
- `Business-Rules: BR-135, BR-116, BR-225c`
- `Rollback: git revert <commit-sha>` 后重建 release；不删除审计或生产数据。
