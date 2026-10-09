# Goal-first 工具包交付与安装操作

2026-10-09本包及weekly/watchdog已正式安装，monitor和日度工具另按完整发布契约切换，实测结果见[发布记录](../handoffs/2026-10-09-goal-first-release.md)。下方保留独立工具包的安装步骤；其旧monitor基线只适用于本次安装前，后续正式monitor变更造成已知漂移。

新版本包含六个工具（新增只读 `sell_reminder_producer`）与 Python 辅助脚本。本轮全部六个 bin 从同一提交进行普通 release 构建，manifest 的 per-bin source 身份全部一致；旧版三次构建历史保留在原不可变包中。`manifest.json` 固定 source commit、release/toolchain/编译 root、每文件字节数/SHA、原 registry/calendar/contracts、CLI/schema、安装前 monitor 输入树/二进制/任务文件哈希。初始五target、assistant-only输出上限修复、weekly-only registry descriptor修复三次构建分别记录source commit/日志/每bin身份。第三次修复会改变被library借用的loader源码，library编译以日志为准；schema/parser/default bytes未改，复用其余四bin并保留其实际旧身份。旧候选包仅保留作历史证据，本次已安装的最终候选源为 `/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c`。哈希识别内容；不证明 Gateway、历史 PIT、交易资格、送达或当前进程身份。Python 为本机 `/usr/bin/python3` 3.9.6。

目录：`<candidate>/goal-first/<version>/{bin,scripts,resources,launchd,manifest.json,RUNBOOK.md}`；唯一发布位置为 `/Users/zhangzhen/.local/share/stock-analysis-runtime/tools/goal-first/<version>`。目录0700，release binary/launcher0500，普通脚本/文档/plist0600，原始资源0400。原 TOML/CSV bytes 与编译资源绑定；Rust calendar 是编译输入，修改复制 CSV 不会更新 binary。

先检查（默认 plan，不复制、不启动）：

```sh
/usr/bin/python3 -B /absolute/candidate/goal-first/VERSION/scripts/prepare_goal_first_delivery.py --bundle /absolute/candidate/goal-first/VERSION
```

授权安装后，显式追加 `--install` 仅原子发布新版本目录；已有相同 manifest 返回 already_published，不同版本内容拒绝。发布前两次核对完整 monitor `src/config`、Cargo/build/contracts、monitor/bridge binary 与旧 launcher/plist 字节哈希；缺失、symlink、非单链接、不匹配、额外包文件或 drift 都拒绝。无 activation_prepare、无 DB 初始化、无 launchctl、无 pruning、无 .env 加载。发布发生在同一 filesystem，macOS renamex_np(RENAME_EXCL) 原子且禁止覆盖。若 source/config/binary/bound contract 需要改变，拒绝 isolated-tool 范围，另按正式 monitor release 契约做未来 activation、单 owner 切换、DB/lock 保留与一致 rollback。

任务加载是另一步明确运行操作，**2026-10-09已按以下步骤完成并核对自然watchdog间隔**。先核对 host timezone 为 Asia/Shanghai、既有 runtime/logs 为真实目录，确认没有另一个同类 label/周报日程；weekly 只保留 `com.stockanalysis.weekly-outcome-review` Friday20:30，RunAtLoad=false/KeepAlive=false；watchdog `com.stockanalysis.watchdog` StartInterval60、RunAtLoad=false。二者 plist 已绑定安装版本绝对路径，watchdog 仅调用已有 runtime/target/release/monitor 的 `--health --json` 早期只读路径；不启动第二个 monitor。无初始 catch-up/manual 周报调用。

1. 重跑默认 check，记录结果。卸载仅将要替换的 weekly/watchdog label（不存在则不卸载），不触碰 monitor、bridge、daily21:17任务。
2. 检查 manifest.rollback 对应的私有旧 plist/launcher bytes/hash。用 `install -m 600` 将**本版本已校验**的 `launchd/<label>.plist` 替换各自 `~/Library/LaunchAgents/<label>.plist`，逐文件原子临时文件+rename；若检查后又漂移，停止。旧 runtime/bin launcher 不必替换：新 weekly plist 直接指向版本内 launcher。
3. `launchctl load -w` 仅加载这两个确切 label；检查 `launchctl print` 的 ProgramArguments、calendar/interval、Umask，等自然时刻/interval，不额外补跑周报。job 加载与 multi-plist 操作不是全局事务；记录每一步，失败只回退已改的 job。
4. 自然 watchdog interval 后看 `runtime/data/watchdog/latest.json`、每日 self-check、private immutable events，至少两个interval验证 dedup。valid unhealthy/exit1不能叫CLI失败；local事件不证明手机送达或用户阅读。手机适配器已实现；未配置时零网络，服务受理不等于手机送达。

