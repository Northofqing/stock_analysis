# Windows / ExternalV1 侧：数据缺口清单与盘中异动监控需求

- 日期：2026-10-17
- 来源：Mac 侧 monitor（ExternalV1 消费方 / 调用方）
- 用途：请对端评估并给出排期；P0 项阻塞 Mac 侧业务，P1 项影响能力覆盖，第四部分为新增需求

> **2026-10-17 回执更正**（详见同目录 `2026-10-17-windows-data-gaps-and-intraday-alert-evaluation.md`）
>
> 1. **P0-1 的编号错误**：本文原写「`QualifiedTradingFacts`（Operation 66）」有误。现役合同
>    （`dba66a27…`，随 2026-10-16.1 bundle 交付）中 **Operation 66 = `TradingCalendar`**
>    （已核本仓 `contracts/external_v1_current/market.proto:385`）。QualifiedTradingFacts 若立项，
>    号应自 **67** 起。
> 2. **P0-3 的定性反了**：对 `invalid type: map, expected f64`，本文原判断为「live 契约与 final
>    不一致，请对端返回裸 f64」。经回执实测，**4 个比率字段（`auction_pct` /
>    `auction_turnover_pct` / `auction_volume_ratio` / `auction_yesterday_ratio_pct`）在全 API
>    就是 `{"value","unit"}` 形态**，`stage=live` 与 `stage=final` 同构造器、形态一致；**是 Mac 侧
>    解析器把这 4 个声明成了标量**（写本文时未核 `current_auction_observations.rs` 的字段类型）。
>    已按回执建议改为宽容解析（接受裸值 / `{value,unit}` / null，零契约变更），见该文件 `ratio_f64`。
> 3. **P0-2 的性质**：`complete=false` 属覆盖 v2 的**设计内行为**（实测当日 1097 条 > 300 条/查询
>    上限），本文「若协议上分批是本意，请告知分页约定」一问**已有答案**（v2 覆盖 envelope 已在线）。

---

## 一、背景

Mac 侧 monitor 通过 ExternalV1（127.0.0.1:50052，mTLS，tls_server_name=magic-market.local）消费 Windows 侧数据。
本文汇总的是 **Mac 侧实测到的缺口**，每条给出 `reason_code` 与日志原文片段，便于对端在自己的日志中定位。
所有计数来自 Mac 侧 monitor 日志全量 grep，不是估算。

---

## 二、P0：阻塞业务

### P0-1 `QualifiedTradingFacts`（Operation 66）未交付

| 项 | 内容 |
|---|---|
| reason_code | `trading_fact_contract_not_delivered` |
| 实测计数 | Mac 侧全日志 **7682 次**（持续出现） |
| 单票样例 | `[R-13] 002058 紫竹高科 qualified trading facts unavailable` / `[R-13] 002242 九阳股份 qualified trading facts unavailable` |
| 影响 | 连板识别、快照 I-03/T-03 契约类请求全部 fail-closed；逐票的"生命周期 / 价格带 / 停牌"三件事实拿不到 |
| 期望 | 按已有对端交付请求文档实现 Operation 66，返回逐票的三件事实 |
| 验收 | Mac 侧 `trading_fact_contract_not_delivered` 计数不再增长（锚定新进程窗口统计）

### P0-2 `MarketAnnouncements`（公告）返回 `complete=false`

| 项 | 内容 |
|---|---|
| reason_code | `invalid_evidence`（原因串：`响应 complete=false, 不得接纳 partial batch`） |
| 实测计数 | Mac 侧当前进程窗口内 **21 次** `outcome=partial provider=Custom` |
| 日志原文 | `[NewsMonitor][BR-138][BR-168] 公告 Gateway 获取失败，本轮公告隔离: MarketAnnouncements data gateway failed reason_code=invalid_evidence provider=None retryable=false: 响应 complete=false, 不得接纳 partial batch` |
| 影响 | **公告类推送全部被隔离**（Mac 侧业务规则禁止接纳 partial batch） |
| 期望 | 返回 `complete=true`；或明确一次性返回全量并置 complete=true（若协议上"分批"是本意，请告知分页约定） |
| 验收 | Mac 侧 `outcome=partial`（公告类）不再出现；公告推送恢复 |

### P0-3 Operation 61（`CurrentAuctionObservations`）`stage=live` 契约不符

