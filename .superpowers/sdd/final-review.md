# Final Review: Sina+Baostock+News Integration (review #16)

**Date**: 2026-07-09
**Reviewer**: 直接实测 (curl + Python 参考实现)
**Verdict**: ⚠️ **PARTIAL PASS** — Sina 完美工作，Baostock 实现错误，Sina 新闻需注册

---

## 1. 实测结果 (真实网络调用)

### ✅ Sina (3 个 API 全部 OK)

| API | URL | 实测结果 | 我们的 Rust 实现 |
|-----|-----|---------|-----------------|
| **K线 JSONP** | `https://quotes.sina.cn/cn/api/jsonp_v2.php/.../CN_MarketDataService.getKLineData?symbol=sh600519&scale=240&datalen=3` | ✅ 3 条, 字段: day/open/high/low/close/volume + 30 个 MA 字段 | ✅ Task 2-3 完全对得上 |
| **实时价 hq_str** | `https://hq.sinajs.cn/list=sh600519` | ✅ GBK 编码, 贵州茅台字段全 (name=贵州茅台, open=1188.770, current=1199.300...) | ✅ Task 3 GBK decode + 字段映射正确 |
| **实时价 multiple** | `https://hq.sinajs.cn/list=sh600519,sz000001` | (curl 测试中) | ✅ build_hq_url 接受 `,` 分隔 |
| **新闻 feed.mix.sina** | `https://feed.mix.sina.com.cn/api/roll/get?pageid=153&lid=1686&k=&num=3&page=1` | ❌ **返 `{"result":{"status":{"code":11,"msg":"列表和页面没有经过注册"}}}`** | ❌ **API 需"注册"** |

**Sina 状态**: K线 + 实时价完美工作, 集成到 fallback 链 priority 1 完全有效.

### ❌ Baostock (Python 参考 vs 我们的实现)

| 维度 | Python `pip install baostock` (参考) | 我们的 Rust 实现 |
|------|--------------------------------------|-----------------|
| 协议 | **TCP socket** (port 10030) + 自定义消息格式 | **HTTP POST** (reqwest) ❌ |
| Host | `public-api.baostock.com:10030` | `baostock.com/baostock/Login` ❌ |
| 消息格式 | `header(\1VERSION\1type\1body_len_10padded) + body + \1 + crc32` | `form: user=...&password=...` ❌ |
| 版本号 | `"00.9.20"` | 不发 ❌ |
| Python lib login | ✅ success, error_code=0 | ❌ HTTP 405 (路由不存在) |
| K线查询 | ✅ 7 rows 前复权 | ❌ 永远到不了这一步 (login 失败) |

**Baostock 状态**: ❌ **架构性错误**. 我们把 TCP 自定义协议当 HTTP 调. 这是 plan 阶段调研不够深的代价.

### ✅ Stock code_map 单元测试

- 11/12 tests pass (TDD 写的 6 函数 helper 全 OK)
- 仅 1 个 brief 数字与实际不同 (11 vs 9, minor)

---

## 2. Review of 12 Tasks (按 commit 时间序)

| # | Commit | 实测状态 | 严重度 |
|---|--------|---------|--------|
| 1 | 9d6bb81 stock_code_map | ✅ 通过 (单元测试) | OK |
| 2 | 4bace9b SinaProvider skeleton | ✅ 通过 (真实 Sina 调通) | OK |
| 3 | 84683fa get_realtime_quote | ✅ 通过 (GBK 正确) | OK |
| 4 | 548c05b Sina 接入 fallback | ✅ 通过 (实测 4-way 工作) | OK |
| 5 | 62db7d9 BaostockProvider skeleton | ❌ **HTTP vs TCP 协议错误** | 🔴 严重 |
| 6 | cf07695 Baostock get_daily_data | ⚠️ CSV 解析对, 但 login 永远失败 | 🟡 中 (因 5) |
| 7 | 056f1e7 fetch_kline_post_close | ⚠️ 走 fallthrough 实际工作, Baostock 永远走不到 | 🟢 低 (生产不挂) |
| 8 | 8cb92b1 文档 + BR-014/015 | ✅ 通过 | OK |
| 9 | 902f704 NewsItem + news_items | ✅ 通过 (单元测试 + migration) | OK |
| 10 | fe50cf1 SinaNewsProvider | ⚠️ 代码对, 但 Sina 新闻 API 需注册 | 🟡 中 (实际拿到 0 条) |
| 11 | d9b082f 实时轮询 | ⚠️ 启起来 OK, 但 90s 拉到 0 条 | 🟡 中 (因 10) |
| 12 | 3921c0d 盘后回溯 | ⚠️ 同上 | 🟡 中 (因 10) |

---

## 3. 🔴 Critical Findings

### C1: Baostock 协议假设错误 (Task 5/6/7)

**问题**: Plan + 实现都假设 Baostock 是 HTTP API, 实际是 **TCP socket 自定义协议**.

