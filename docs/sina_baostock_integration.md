# Sina + Baostock 数据源集成 (review #15 / #16)

## 背景

review #15 之前 K 线 fallback 链有 4 源 (腾讯 gtimg / 东财 push2 / 东财 HTTP / RustDX TCP), 全部公开 HTTP/TCP, 风险同质且集中在 3 个域名。

Phase 1 (Task 1–7) 加 2 个新源, 提升 fallback 多样性与稳定性:

- **Sina** (`hq.sinajs.cn`, K线 fallback priority 1): 公开 HTTP, 域名独立, 0 费用, GBK 内置反编码
- **Baostock** (`baostock.com`, 盘后专用): 证券所级别日终数据, 无限调用, WebSocket-like session + 复权 `adjustflag=2`

## K线 Fallback 链 (review #15 / Phase 1 — 4-way 盘中竞速)

并行 `tokio::join!` 4 源 (P1 → P4 priority, **首个 Ok+质检通过**即返回):

| Priority | Source | 协议 | 失败行为 | 备注 |
|----------|--------|------|----------|------|
| P1 | `sina_hq` | HTTP (公开, JSONP 解析) | 503/超时 → fallthrough | 域名独立于腾讯/东财, 最先到即返回 |
| P2 | `tencent_qfq` | HTTP (`web.ifzq.gtimg.cn`) | 503/超时 → fallthrough | 复权 (qfq) |
| P3 | `eastmoney_qfq` | HTTP (`push2his.eastmoney.com`) | 503/超时 → fallthrough | 复权 (qfq) |
| P4 | `rustdx_none` | TCP (`82.11.55.178:7709`, 通达信) | spawn_blocking 失败 → fallthrough | 盘中速度最快, 不复权 |

代码入口: `src/data_provider/fallback.rs` `fetch_kline_with_fallback()` (每只股票触发一次该链)。

## 盘后路径 (Phase 1 post_close — Baostock 专属 → 4-way fallthrough)

仅在盘后数据补全路径 (`fetch_kline_post_close`) 使用, 策略与盘中不同:

```
                  ┌──── Baostock (P1, 盘后首选)
                  │     - 日终数据, 无限调用, 稳定
                  │     - session 复用 (login → query → logout)
                  └─失败/异常 → fallthrough
                                  │
                                  ↓
                  4-way join (P2, 同 review #15 盘中竞速)
```

代码入口: `src/data_provider/fallback.rs` `fetch_kline_post_close()`。

## 配置

- 无新增必需 env var — 新源自动启用
- 可选 `BAOSTOCK_BASE_URL` (默认 `http://baostock.com/baostock`)
- 可选 `SINA_HQ_BASE_URL` (默认 `https://hq.sinajs.cn`), 仅调试用
- 现有配置 `MAX_FALLBACK_DEPTH`, `STALE_DATA_TOLERANCE_SECS` 继续生效

## 故障排查

| 现象 | 排查路径 |
|------|----------|
| Sina 503 | 偶发 (CDN 抖动), 4-way 竞速自动 fallthrough, 无需人工介入 |
| Sina 返回乱码 | `sina_provider.rs:decode_gbk()` 已处理; 如仍有乱码 → `charset` header 异常, 检查 HTTP client |
| Baostock login 失败 | 重试 1 次, 仍失败 → fallthrough 到 4-way join |
| Baostock 字段缺失 | `baostock_provider.rs:parse_baostock_response()` 容忍 `\r\n` + 尾部空白; 异常时返回 `Err`, 不返回空数据 |
| 全失败 | `fetch_kline_with_fallback` 返回 `Err` (含每源失败原因), 由上层 caller 决定 fallback (DB cache / 上次 K线 / 跳过) |

## 已知限制 (B-002)

- **B-002**: 当前环境 Baostock login 协议响应可能无 `ErrorCode` 行 (实测异常), `parse_baostock_response` 走 fallthrough.
- 表现: 盘后路径 Baostock 段实际不可用, 自动降级到 4-way. 这是兜底行为, 不是 crash.
- 修复: 后续 Task 调研 (Task 7 报告已记录).

详见 `.superpowers/sdd/progress.md` 的 Bug Log 段落。

## 数据流时序 (盘中)