| 项 | 内容 |
|---|---|
| 现象 | 竞价时段（9:15-9:25）请求 `stage=live` 时，Mac 侧解析失败 |
| 原始错误 | `invalid_evidence: auction record fields invalid: invalid type: map, expected f64` |
| 对比 | 同一 Operation 的 `stage=final` 正常：`admission=ADMITTED complete=true records=1 evidence_shape=ok` |
| 影响 | 竞价**实时**观测不可用，只能等 9:25 后的 final 批次；影响"竞价确认"类业务窗口（9:25-9:30 只剩 5 分钟） |
| 期望 | `stage=live` 的数值字段（auction_price / pre_close_price / auction_pct / auction_volume_shares / auction_amount / auction_unmatched / auction_turnover_pct / auction_volume_ratio / auction_yesterday_ratio_pct / float_market_cap / last_price / open_price）按合同返回**裸 `f64` 或 `null`**，与 `stage=final` 一致 |
| 验收 | 竞价时段内 `stage=live` 返回 `records>0` 且无 `invalid type` |
| 备注 | 该错误最后一次出现在 2026-10-16 早盘竞价时段；Mac 侧已加字段级解析观测（报出首个类型不符的字段名 + 实际类型 + 原值片段），下一竞价时段（10-19 周一）可给出确切字段名 |

### P0-4 `T0Evidence` 数据

| 项 | 内容 |
|---|---|
| 现象 | Mac 侧 T0Evidence 批次 `records=0`（空批次），导致做 T 建议缺数据 |
| 备注 | **Mac 侧盘后无法验证此路径**（T0Evidence 只在盘中 live 窗口取数）。本条待 10-19（周一）盘中实测后补充确切证据 |
| 期望 | 盘中 `records > 0` |

---

## 三、P1：数据源缺口

以下均在 Mac 侧当前进程窗口（约 15 分钟）内实测，来源为 `[DataGateway][<能力>][BR-159] outcome=...`

| 能力 | 结果 | provider | 窗口内次数 |
|---|---|---|---|
| `BlockTrades`（大宗交易） | `unavailable` | Custom | 12 |
| `R-08-global-fx`（汇率） | `unavailable` | Custom | 4 |
| `R-08-global-indices`（全球指数） | `unavailable` | Sina | 4 |
| `CapitalProviderTopNMainNetInflow` | `unavailable` | Eastmoney | 2 |
| `CapitalProviderTopNVolumeRatio` | `unavailable` | Eastmoney | 2 |
| `R-09`（provider_top_n） | `source_transport_failed` | — | 2 |
| `cffex_confirmed_delivery_authority_unavailable` | — | — | 9 |

对照：同窗口内以下能力**正常**，可作健康基线——`board-memberships`（Tdx，22 次 available）、`GlobalNews` 四源（Cailianpress / Eastmoney / Jin / ThePaper，各 5 次 available）、`SecurityIdentity`（Tencent，6 次 available）。

---

## 四、新增需求：盘中异动监控（本次新提出）

### 4.1 需求描述

Mac 侧维护一个**观察池**（每交易日几只到十几只票，随板块轮动每日更新）。这些票在盘中出现异动时，希望 **Windows 侧主动推事件给 Mac**。

- 频率要求：**30 秒级即可**（不需要秒级）
- 触发类型：价格突破 / 涨跌幅 / 量比 / 大单 / 自定义
- 时效要求：触发后 30 秒内到达 Mac

### 4.2 两条候选实现路线（请评估哪条可行、代价各是多少）

**路线 A：TDX 客户端原生预警 → 抓取转发**

对端在 Windows 的 TDX 客户端里配置预警条件，客户端触发后由对端程序抓取并转发。

- 需要评估：TDX 的预警条件是否**可编程写入**（配置文件 / 脚本 / API），还是只能 GUI 手工配置
- 若只能手工配置：请评估"条件每日变更时的人工成本"（观察池每天变，阈值随价格移动）

**路线 B：Windows 侧自算（Mac 侧倾向此路）**

不依赖 TDX 的预警 GUI。对端利用已有的 TDX 行情数据，按 Mac 下发的条件自行轮询监控（30 秒级），触发即上报。

- 需要的开发：一个"取行情 + 比对阈值 + 上报"的循环（对端已在收行情数据，增量不大）
- 好处：条件完全由 Mac 侧控制，观察池每日自动更新，零人工
- 需要新增一个 Mac→Windows 的设置方法（见 4.3）

### 4.3 接口定义

#### （1）事件：Windows → Mac（两条路线都需要）