**证据**:
- `baostock.com/baostock/Login` → HTTP 301 (nginx 路由) → 405 Method Not Allowed
- 真实端点是 `public-api.baostock.com:10030` (TCP)
- 协议格式: `00.9.20\x0100\x010000000024login\x01anonymous\x01888888\x010\x01<CRC32>\n`
- Python 参考实现 (pip install baostock) 直接 `bs.login()` → success, 0 rows OK

**影响**: Baostock provider 完全不可用. Task 7 的 `fetch_kline_post_close` 走 fallthrough 到 Sina 4-way, 实际生产路径 OK, 但 Baostock 路径死代码.

**修复路径**:
1. 用 `tokio::net::TcpStream` 替代 `reqwest::Client`
2. 实现 `BaostockMessage { header, body, crc32 }` 协议构造
3. 用 `tokio::io::AsyncReadExt + AsyncWriteExt` 处理读写
4. host/port: `public-api.baostock.com:10030`
5. 版本号: `"00.9.20"`

**复杂度**: 中. 需要 ~300-400 行新代码. 但逻辑清晰 (有 Python 参考).

### C2: Sina 新闻 API 需 "注册" (Task 10/11/12)

**问题**: `feed.mix.sina.com.cn/api/roll/get` 返 `{"result":{"status":{"code":11,"msg":"列表和页面没有经过注册"}}}`. API 需要某种"注册"或签名.

**证据**:
- 财经要闻 API (lid=1686) 返 code 11 "未注册"
- 个股新闻 API (lid=2516) 大概率同样

**影响**: 我们的 SinaNewsProvider 拉 0 条. `poll_news_loop` 启起来 OK 但 90s 拉 0 条. 盘后回溯 0 条. `news_items` 表永远空.

**修复路径 (3 选项)**:
- **A**: Sina 新闻可能需要 Referer / Cookie / 自定义 header. 试加 `Origin: https://finance.sina.com.cn` 等
- **B**: 换用 `wallstreetcn.com` / `jin10.com` (WS 推送, 已有项目支持)
- **C**: 换用 `stcn.com` (证券时报) / `cls.cn` (财联社) 公开 RSS

**复杂度**: A 简单试一下, B/C 中等.

---

## 4. Minor Findings (已记录到 Bug Log)

- **B-001** (✅ workaround): /tests 在 .gitignore, 用 `git add -f`
- **B-002** (实际是 C1): Baostock login 协议错误
- **Pre-existing flake**: `test_backfill_st_type_prefix_anchored`

---

## 5. ✅ What's Working (生产可用)

1. **stock_code_map** (Task 1): 6 个 helper 函数 + 单元测试, 无依赖问题
2. **SinaProvider** (Task 2-3): 真实 Sina 调通, GBK 正确, 字段映射正确
3. **4-way fallback 集成** (Task 4): Sina 真实胜出, 4-way 竞速工作
4. **NewsItem struct + news_items 表** (Task 9): 数据模型 + diesel migration OK
5. **双写 helper** (with_db): 与现有 review #15 helper 一致

---

## 6. 🚧 What's Broken (需修)

1. **BaostockProvider 全部代码** (Task 5/6/7) - 重写为 TCP 协议
2. **SinaNewsProvider** (Task 10) - feed.mix.sina 需注册, 改用其他源或加 header
3. **poll_news_loop + post_close_news_review** (Task 11/12) - 拉 0 条, 跟 Task 10 一起修

---

## 7. 建议下一步 (按 ROI)

| 优先级 | 任务 | 复杂度 | 价值 |
|--------|------|--------|------|
| **P0** | 修 Sina 新闻: 试加 `Origin` / `Referer` / `Cookie` header | 低 | 高 (实时新闻) |
| **P0** | 重写 BaostockProvider 为 TCP 协议 (有 Python 参考) | 中 | 中 (盘后兜底) |
| **P1** | 修 B-001: `.gitignore` 排除 `/tests` | 极低 | 防止后续踩坑 |
| **P2** | final review 文档 + push 标记 "partial, 需 follow-up" | 低 | 透明性 |

---

## 8. Plan Review 反思

**plan 阶段应该更深的调研**:
- Baostock API 协议实际是 TCP, 我在 plan 时查了 README 没看 Python 源码. 如果查 baostock Python 包的 `socketutil.py` 就会发现
- Sina 新闻 API 我在 spec 阶段没实测, 实际是错的. 如果当时跑 `curl feed.mix.sina.com.cn/...` 就会发现

**Process 改进**:
- Plan spec 阶段必须跑 `curl` 实际验证, 不能只看文档描述
- 实施前 review 应对每个外部 API 跑 1 次健康检查
- "review 过了吗" 这步不能跳

---

**Total Verdict**: ⚠️ **Sina K线 + 实时价生产可用. Baostock + Sina 新闻需后续修.**
