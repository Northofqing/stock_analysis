# InstrumentNews 置顶行修复与后续交接 — 2026-09-23

本文交接 Sina 个股新闻（InstrumentNews）本轮两个缺陷的**根因、修复、部署、实测**，以及
**仍未闭合的三项**。前置文档是同目录两份：
`2026-09-23-instrument-news-pagination-certificate-upstream-reply.md`（第一版，第 5 节归因
已被更正）与 `2026-09-23-instrument-news-pinned-placement-correction.md`（更正版，本文不重复
其内容）。

## 一句话状态

两个缺陷都已修复、已部署、已由下游自己的 `[盘后]` 周期验证通过（2026-09-23 21:02:15，五只
持仓全部 `status=available`，002131 首次进入）。**InstrumentNews 这一路现在是干净的**；剩下
三项不属于本缺陷。

## 1. 本轮修的两个缺陷

| # | 缺陷 | 提交 | 影响面 |
| --- | --- | --- | --- |
| 1 | `execute_instrument_news` 一律按 200 向 provider 请求，且分页证书用「本页**最新**时间戳」判定，五页最多证明 160 条 | `483cff6` | 任何调用方 limit 都失败 |
| 2 | Sina 在部分证券第 1 页头部钉了一行 `[置顶]` 推广，由来源按「置顶」而非发布时间放置，制造假逆序并污染本页时间极值 | `2cd8665` | 只有 `sz002131` 一只持续失败 |

缺陷 2 的实测：`sz002131` 第 1 页唯一一处逆序来自置顶行 `2026-09-23 00:52`
（`https://wq.finance.sina.com.cn/company/detail/1017/1`），排在 `16:39` 起的新闻行之前；
第 2–5 页无逆序；同一时刻抽查的另外 6 只（`sh600519`/`sh600396`/`sz000001`/`sh600667`/
`sh600703`/`sz002421`）都不带置顶行 —— 这正是「5 只持仓里只有它失败」的原因。

修复方式不是放宽校验：置顶行**仍按 BR-025 全量校验**（身份、MIME、发布时间、未来时间），
校验后才整行排除（记录集、本页逆序校验、本页时间极值）；未带 `[置顶]` 前缀的真实逆序仍然
显式失败。BR-025 与 `docs/integrations/sina-web.md` 已写入该条与实测证据。

## 2. 线上状态（接手时的起点）

```text
提交          main @ 2cd8665（483cff6 → 2cd8665，均已推送）
二进制        target\runtime\bin\magic-market-grpc-server.exe
sha256        13E5C9F986787A7BB3A872FDAC6ADBB34A8573D73AE1D4AAACAF4E746140E2A3
部署时间      2026-09-23 20:52:29（stop → copy → hash 比对一致 → start）
日志归档      本次重启归档生效：grpc-server.stderr.log -> .20260923-205229
```

发布门禁本轮全绿：`cargo fmt --all -- --check`、`cargo test --workspace --all-targets
--locked --offline`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo doc --workspace --no-deps`、`tools/compliance/check.sh`（5 项 registry 均 passed）、
`tools/docs/check_links.sh`。

## 3. 已完成的实测

| 观测 | 结果 |
| --- | --- |
| 回归测试（旧证书 / 旧解析） | 用 production 同一条报错失败，已复现确认；修复后通过（20 项全通过） |
| 002131 @ limit=100, 2026-08-24..2026-09-23（下游同形状） | `complete: true`，`pages-3`，55 条严格倒序，置顶行 0 条 |
| 002131 @ limit=100 当日 | `complete: true`，`pages-2`，6 条 |
| 600396 @ 199 / 600519 @ 196 | `complete: true`，`pages-5`（缺陷 1 回归） |
| 下游 `[盘后]` 20:32:15（部署前） | 002131 仍 `instrument-news page is not newest-first` |
| 下游 `[盘后]` 21:02:15（部署后首个周期） | 五只全部 available；`002131 ... 拉 55 条, DB 写 55 条, 失败 0 条` |

## 4. 未闭合项

### 4.1 Consensus 全线失败（**已归因 2026-09-23**，见 `2026-09-23-consensus-earnings-batch-attribution.md`）

下游日志里与 InstrumentNews 无关的第二条失败线，规模远大于本轮修的那条：

```text
[21:00:47 WARN] [v17_sources][BR-115] 688690 earnings batch rejected: financials=ok;
  consensus=GrpcBridge data gateway failed reason_code=no_current_reports
  provider=Some(Eastmoney) retryable=false: gRPC Consensus 查询失败: 服务端内部错误
