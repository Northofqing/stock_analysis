# monitor 的 launchd / Desktop TCC 故障交接（2026-09-28）

## 当前状态（11:55 CST）

Desktop 外运行根切换已完成。`com.northofqing.grpc-market-server`（PID 56417）与 `com.stockanalysis.monitor`（PID 59028）均由 `gui/501` launchd 管理，`runs=1`，程序和工作目录都在 `/Users/zhangzhen/.local/share/stock-analysis-runtime`。本地桥接于 11:54:58 监听 `127.0.0.1:18082`，非 fixture 模式；monitor 于 11:53:59 完成新根主库初始化，11:55:04 记录“gRPC 桥已连接”。旧 Desktop `stock_analysis.db` 与 `durable_delivery.sqlite3` 已无进程占用。当前运行版本是新根构建出的 monitor SHA-256 `2704c32f505f168c6f61dacce879c9db282df9eb3990622f3d8d96b36eeb6219`；后续仓库源码修改尚未部署。

停旧实例后，完整 `rsync -a --delete --checksum data/` 至新根退出 0；新根主库和持久投递库的 `PRAGMA quick_check` 均为 `ok`。从新根生成的 activation 已在 11:25 CST 生效并提交为 `57f53aa8`，预期配置哈希为 `ba4087dbd9d76d377a3804cd56738e91dd40e177dc131c3e5056650be660b158`。本地桥接只读探针的 Health/Capabilities 通过，午间 RealtimeQuotes 返回 `no_verified_batch`；不能据此宣称行情数据已完整恢复，开盘后仍需复验。上游 VM 的 R-08 `Planned` 已验证，`Confirmed` 仍不可用；Eastmoney GlobalNews 的 `source_precondition_failed` 已定位为 `bank.eastmoney.com` 文章域名未列入资格集合，详见[上游交接](../../grpc_handoffs/2026-09-28-eastmoney-globalnews-source-precondition-vm-handoff.md)。

启动耗时的原因已定位：主库 `data_acquisition_audit` 与链各约 343 万行，数据库初始化先在采集审计模块、再在 benchmark manifest 模块各验证一次完整哈希链。本次桥接启动约 9 分钟，monitor 约 4 分钟；不能只以 launchd PID 存在判定就绪。monitor 比桥接先完成初始化期间，部分 LocalBridge 请求按 `no_verified_batch` fail closed 并留有失败记录；桥接就绪后 monitor 自动重连。后续应优化重复全量校验的启动成本并核对这些欠账的正常重试，不应绕过链完整性检查。

## 故障与根因（切换前）

2026-09-28 09:12 CST 的临时单实例是 Terminal 启动的新版 monitor（当时 PID 23247）；当时 `gui/501/com.stockanalysis.monitor` 已 bootout。该实例已于盘中午休停下，并由上述新运行根的 launchd 实例接替。

原 plist 位于 `~/Library/LaunchAgents/com.stockanalysis.monitor.plist`，`ProgramArguments`、`WorkingDirectory` 和 stdout/stderr 均指向 `~/Desktop/Quant/stock_analysis`，`KeepAlive=true`、`ThrottleInterval=60`。多次 launchd 启动旧、新二进制都在进入 `main` 前停住；`/private/tmp/monitor_2026-09-28_091239_n503.sample.txt` 中 PID 22805 的 668 次采样全部停在 dyld `__open`，物理占用仅 56 KiB。独立 `/bin/sh` LaunchAgent 探针报 `getcwd: cannot access parent directories: Operation not permitted`。这不是 gRPC 或 monitor 业务循环错误。

`/usr/bin/log show --style compact --info --start '2026-09-28 09:11:00' --end '2026-09-28 09:13:00' --predicate 'process == "tccd" AND eventMessage CONTAINS[c] "/Users/zhangzhen/Desktop/Quant/stock_analysis/target/release/monitor"'` 的关键证据：