```
T0: 用户调 fetch_kline_with_fallback("sh600000", 90)
T0+1ms: tokio::join! 启动 4 个 future
        ├─ Sina HTTP GET /list=sh600000...  (Promise A)
        ├─ Tencent HTTP GET ...              (Promise B)
        ├─ Eastmoney HTTP GET ...            (Promise C)
        └─ RustDX spawn_blocking TCP 7709   (Promise D)
T0+50ms: Promise A 返回 Ok → 质检 (gap check + 复权验证)
T0+51ms: 输出 (data, "sina_hq")
         B/C/D 仍在 race, 任务自动 cancel
```

## 数据流时序 (盘后)

```
T0: 用户调 fetch_kline_post_close("sh600000", 90)
T0+1ms: Baostock login → query_kdata_plus → 解析 → logout
        成功 → 输出 (data, "baostock")
        失败 → fallthrough
T0+10ms: 进入 4-way 竞速 (同上 盘中 路径)
T0+60ms: 输出 (data, "tencent_qfq") | ... | Err(...)
```

## 参考

- 代码: `src/data_provider/{sina_provider, baostock_provider, fallback, stock_code_map}.rs`
- 测试: `src/data_provider/{sina_provider, baostock_provider, fallback, stock_code_map}_test.rs`
- 业务规则: `docs/business_rules.md` (BR-014, BR-015)
- 评审: review #15 (4-way 竞速) + review #16 (Phase 1 集成)

---

## Phase 2: Sina 新闻数据集成 (review #16, 2026-07-09)

### 数据源

Sina 公开新闻 feed (与 K线 hq.sinajs.cn 同源不同域, 复用 GBK 容错):

| 用途 | lid | pageid | URL 模板 | 字段 |
|------|-----|--------|----------|------|
| 财经要闻 | 1686 | 153 | `?pageid=153&lid=1686&k=&num=20&page=1` | url / title / intro / ctime / media_name |
| 个股新闻 | 2516 | 155 | `?pageid=155&lid=2516&k={code}&num=20&page=1` | url / title / intro / ctime / media_name |

Base URL: `https://feed.mix.sina.com.cn/api/roll/get`.

### 架构

```
              ┌──── 实时轮询 (poll_news_loop, 90s)
              │     - 拉 20 条财经要闻
              │     - 每条 → insert_news_item (dedup by content_hash)
              │
Sina  ────────┤
              │
              │
              └──── 盘后回溯 (post_close_news_scheduler, 30min tick)
                    - 触发条件: 本地时间 >= 15:30
                    - 取持仓代码列表 (get_positions)
                    - 每只 code → fetch_stock_news_in_range (5 页 × 20 = 100 条)
                    - 客户端过滤 30 天时间窗 → insert_news_item
```

- 实时路径: `poll_news_loop` (`src/bin/monitor/main.rs:5150`) — Task 11
- 盘后路径: `post_close_news_scheduler` → `post_close_news_review` (`src/bin/monitor/main.rs:5190-5240`) — Task 12

### 双写 news_items 表

- 旧 `news_dedup` (5 min 滑窗) — 仍用于实时"短时间内不要再推"提示
- 新 `news_items` (永久详存) — 跨重启追溯 + 后续 LLM 复盘数据源
- 去重键: `content_hash` = SHA256(title + summary) — 同源同 ID 但内容变时可检测
- 入库函数: `DatabaseManager::with_db(caller, |db| db.insert_news_item(item))` (review #15 helper, 一次 warn 不刷屏)

### BR-016

- 业务规则: `docs/business_rules.md` BR-016
- intent: Sina 新闻 API 双路径 (实时 + 盘后) 双写 news_items; 含 content_hash 去重
- code: `src/data_provider/sina_news_provider.rs`, `src/data_provider/news_item.rs`, `src/database/mod.rs`

### 启动 banner

```text
[启动] 新闻轮询: Sina 财经要闻 (90s 间隔, 双写 news_items)
[启动] 盘后回溯: Sina 个股新闻 (15:30 后, 30 天, 持仓代码)
```

### Commit 列表 (Phase 2)

| Task | 内容 | Commit |
|------|------|--------|
| 9 | NewsItem struct + news_items migration | (Task 9 实施, 详见 `.superpowers/sdd/task-9-report.md`) |
| 10 | SinaNewsProvider (top + stock + history range) | (Task 10 实施) |
| 11 | poll_news_loop (90s 财经要闻) | `d9b082f` |
| 12 | post_close_news_review + BR-016 + Phase 2 docs | (Task 12 commit, 详见 `.superpowers/sdd/task-12-report.md`) |