```

实测范围（`monitor-launchd-20260916.stderr.log` tail 12 MB）：**638 条，2026-09-23
15:01:37 → 21:00:53，覆盖 25+ 个代码**（`000021`…`688690`，含 `002131`/`600396`/`600667`
/`600703` 等本次持仓）。`reason_code` 以 `no_current_reports` 为主，另有 `invalid_evidence`
（如 `605178`）。每次 `financials=ok`，只有 consensus 一半被拒。

已知与未知：

- `no_current_reports` **不在本仓任何位置出现**（全仓 grep 无命中），所以它不是我们的
  reason code —— 要么是我们 trailer 的转发，要么是下游自己的分类。
- 本仓服务端 stderr **不为这类调用写任何记录**：重启前归档只有 3059 字节，内容全部是 6 条
  `instrument_news` 的 `service_failure`，没有一条 Consensus。也就是说，这条失败线在我们
  这边**没有留痕**，无法从日志反推。
- 下游索引里的 **GD-004** 已覆盖「Consensus 转换固定丢最近报告、日期和目标价」，所以本条是
  **运行证据**，不是新结论。

建议的下一步（下次接手的第一件事）：固定 provider 调一次 Consensus（用 `688690` 或
`002131`），解 `magic-error-detail-bin` trailer，先分清是「我们返回 Internal」还是「我们返回
OK 空批次、下游把它归类为不可用」。这两者的归属方完全不同。

> **2026-09-23 已闭合：归属方是下游本机路由，不是我们的 VM。** `GrpcBridge` = `ContractProfile::LocalBridgeV1`
> → `GRPC_MARKET_ADDR` 默认 `http://127.0.0.1:18082`（你们本机旧 `grpc_market_server`）；我们的服务端
> Consensus 实测正常（`688690` → ADMITTED / Tonghuashun / complete / 无 trailer），且把你们 LocalBridgeV1
> 信封发给我们只会得到 `InvalidArgument`/`invalid_request`，不会是 Internal。`no_current_reports` 由你们
> 自己已删除的 `120b90dc:src/data_gateway/consensus.rs`（Eastmoney 研报 180 天窗口）产生，`retryable=false`。
> 完整证据链、计数分解（1647 行 / 1285+211+136+31）与两个可选下一步见
> `grpc_handoffs/share/2026-09-23-consensus-earnings-batch-attribution.md`。

### 4.2 供给上限（规则要求，不是缺陷）

Sina 每页实际供给（直接抓页计数）：`sh600396` 五页 40×5，成记录 199；`sh600519` 为
39+40×4，distinct URL 197。limit ≥199 时五页无法证明，按 BR-025 必须显式失败，不允许把
不足的条数当「完整」交付。**建议下游客端 limit ≤195**；下游当前 limit=100，不受影响。

### 4.3 未做的选项

放宽 BR-025 的「最多五页」上限（第 6 页仍有内容）。这是业务规则变更，按仓内工程规则需要
Gate A 设计加 provider admission 证据，本轮没有擅自改。

## 5. 复现与验证入口

探针脚本都在 `target\runtime\probe\`（scratch 目录，非交付物）：

```powershell
# 复现下游那次调用（limit=100 + 30 天 range）
target\runtime\probe\call-news.ps1 -Code 002131 -Exchange Shenzhen -Limit 100 `
  -Start (Get-Date).AddDays(-30).ToString('yyyy-MM-dd') -End (Get-Date).ToString('yyyy-MM-dd') `
  -RequestId claude-news-1 -Out target\runtime\probe\news-002131.out.txt
target\runtime\probe\decode-news.ps1 -Path target\runtime\probe\news-002131.out.txt   # 逐条列出并标记置顶行
# 来源侧：置顶行、逐页逆序、页头
target\runtime\probe\page-pinned.ps1     # 多只证券扫 [置顶] 行与逆序数
target\runtime\probe\page-order.ps1 -Symbol sz002131
target\runtime\probe\page-head.ps1  -Symbol sz002131
# 下游日志（148 MB，必须走 tail-scan 而不是 Get-Content）
target\runtime\probe\tail-scan.ps1 -Path '\\Mac\Home\Desktop\Quant\stock_analysis\logs\monitor-launchd-20260916.stderr.log' -Megabytes 3 -Pattern '002131'
```

两个环境坑（都已踩过）：

1. **发布门禁不能用 Bash 工具跑。** 本机 `rg` 不在 PATH，且 Bash 工具的 `bash` 是不可用的
   WSL stub；`bash tools/compliance/check.sh` 会以退出码 1 报 `missing workspace member:
   crates/magic-market-core` —— 那是缺工具，不是仓库缺陷。可用做法是写一个 `.cmd`，把 VS Code
   自带的 rg 目录（`...\@vscode\ripgrep-universal\bin\win32-x64`）加到 PATH 后调
   `"C:\Program Files\Git\bin\bash.exe"`（`target\runtime\probe\compliance.cmd` 即此形状）。
2. **部署必须先停后拷。** 服务在跑时拷贝会因文件占用静默失败，而随后的 start 仍然成功 ——
   运行起来的是**旧**二进制。每次拷完必须比对源/目标 SHA256（见 §2）。

## 6. 本交接不宣称

- 不宣称 Consensus 失败已定位；它只是被**量化**了，归属方仍待一次带 trailer 的调用确认。
- 不宣称 limit ≥199 可以工作；那是规则要求下的正确失败。
- 不宣称下游其余 provider 路径（HistoricalBars、限价池等）本轮有改动 —— 本轮只触及 Sina
  个股新闻的解析、取数上限与分页停止条件，它们实测仍为 complete。