- 09:12:09.614：请求 `kTCCServiceSystemPolicyDesktopFolder`，访问者为 launchd 子进程 PID 22805。
- 09:12:09.611、09:12:09.838：`SecStaticCodeCheckValidity ... status: -67050`，随后 `Failed to match existing code requirement`；日志列出两个 `cdhash`，但不据此推断哪个属于当前文件。
- 09:12:09.839：`Auth Right: Unknown (None)`，随后 `Delaying prompt`。09:07 重启用户 tccd 后，普通 Desktop 访问恢复，此 launchd 请求仍重现。

本机 `codesign -dv` 显示 monitor 未签名，`security find-identity -p codesigning -v` 显示 0 个有效身份。结论是当前 launchd 进程没有匹配的 Desktop TCC 授权，后台提示未完成；普通 POSIX 文件权限和文件存在性不足以解决它。Apple 文档确认 [Desktop 属于受保护文件夹](https://support.apple.com/en-gb/guide/mac-help/-mchld5a35146/mac)，[代码签名要求用于判断跨版本代码身份](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)。

## 盘后迁移到 Desktop 外的运行根

推荐真实目录 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，不得用指回 Desktop 的符号链接。先保留旧 plist、二进制及数据快照，并记录 SHA-256。仓库整体约 135 GiB；运行所需 `data/` 约 22 GiB，不能把整个仓库直接视作部署包。

1. 在 Desktop 外准备同一版本的构建源和运行根：`src/`、`config/`、`Cargo.toml`、`Cargo.lock`、`build.rs`、`contracts/`、Cargo 构建所需文件、经 manifest 校验的公开 `client-bundle/` 文件、release `monitor`、`.env`、`data/`、`reports/`、新 `logs/`。`config/selection/selection_activation.v1.json` 必须随同版配置复制。根和凭据目录限本机用户访问，`.env` 与密钥文件为 0600；不把凭据提交到 Git。初次迁移先完整保留 `data/` 的生产 DB、WAL、审计、投递、锁和证据，不凭文件名猜测哪些历史记录可删。
2. 从 Desktop 外的源码目录构建，并在构建时设置 `STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT=/Users/zhangzhen/.local/share/stock-analysis-runtime`。`src/production_root.rs` 将生产 DB、审计和锁绑定到这个**编译期**绝对路径；改运行时环境或 plist 的 `WorkingDirectory` 不会改该身份。构建目录也应在 Desktop 外，因为 `root_for_mode(true)` 使用编译时的 `CARGO_MANIFEST_DIR`，否则 shadow `--test` 仍会访问 Desktop。源码与配置应保持真实目录、同版字节；activation 会扫描 `src/`、`config/`、Cargo 清单和 `build.rs`。
3. 新 `.env` 中的 `GRPC_MARKET_CLIENT_BUNDLE` 当前指向 Desktop：将其认证文件完整移至受限的非 Desktop 目录，更新路径并验证 mTLS/Health。MagicLaw 的默认 binary/home 也在 `$HOME/Desktop/magiclaw`（`src/bin/monitor/notify.rs`）；盘后同时准备非 Desktop 的 MagicLaw binary/home 与凭据，设置并验证 `MAGICLAW_BIN`、`MAGICLAW_HOME`，否则推送服务重启时仍会触碰受保护路径。`WECHAT_SEND_SCRIPT` 为死配置，不把它当作实际推送路径。
4. 旧 monitor 运行期间可以预拷静态文件与数据，但预拷的活动 SQLite 文件不能作为一致快照。盘后先停止旧 PID，并确认它及相关数据库写者退出，再最终同步 `data/` 和 WAL/SHM 侧文件；使用 SQLite 备份/完整性检查核实数据库，核对审计水位、文件身份与配置摘要。新旧运行根的单实例锁是不同文件，**不能依赖锁防止两实例重叠**。
5. 正式切换前，在非 Desktop 构建的制品上用独立标签、无 KeepAlive 的 shadow LaunchAgent 运行 `monitor --test --push-dry-run`，stdout/stderr 放在新根，确认 dyld 已进入程序、测试命名空间隔离、生产目录无写入。关闭 shadow job 后，确认旧 PID 已退出并完成最终同步，安装指向新根 binary、工作目录和日志路径的正式 plist；按仓库部署规则 `launchctl load -w`，只启动一个生产实例。检查新 PID、`cwd`、二进制 SHA、生产 DB/锁/审计根、启动对账、gRPC Health 和实际投递回执；不以进程存在作为完成验收。

**切换前路径审查。** 已核对的 `selection/activation_gate.rs:165`、`selection/audit.rs:1800` 和 `monitor/notify.rs:3719` 的 `CARGO_MANIFEST_DIR` 使用位于测试代码；`durable_delivery/model.rs:165`、`event/dispatcher.rs:323,1156` 和 `monitor/br196_transport.rs:193` 在可编译运行路径中仍直接使用构建根，分别用于测试命名空间或非生产验收。源码、配置、`.env`、bundle、MagicLaw 与派生进程工作目录均迁至 Desktop 外；新根 shadow dry-run 与生产实例已实测进入程序并访问新根数据库。后续新增生产路径仍须检查构建根和绝对路径，不能只凭 production_root 推断。

## 回退边界

若新 launchd 无法启动，先卸载新 job 并确认其 PID 退出。优先用**同一个新运行根和同一份新数据**的 Terminal 启动方式恢复单实例，这样保留切换后已经提交的状态。只有确认新根没有生产写入，或已完成人工数据/投递对账，才能切回旧根的数据库与旧二进制；不得直接覆盖数据库、重放不确定投递或同时启动两根的 monitor。原 Terminal 路径是当前已实测可启动的临时回退路径，原 Desktop launchd plist 仍受 TCC 阻塞，不能把重新 load 旧 plist 视为有效回退。

## 10:53 CST 预检记录（随后已完成切换）

- 新真实目录为 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，权限 0700；原样复制同版 `src/config/contracts/Cargo` 输入、公开 `client-bundle`、本机 `.env`、MagicLaw 与报告。新 `.env` 只更新 gRPC bundle、MagicLaw bin/home 路径并移除源码不读取的 `WECHAT_SEND_SCRIPT`；两份 `.env` 及私钥为 0600。公开 bundle 清单 9/9 匹配，MagicLaw 二进制与旧版 SHA-256 同为 `16024d290ee302ffe1872db1e8b26e95d0b5f43f451510a1da38910300fcf213`。
- 从新目录 `cargo build --locked --offline --release -j 2 --bin monitor --bin grpc_bundle_probe` 成功；新 monitor SHA-256 `2704c32f505f168c6f61dacce879c9db282df9eb3990622f3d8d96b36eeb6219`，probe SHA-256 `c2a3f86b3e5749036e2752ea01cceb89f1eff064e2aa7b0a631e0a60ba6cc2d9`。另构建 `selection_activation_prepare` 成功。构建期源码与本仓 `src/` 字节一致；待停旧后生成新 activation，不能使用原文件的旧 hash。
- 新 probe 的 R-08 真实查询退出 0：Health identity matched，2026-09 四条 `Planned` 均 `ADMITTED`，`confirmed_delivery=false`。`--opening` 退出 0，九条静态路由中八条 ready；Eastmoney `Unadmitted` 的真实原因现为 `source_precondition_failed`，本仓诊断修复见 `d9f80181`，上游故障未关闭。
- 临时 shadow LaunchAgent 使用新二进制、新工作目录与日志、`--test --push-dry-run`、无 KeepAlive；已运行一次并以 exit code 0 退出。日志确认 `bound root mode=test` 为新目录、核心库在独立 `TEST_CODE` 临时目录、`external_process_attempted=0` 和 `receipt_audit_appended=0`，随后已 bootout。证明 launchd 可从 Desktop 外进入程序并完成隔离 dry-run；不代表生产主库已迁移。
- 限速预拷在读取活动 `stock_analysis.db` 时返回 `unexpected end of file`，目标未生成该主库；跳过主库及其 WAL/SHM 后其余数据预拷退出 0。**目标 `data/` 尚非一致生产快照**。正式切换必须在旧 PID 退出、其他写者核清后完整无排除同步并检查数据库；不得因 shadow 成功提前启动第二个生产进程。
