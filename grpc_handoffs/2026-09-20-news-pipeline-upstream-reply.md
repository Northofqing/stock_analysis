# 新闻零推送交接回复: 上游侧核实结论 (2026-09-20, 修订版 2 — 含实测量)
> **已有修订版 3 追加于文末（2026-09-21 00:05）：** 上游时钟校正已执行并在收敛中；
> 上游 runtime 曾于 2026-09-20 22:33~23:58 停机 85 分钟，根因为 autostart 竞态缺陷（已修复）。

对 `2026-09-20-news-pipeline-invalid-observation-time.md` 的核实回复。
修订版 2 补上了直接测量, 替换了修订版 1 的推断式结论。

## 结论

1. **不是上游代码缺陷。** 上游发出的 v2 证据自洽且符合契约。
2. **但上游部署主机有时钟缺陷, 这是可实测的事实:**
   VM `10.211.55.3` / `DESKTOP-IMSDKM0`(运行 magic-market-grpc-server 的主机)
   系统时钟**固定超前真实时间约 0.70 s**, 且 Windows 时间服务从未同步过。
3. gate 本身是下游代码, 且它对"上游盖章的时刻"与"本机 now"做**零容差**比较,
   这是放大器: 0.70 s 偏差 + 零容差 = 每个 tick 稳定拒绝最后返回的 provider。
4. 两侧都要动。VM 校时是必须项; 下游给容差是消除该类故障的根治项。

## 决定性证据 (本次实测, 四个独立 NTP 源)

```
VM (10.211.55.3)  w32tm /stripchart /computer:<ntp> /samples:2 /dataonly
  ntp.aliyun.com    +00.6940266s   +00.6931156s
  ntp.tencent.com   +00.7075659s   +00.6996276s
  pool.ntp.org      +00.6990743s   +00.7083375s
  time.windows.com  +00.9961830s   +00.7152658s   (首采样抖动, 次采样一致)

VM (10.211.55.3)  w32tm /query /status
  来源:            Local CMOS Clock
  Leap 指示符:     3 (未同步)
  上次成功同步时间: 未指定
```

即: VM 自由运行, 从未与任何时间源同步, 当前超前真实时间 ~0.70 s。
这不是"日期卡住"级别的故障, 所以没有明显症状, 但足以让跨主机时刻比较失配。

## 触发机制 (逐行可复核)

gate 判定链 `news_aggregator_init.rs:491-513`, 实际命中的是第 3 条:

| 分支 | 判定 | 实际是否命中 |
|---|---|---|
| 1 | `occurred_at > now` → `future_publication` | 否 |
| 2 | `occurred_at.date_naive() != now.date_naive()` → `publication_date_not_current` | 否 |
| 3 | `fetched_at > now \|\| fetched_at < occurred_at` → `invalid_observation_time` | **是** |

- `fetched_at` = 上游盖章的 `observed_at`(绝对时刻)
- `now` = 下游 `chrono::Local::now()`, 取自 `main.rs:7834`, 位于抓取(`main.rs:7784`)**之后**
- `fetched_at < occurred_at` 一侧被双向排除: 上游 gRPC 边界原子强制
  `source_instant == published_instant <= observed_instant`
  (`magic-market-composition/src/grpc_production.rs`), 下游 `feed.rs:78` 还有 BR-166 兜底

⇒ 只剩 `fetched_at > now`: **上游时刻 > 下游当前时刻**。
只有当两侧时钟偏差 Δ(上游超前)大于"抓取完成 → reserve"的耗时 Δt 时才成立。
实测 Δ ≈ 0.70 s, 而 Δt 通常 < 0.7 s ⇒ 每个 tick 里最晚返回的那个 provider
的整批记录被拒。

## 对照证据: 该机制能完整解释观测到的形态

| tick (下游日志) | 四源 observed_at 相对顺序 | 拒绝数 | 解释 |
|---|---|---|---|
| 22:13:14 | 四源集中在 .3~.96 s, ThePaper 最晚 | 12 | 只有 ThePaper 落在 0.70 s 窗口内 |
| 22:28:08 | CLS 最先, EM/JN/TP 挤在最后 0.15 s | 47 | 20+15+12, 三个源都落进窗口 |
| 20:40:38 | 批次 observed_at = 20:39:30, **比日志早 68 s** | **0** | 上游返回了缓存批次, 观察时刻远离 now |
| 19:25 / 20:09 / 18:46 | 三源几乎同时完成 | 61 / 59 / 62 | 四源几乎全部落进窗口 |