回退：卸载本次已加载 weekly/watchdog；按 manifest.rollback 保存的 bytes/hash 恢复**原有**plist（原不存在则只撤掉本次plist），reload 原有 job，不恢复旧 Desktop monitor plist。不删除新/旧 version目录或 reports/events/archive/state/JSON；保留本次证据。原 runtime/bin launcher 没有改动；其保存 bytes 只用于核对/必要的精确恢复。不得恢复 DB 来撤销 release，不重发 Unknown，不置换 lock/date目录，不自动裁定历史 monetary/Uncertain。保留 pruning 为另行授权的 retention操作。

离线运行必须给显式输入路径。例如：

```sh
PACKAGE=/absolute/candidate/goal-first/VERSION
export PYTHONDONTWRITEBYTECODE=1
"$PACKAGE/bin/assistant_review" --report /absolute/private/review.json --manifest /absolute/private/evidence-manifest.json --registry "$PACKAGE/resources/signal_registry.toml" --as-of 2026-10-08T16:00:00+08:00 --completed-session 2026-10-08 --output /absolute/private/new-comparison.md
"$PACKAGE/bin/sell_reminder_preview" --evidence /absolute/private/sell-observed.json --as-of 2026-10-09T15:05:00+08:00
"$PACKAGE/bin/streak_leader_research" --evidence /absolute/private/streak-observed.json --as-of 2026-10-09T15:05:00+08:00
/usr/bin/python3 -B "$PACKAGE/scripts/rotate_push_log.py" --help
```

需要真实来源才能晋级：SELL lots/fee/expiry与独立close，Streak PIT/可执行历史，scorecard family/version，model端点/价格/tokenizer/账单，人类≥20completed-session价值比较。当前预览不可发送/下单，模型默认0calls。report64MiB/manifest2MiB/output8MiB有限，CLI输入65536/request131072，保持2attempts、18+2s、1500 output tokens、response32000/content16000、cash ceiling/rounded reservation/no refund；较大文件JSON树有额外内存开销，较大输入可能因原cash ceiling拒绝。

## 新模拟账户与自动复核

新版 weekly launcher 显式读取生产根私有 `.env` 的 `PAPER_LEDGER_ACCOUNT_BINDING`，只向子进程转交这个精确 JSON，其他凭证不加载。SQLite mode=ro/query_only 一致 backup 和原数据库 path/device/inode 证明由 wrapper 冻结；CLI 对照不可变期初导入回执，错误绑定、缺失来源、复制数据库冒充原目标或未来账本截面均不可用，不回退旧 raw ledger。

周报新增 `paper_account`（截至本次观察的新账户）和 `legacy_verified_paper`（期初前旧历史）两段。新期初以来权益、费用、PnL 与旧争议分开；持久价格保留原观察时间，缺少此前收盘基线/当日完整价格时当日盈亏保持空值。当前账户不冒充历史周末截面，模拟费用不冒充券商结算费。

JSON 与 Markdown 周报均成功后，同一 run 自动调用一次 `run-weekly-assistant-review.py`，生成 `assistant/comparison.json`、`comparison.md`、`status.json`。三个对照臂共用一个账户观察、报告、manifest、registry 和时间；默认零模型调用，不加载模型凭证。后续结果仍缺真实 family/version/PIT join 与20个完成交易日的人类试用，不能把模板生成称为模型试验有效。失败保留周报和部分产物，明示阶段与非零退出，不重跑。

## 可选手机告警

可变凭证只使用 `/Users/zhangzhen/.local/share/stock-analysis-runtime/data/private_config/watchdog-mobile.json`，父目录0700、文件0600。不要放在 `runtime/config/`，该目录是交易 activation 的固定校验输入。模板仅在包内 `resources/watchdog-mobile.example.json`，不会复制到启用路径。缺文件=unconfigured/零网络；坏权限、symlink或坏JSON=invalid。具体配置见包内 `resources/watchdog-mobile.md`。

适配器仅传白名单运维字段，不传持仓、原health、路径、凭证或原异常；本地事件与状态先持久化，再有限超时发送。确定连接前失败最多三次退避；发送结果不明保留Unknown，不跨重启或同事故小时提醒盲重发。恢复后新事故以 generation 区分。HTTP/JSON 受理证明只授予服务受理，不授予手机显示或用户阅读。

## 盘后观察 producer

`sell_reminder_producer` 增加15:02–15:04准备窗口、≤120秒读取预算及15:03–15:30消费窗口。既已准备的 candidate 在进入 durable owner/恢复物理尝试前再查账户30秒时效、时间与过期。公开原始观察输入只能输出NotReady，零发送；独立日线收盘资格不能代替真实可卖批次/预留/费用/证券状态与数量合同。本包没有使用paper lots冒充实盘批次，也没有复用T-21模拟售出确认卡当真人卖出建议。来源和独立counted tuple未齐之前只交付可测试观察入口，不宣称真人提醒正式接线或两分钟SLI已验收。

本轮只发布独立版本包，monitor/bridge 的已校验源码、config、二进制和旧activation均作为包安装前后前置条件保持一致；不触发其重启，不重建期初，不恢复数据库。未来修改这些主链输入时再按完整上线契约构建与重发activation。
