# Goal-first 工具包交付与安装操作

2026-10-09本包及weekly/watchdog已正式安装，monitor和日度工具另按完整发布契约切换，实测结果见[发布记录](../handoffs/2026-10-09-goal-first-release.md)。下方保留独立工具包的安装步骤；其旧monitor基线只适用于本次安装前，后续正式monitor变更造成已知漂移。

本包只包含五个离线工具与 Python 辅助脚本。`manifest.json` 固定 source commit、release/toolchain/编译 root、每文件字节数/SHA、原 registry/calendar/contracts、CLI/schema、安装前 monitor 输入树/二进制/任务文件哈希。初始五target、assistant-only输出上限修复、weekly-only registry descriptor修复三次构建分别记录source commit/日志/每bin身份。第三次修复会改变被library借用的loader源码，library编译以日志为准；schema/parser/default bytes未改，复用其余四bin并保留其实际旧身份。旧候选包仅保留作历史证据，本次已安装的最终候选源为 `/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c`。哈希识别内容；不证明 Gateway、历史 PIT、交易资格、送达或当前进程身份。Python 为本机 `/usr/bin/python3` 3.9.6。

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
4. 自然 watchdog interval 后看 `runtime/data/watchdog/latest.json`、每日 self-check、private immutable events，至少两个interval验证 dedup。valid unhealthy/exit1不能叫CLI失败；local事件不证明手机送达或用户阅读。尚无独立 mobile channel。

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