```
event: tdx_intraday_alert
```

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| code | string | 是 | 标的代码（6 位数字，如 `600396`） |
| exchange | string | 是 | `SHANGHAI` / `SHENZHEN` / `BEIJING` |
| alert_at | string | 是 | ISO 8601 **带时区**（如 `2026-10-19T09:35:12+08:00`）。**必须含日期** —— Mac 侧日志行首只有时间无日期，缺日期会导致无法判断事件所属交易日 |
| alert_type | string | 是 | `price_breakout` / `change_pct` / `volume_ratio` / `block_trade` / `custom` |
| direction | string | 否 | `up` / `down`（突破/跌破方向） |
| threshold | number | 是 | 触发阈值 |
| current_price | number | 是 | 触发时的价格 |
| trigger_value | number | 是 | 实际触发值（如涨跌幅 %、量比值） |
| stage | string | 是 | `live` / `final` |
| source | string | 是 | 数据来源标识（如 `tdx` / `tdx-alert-client`） |

传输通道：使用现有的 **ExternalV1 事件订阅机制**（无需新增 RPC）。

#### （2）方法：Mac → Windows（仅路线 B 需要）

```
method: set_intraday_alerts
```

请求：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| alerts | array | 是 | 条件列表，元素见下 |
| replace_all | bool | 是 | `true` = 整批替换（观察池每日更新时使用）；`false` = 追加 |

`alerts[]` 元素：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| code | string | 是 | 标的代码 |
| exchange | string | 是 | `SHANGHAI` / `SHENZHEN` / `BEIJING` |
| alert_type | string | 是 | 同事件字段 |
| direction | string | 否 | `up` / `down` |
| threshold | number | 是 | 阈值 |
| expire_at | string | 是 | 条件失效时间（ISO 8601 带时区；通常为当日 15:00） |

响应：

| 字段 | 类型 | 说明 |
|---|---|---|
| accepted | number | 已接受的条件数 |
| rejected | number | 被拒绝的条件数（附原因更佳） |

### 4.4 验收标准

- **共同**：观察池内某票盘中触发条件 → Mac 侧 30 秒内收到 `tdx_intraday_alert` 事件（Mac 侧日志出现 `[tdx-alert]` 记录）
- **路线 B 专属**：调用 `set_intraday_alerts`（`replace_all=true`，6 条条件）→ 响应 `accepted=6`；随后重新调用（不同 6 条）→ 旧条件不再触发
- **边界**：非交易时段调用 `set_intraday_alerts` 不应报错（返回 accepted 但当日不触发即可）；`expire_at` 到期后条件自动失效

### 4.5 开放问题（请对端答复）

1. 路线 A 的 TDX 预警条件能否编程写入？若不能，是否接受路线 B？
2. 事件订阅是否需要新增 `event_kind` 注册？还是可复用现有通道？
3. Windows 侧是否具备"按条件轮询行情"的现成能力？复用哪套行情源（与 `board-memberships` 同源，还是独立）？
4. `set_intraday_alerts` 的调用频率预期（Mac 侧计划：每交易日盘后一次 + 观察池变更时一次）

---

## 五、汇总（供排期）

| 编号 | 项目 | 优先级 | 类型 | 阻塞的业务 |
|---|---|---|---|---|
| P0-1 | `QualifiedTradingFacts` op 66 | P0 | 已有交付文档 | 连板识别、快照 I-03/T-03 |
| P0-2 | `MarketAnnouncements` complete=true | P0 | 修复 | 公告推送（盘中盯公告） |
| P0-3 | op 61 `stage=live` 契约 | P0 | 修复 | 竞价实时观测 |
| P0-4 | `T0Evidence` 数据 | P0 | 待 10-19 实测确认 | 做 T 建议 |
| P1-1~7 | 见第三部分表 | P1 | 修复 | 大宗交易 / 汇率 / 全球指数 / 主力净流入 / 量比 / R-09 / cffex |
| NEW | 盘中异动监控（路线 A 或 B） | NEW | 新增开发 | 盘中监控能力（当前完全缺失） |

---

## 附：Mac 侧自查命令（供对端参考对应关系）

Mac 侧用以下命令统计，对端可用相同 `reason_code` 在自己日志中定位：

```bash
LOG=<mac 运行根>/logs/monitor-launchd.stderr.log
grep -c "trading_fact_contract_not_delivered" "$LOG"
grep -c "reason_code=no_verified_batch" "$LOG"
grep -a "响应 complete=false" "$LOG" | tail -3
grep -a "invalid type: map, expected f64" "$LOG" | tail -3
```

> 注意：Mac 侧该日志行首只有 `HH:MM:SS`（无日期）且多日 append，因此统计"是否仍在发生"需按字节偏移锚定新进程窗口，不能直接数全文件频次。