**20:40:38 那个 0 拒绝的 tick 是决定性对照**: 同一份代码、同样的源、同样的
下游, 只因为批次观察时刻旧了 68 s 就全部放行。这排除了"某个 provider 数据异常"
和"某条记录格式异常"这类解释 —— 拒绝集合不是由数据决定的, 是由
`观察时刻` 与 `reserve 时刻` 的距离决定的。

## 已排除的假设 (避免重复劳动)

1. **上游时间戳格式/时区 bug** — 排除。五个 provider 的 `observed_at` 都是纯 epoch
   瞬时值(`cls:577` / `thepaper:510` / `jin10:922` / `sina news:863`;
   eastmoney `lib.rs:463` 为 `unix-ms:`), 与任何时区无关; `published_at` 全部显式
   `+08:00`。下游 `data_gateway/evidence_time.rs:9-60` 覆盖上述全部编码
   (epoch 秒 / 1-9 位小数 / `unix-ms:` / RFC3339), 9 位纳秒在其接受范围内。
2. **上游证据不变式被破坏** — 排除。`grpc_production.rs` 在 gRPC 边界原子校验
   `source_instant == published_instant <= observed_instant`, 违反即整批
   `invalid_evidence` 拒绝, 不会输出半有效批次。
3. **下游"抓取前快照 now"** — 排除。`reserve_now`(`main.rs:7834`) 在
   抓取(`7784`)之后, 且 `news_flash_gate.reserve_from_authority` 是
   `NewsFlashGate` 唯一的生产调用点(全仓 grep 确认)。
4. **下游二进制陈旧** — 排除。`target/release/monitor` mtime 09-20 21:08:55,
   晚于全部相关源文件 (`feed.rs`/`raw_v2.rs` 08-30, `global_news.rs` 16:57,
   `news_aggregator_init.rs` 17:11, `main.rs` 19:16)。
5. **用下游日志时间戳反推时钟偏差** — 不可行, 别再走这条路。日志写入延迟在
   0~1.5 s 之间抖动, 且 20:40 处出现过 68 s 的缓存批次, 逐行差值无法把
   真实偏差和写入延迟分离。要测就直接测: `w32tm /stripchart` 或两侧取绝对时钟。

## 修复

### 上游侧 (必须, 环境)

```powershell
w32tm /config /manualpeerlist:"ntp.aliyun.com,0x8 time.windows.com,0x8" /syncfromflags:manual /update
net stop w32time
net start w32time
w32tm /resync
w32tm /stripchart /computer:ntp.aliyun.com /samples:3 /dataonly   # 应回到 ±0.0xs
```

注意: `/resync` 会把 VM 时钟**回拨约 0.70 s**。同时建议在 Parallels 里打开该 VM 的
"Time synchronization"。校正后 gate 自动放行, 无需改动任何一侧代码 ——
原文档 "修复上游后无需改代码" 的判断在这一点上是成立的。

### 下游侧 (建议, 根治)

`news_aggregator_init.rs:507-510` 给 `observed_at` 一个显式容差, 例如
`item.fetched_at > now + chrono::Duration::seconds(5)`。
**跨主机盖章的时刻本来就不该和本机 `now` 做零容差比较** —— 上游时钟再准,
NTP 残差、VM 挂起恢复、日志写入抖动都会重新制造这个故障。保留 5 s 容差仍然
能挡住真正的"未来时刻"。

## 附带说明: 2026-09-20-remaining-source-boundaries.md

四项均非上游问题, 该文档自己的判断("无本地代码可修")成立。
`register_trade_event_source` / `TradeEventSource` / `fetch_pending_trade_events` /
`push_templates.rs` / `dispatch_candidate_board` / VirtualWatch / PaperReview /
AccountMode 在 magic-market-data-rs 中零命中, 全部是下游符号。
四项都是外部数据源/券商 feed 的集成前置。

---

# 修订版 3 追加 (2026-09-21 00:05) — 处置进展与新增证据

修订版 2 的两条结论不变。本段追加 2026-09-20 23:40 至 2026-09-21 00:05 之间实际发生的事：
上游时钟已校正（收敛中）、上游 runtime 曾停机 85 分钟并已恢复、以及一个已定位并修复的上游
autostart 缺陷。

## 1. 上游时钟校正已执行（修订版 2 第 4 条的落地结果）

```
w32tm /config /manualpeerlist:"ntp.aliyun.com,0x8 time.windows.com,0x8 ntp.tencent.com,0x8" /syncfromflags:manual /update
Restart-Service w32time -Force
w32tm /resync /force     -> 成功，源=ntp.tencent.com，Leap=0，层次=3
```

偏差随时间收敛（实测 `w32tm /stripchart /computer:ntp.aliyun.com /dataonly`）：

| 时刻 | 偏差 |
|---|---|
| 23:40:21（校时前） | +0.705 s |
| 23:45:16 | +0.666 s |
| 00:00:53 | **+0.505 s** |

w32time 对小于 `MaxAllowedPhaseOffset`（默认 300 s）的偏差不做跳变，只做**缓慢斜降**，
实测斜率约 167 ppm，外推约在 **2026-09-21 00:52** 附近归零。按修订版 2 的判断，届时
无需改动任何一侧代码，gate 应自行放行——这正是需要验证的那一点。

截至本段写下时（00:05），**校时后还没跑过一轮 news tick**（上一轮拒绝在 22:28:09，
下一轮约 00:14），所以"时钟归零后 gate 放行"仍是待验证结论，不是既成事实。

仍未验证：**Mac 侧自身时钟**。`net time \\Mac` 在本机不可用（System error 1707），
需要在 Mac 上直接跑 `sntp -d time.apple.com`（或 `sudo sntp -sS time.apple.com`）。
若 Mac 也偏离真实时间，相对偏差可能在 VM 归零后依然存在。

## 2. 上游 runtime 曾停机 85 分钟（2026-09-20 22:33 ~ 23:58）

这是与时钟无关的第二个独立故障，下游在此期间完全收不到数据：

- VM 开机时间：`2026-09-20 22:33:39`
- gRPC server 实际启动：`2026-09-20 23:58:35`（人工拉起，非自动）
- 下游 23:45:42~48 四源全部 `[BR-244] NewsFlash source unavailable ... diagnostic_code=provider_error_mapping_missing`
- 下游 23:59:46 四源恢复：`outcome=available ... rejected=0`，并打出
  `provider state changed previous=unavailable current=available`

也就是说，22:28:09 那批 `invalid_observation_time` 拒绝之后，上游在 22:33 重启，
但**没能自己起来**。

## 3. 根因：上游 autostart 竞态缺陷（已修复）

`tools/runtime/windows-autostart.ps1` 启动 TDX 终端看门狗后，只用固定 500 ms `Sleep`
等待看门狗写 pid 文件：

```powershell
Start-Process -FilePath $powershell -ArgumentList $watchdogArguments ... | Out-Null
[Threading.Thread]::Sleep(500)
if (-not [IO.File]::Exists($watchdogPidPath)) {
    throw "TDX terminal watchdog did not remain running after startup"
}
```

而看门狗（`tools/runtime/tdx-terminal-watchdog.ps1`）要先冷启动 PowerShell、强制导入两个
内置模块、再对 `TdxW.exe` 做 SHA-256 准入校验，之后才写 pid 文件。实测：

- 22:37:11 看门狗进程启动
- 22:37:12.251 autostart 检查落空 → 抛错
- 22:37:15 看门狗才写下 pid

因为 `Start-TdxTerminalWatchdog` 在 autostart 中排在 runtime 启动**之前**，这个 throw
直接跳过了后面的 `start.ps1`，于是**每次重启后 gRPC server 都不会自动起来**。同一报错在
09-13、09-16、09-18（两次）、09-19、09-20 共出现 6 次。

修法：固定 500 ms 等待改为 **20 秒有界轮询**（250 ms 间隔），失败条件与语义不变。
已空跑验证：语法解析通过、退出码 0、日志输出 `TDX terminal watchdog already running`
与 `runtime already running`。

## 4. 下游建议不变（本次事件再次验证）

修订版 2 的下游建议仍然成立且仍然建议做：跨主机盖章的 `observed_at` 不应与本机 `now`
做零容差比较，应给 5 s 容差。本次事件说明理由不止时钟一条——上游重启、断供、批次缓存
都会让这个比较产生假阳性，时钟只是其中一个诱因。

## 5. 新增的实测排除项

- **不是"上游进程崩溃"**：`grpc-server.stderr.log` 在重启前没有新的写入，也没有
  panic 或退出记录；停机是随整机重启发生的。
- **不是"上游批次为空"**：恢复后首批 `accepted=20 / 20 / 20 / 12`，`rejected=0`。
- **`net time \\Mac` 不可用**（System error 1707），不要再拿它测 Mac 时钟；
  用 `w32tm /stripchart`（Windows 侧）或 `sntp`（Mac 侧）。
- **不要用下游日志时间戳反推时钟偏差**（修订版 2 已列，此处重复）：日志写入延迟在
  0~1.5 s 抖动，且出现过 68 s 的缓存批次。
