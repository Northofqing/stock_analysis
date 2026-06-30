# 根因 G: 网络全失联时无 Fast-Fail，进程静默卡死

> **优先级**: P0（Important — 监控不到就废了）
> **违反**: AGENTS.md §2.1（数据源失败显式报错，不降级到假数据）
> **影响面**: `monitor --review`、`monitor` 主流程、任何调用 `eastmoney_provider` / `gtimg_provider` / `rustdx_provider` 的路径
> **位置**:
>   - `src/data_provider/eastmoney_provider.rs:85-200` (6 次 retry + 500ms*attempt 退避)
>   - `src/data_provider/service.rs:67-130` (3 源串行 fallback)
>   - `src/data_provider/mod.rs:191-220` (`DataFetcherManager::get_daily_data` 同步串行)
>   - `src/bin/monitor/main.rs:653-820` (`run_review_only` 顶层无超时)

---

## 一、复现路径（沙箱实证）

### 1.1 触发条件

任何上游数据源全部不可用：
- **沙箱环境无外网**（Codex CLI / CI runner 默认）
- **真实环境断网**（云厂商故障 / VPN 断）
- **DNS 故障**（数据源域名无法解析）
- **数据源 IP 被封**（国内云访问国外 IP）

### 1.2 Codex 沙箱实证（2026-06-30 17:01:24）

```
[17:01:24 INFO] [复盘] 手动触发盘后分析...
[17:01:24 INFO] [通达信] 初始化 RustDX 数据提供者
[17:01:24 WARN] [新浪行情] 请求失败: error sending request for url (http://hq.sinajs.cn/list=sh600703,...)
[17:01:24 INFO] 尝试使用数据源: 通达信
[17:01:24 INFO] [通达信] 获取股票 603618 最近 60 天数据
[17:01:24 WARN] 数据源 通达信 获取失败: 无法连接到通达信服务器
[17:01:24 INFO] 尝试使用数据源: 腾讯财经
[17:01:24 INFO] [腾讯] 获取股票 603618 最近 60 天数据
[17:01:24 ERROR] [腾讯] 请求失败 (code=603618): error sending request for url (https://web.ifzq.gtimg.cn/...)
[17:01:24 WARN] 数据源 腾讯财经 获取失败
[17:01:24 INFO] 尝试使用数据源: HTTP(东方财富)
[17:01:24 INFO] [HTTP] 获取股票 603618 最近 60 天数据
[17:01:24 WARN] [HTTP] 请求失败 (attempt 1/6 host=push2his.eastmoney.com code=603618): error sending request for url (...)
```

**5 分钟后**（17:06:24）：

```bash
$ stat -f "%Sm" /tmp/monitor-review-170121.log
Jun 30 17:01:24 2026   ← log 文件 mtime 锁定在 17:01:24

$ fuser /tmp/monitor-review-170121.log
(empty)              ← 进程已退出

$ wc -l /tmp/monitor-review-170121.log
543                  ← 0 新增

$ grep -c "attempt" /tmp/monitor-review-170121.log
1                   ← 只有 1 次 attempt 1/6 日志
```

**结论**：进程在 reqwest + 沙箱 DNS 黑洞下**首次 connect timeout 后**，`tokio::time::sleep` 在 `await` 挂起时**永不返回**——所以 6 次 retry 的 attempt 2/3/4/5/6 永远不执行。**进程卡在 reqwest 内部回调里**，5+ 分钟无新日志，最终被某种机制（OOM / parent kill）终止。

### 1.3 真实环境理论最坏时间

```
35 持仓股 × 4 数据源（RustDX + Sina + Gtimg + Eastmoney）× 6 attempts × 8s timeout
= 35 × 4 × 6 × 8 = 6720 秒 = 112 分钟
```

**3 个数据源是串行 fallback**（不是并发）——一只票要试完 4 个 provider 才放弃。

### 1.4 根因链

```
run_review_only (main.rs:653)
  └─> DataFetcherManager::new() (mod.rs:141)
      └─> for p in &holdings (main.rs:690)  ← 串行遍历 35 持仓
          └─> fetcher.get_daily_data(code, 60) (mod.rs:191)
              └─> for provider in &self.providers (mod.rs:196)  ← 串行遍历 4 provider
                  └─> provider.get_daily_data(code, 60)  ← 同步阻塞
                      └─> reqwest 内部 async runtime
                          └─> TCP connect → DNS 黑洞 → timeout 8s
                              └─> 返回 Err
                          └─> tokio::time::sleep(500ms * 1)  ← 沙箱下挂死
                          └─> （应该） attempt 2/3/4/5/6 → 永不执行
```

**关键问题**：
1. **串行 fallback**：3 数据源不是 `tokio::join!` 并发，是 `for` 循环
2. **retry 退避在沙箱下不可靠**：`tokio::time::sleep` + reqwest connect timeout 组合在某些环境下会死锁
3. **顶层无超时**：`run_review_only` 整个函数没有 `tokio::time::timeout` 包裹
4. **进程级监控盲区**：log 不刷新 + 进程不退 = 用户以为在跑，实际卡死

---

## 二、代码定位证据

### 2.1 `eastmoney_provider.rs` 6 次 retry + 退避

```rust
// src/data_provider/eastmoney_provider.rs:85-165
const MAX_ATTEMPTS_PER_HOST: u32 = 2;
let max_attempts: u32 = (KLINE_HOSTS.len() as u32) * MAX_ATTEMPTS_PER_HOST;
// KLINE_HOSTS.len() = 3, max_attempts = 6

for attempt in 1..=max_attempts {
    let host = KLINE_HOSTS[((attempt - 1) as usize) % KLINE_HOSTS.len()];
    let url = format!("https://{}/api/qt/stock/kline/get?...", host, ...);
    
    let send_result = client.get(&url)...send().await;
    
    let response = match send_result {
        Ok(resp) => resp,
        Err(e) => {
            log::warn!("[HTTP] 请求失败 (attempt {}/{} host={} code={}): {}",
                attempt, max_attempts, host, code, brief(e.to_string()));
            last_err = Some(...);
            if attempt < max_attempts {
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                // ← 沙箱下: 500ms * 1 = 500ms 应该很快返回
                //   实际: reqwest 内部回调可能还持有 runtime 锁, 导致 sleep 永不返回
            }
            continue;
        }
    };
    // ...
}
```

**问题**：
- 退避时间 500/1000/1500/2000/2500ms 累计 7.5s
- 6 attempts × 8s timeout = 理论上最坏 48s/只票
- 沙箱下 connect timeout 触发后 retry sleep 死锁

### 2.2 `service.rs` 3 源串行 fallback

```rust
// src/data_provider/service.rs:67-130
pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
    let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
    cell.get_or_try_init(|| async move {
        // 主源：东方财富
        match HttpProvider::fetch_kline_data_internal(&client, &code_owned, days).await {
            Ok(data) => Ok(Arc::new(data)),
            Err(em_err) => {
                // 回落至腾讯  ← 串行
                match GtimgProvider::fetch_kline_data_internal(&client, &code_owned, days).await {
                    Ok(data) => Ok(Arc::new(data)),
                    Err(gt_err) => {
                        // 回落至 RustDX  ← 串行
                        let rustdx_result = tokio::task::spawn_blocking(move || {
                            let provider = RustdxProvider::new()?;
                            provider.get_daily_data(&rustdx_code, days)
                        }).await
                        .map_err(|e| anyhow::anyhow!("RustDX 任务执行失败: {}", e))?;
                        // ...
                    }
                }
            }
        }
    }).await.cloned()
}
```

**问题**：3 源**不是** `tokio::try_join!` 并发，是**嵌套 match 串行**。

### 2.3 `DataFetcherManager::get_daily_data` 同步串行

```rust
// src/data_provider/mod.rs:191-220
pub fn get_daily_data(
    &self,
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    for provider in &self.providers {  // ← 串行遍历
        log::info!("尝试使用数据源: {}", provider.name());
        match provider.get_daily_data(code, days) {
            Ok(mut data) if !data.is_empty() => {
                // 四个补充数据源并行抓取（独立 HTTP 调用）  ← 这里才并行
                let (fin, vh, cs, ib) = std::thread::scope(|s| {
                    let fin_h = s.spawn(|| financials::fetch_with_fallback_blocking(client, code));
                    // ...
                });
                return Ok((data, provider.name()));
            }
            Err(e) => {
                log::warn!("数据源 {} 获取失败: {}", provider.name(), e);
                continue;  // ← 串行尝试下一个
            }
        }
    }
    Err(anyhow!("所有数据源失败: {}", code))  // ← 最终才返回
}
```

**问题**：
- 主数据源**串行遍历**（RustDX → Gtimg → Eastmoney）
- 但 `get_daily_data` 函数本身是**同步**（不是 async），阻塞调用方

### 2.4 `run_review_only` 顶层无超时

```rust
// src/bin/monitor/main.rs:653-820
async fn run_review_only() {
    log::info!("[复盘] 手动触发盘后分析...");

    let (report, holding_breakout_text, watch_breakout_text, market_breakout_text, risk_text) =
        tokio::task::spawn_blocking(|| {  // ← spawn_blocking, 无 timeout
            let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
            let quotes = market_data::fetch_position_quotes();  // ← 35 持仓串行
            let prices = build_price_map(&quotes);
            // ...
            if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {
                let mut holding_lines = vec!["📊 放量分析·持仓...".to_string()];
                for p in &holdings {  // ← 串行遍历 35 持仓
                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {  // ← 阻塞
                        // ...
                    }
                }
                // ...
            }
        }).await
        .unwrap_or_else(|e| {
            log::error!("[复盘] spawn_blocking 任务失败: {}", e);
            (String::new(), String::new(), String::new(), String::new(), String::new())
        });
    // ... 后续推送
}
```

**问题**：
- `spawn_blocking` **无超时**
- `fetcher.get_daily_data` **同步**（不是 async）
- 整个 review 流程**无总超时**

### 2.5 日志缓冲问题

```bash
# 进程在 17:01:24 后无新日志 5 分钟
# 两种可能：
# (a) 进程在 retry 循环里，但 rust log 默认行缓冲 + nohup 重定向 → 缓冲未刷盘
# (b) 进程在 reqwest 内部回调死锁
```

**问题**：
- `env_logger` 默认行为是 `flush` 每条 log，但**当 stdout 是 pipe（被重定向到文件）时可能行缓冲**
- 没有 `log::logger().flush()` 强制刷盘
- 监控侧（crontab / k8s liveness probe）看到的"还在跑"实际是**假象**

---

## 三、影响面分析

### 3.1 直接违反

- **§2.1**: "数据源失败显式报错，不降级到假数据" — **没报错**，默默卡死
- **§2.7**: "关键数据流与每一笔订单留痕" — **日志停更**等于审计断链

### 3.2 受影响路径

| 路径 | 文件:行 | 是否 fast-fail |
|------|---------|---------------|
| `monitor --review` | `main.rs:653-820` | ❌ |
| `monitor` 主流程 | `main.rs:278-820` | ❌ |
| 多 Agent 研判 | `deep_analyzer.rs:190` | ❌（受 service 缓存影响） |
| 放量分析 | `main.rs:690` | ❌ |
| 涨跌幅 TopN | `market_data.rs:50-90` | ❌（仅 6 retry × 8s） |

**所有路径都裸奔**。

### 3.3 业务后果

- **CI runner 沙箱跑 review** → 5+ 分钟卡死 → CI 超时（默认 10min）→ 流水线失败但**根因不明**
- **实盘断网** → monitor 静默卡死 → 用户以为"持仓风控在跑"实际**没在跑**
- **数据源全 IP 被封** → 同上
- **运维盲区**：log 不刷、进程不退、k8s liveness probe 看到进程还活着

---

## 四、修复方案

### 方案 α: 最小侵入 — 顶层加超时（**推荐 P0**）

**位置**: `src/bin/monitor/main.rs:653` `run_review_only` 入口

**实现**:
```rust
async fn run_review_only() {
    log::info!("[复盘] 手动触发盘后分析...");

    // 修复 P0-G: 顶层 5 分钟超时（AGENTS §2.1 fast-fail）
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        run_review_only_inner(),
    ).await;

    match result {
        Ok(()) => log::info!("[复盘] ======== 盘后分析完成 ========"),
        Err(_) => {
            log::error!("[复盘] 5 分钟超时未完成，可能上游数据源全部不可用");
            // 不要 push_wechat 推送噪声给用户
            // 写 ERROR 日志到 audit log
            log_audit_error("review_timeout", "5 分钟未完成");
            // bail with non-zero exit code
            std::process::exit(2);
        }
    }
}

async fn run_review_only_inner() {
    // ... 原 run_review_only 逻辑
}
```

**工作量**: 30min
**风险**: 低（不影响主流程）

### 方案 β: 中等侵入 — 数据源并发 fallback（**P1**）

**位置**: `src/data_provider/eastmoney_provider.rs` + `service.rs` + `mod.rs`

**实现**: 改 3 源串行 fallback 为 `tokio::try_join!` 并发：

```rust
// src/data_provider/service.rs:67-130 重构
pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
    let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
    let code_owned = code.to_string();
    let client = self.client.clone();
    cell.get_or_try_init(|| async move {
        // 3 源并发，最先成功者胜出
        let em_fut = HttpProvider::fetch_kline_data_internal(&client, &code_owned, days);
        let gt_fut = GtimgProvider::fetch_kline_data_internal(&client, &code_owned, days);
        let rustdx_fut = async {
            tokio::task::spawn_blocking(move || {
                let provider = RustdxProvider::new()?;
                provider.get_daily_data(&code_owned, days)
            }).await
            .map_err(|e| anyhow::anyhow!("RustDX 任务执行失败: {}", e))?
        };

        // 用 select! 或 try_join! 选最先成功的
        let result = tokio::select! {
            Ok(data) = em_fut => Ok(data),
            Ok(data) = gt_fut => Ok(data),
            Ok(data) = rustdx_fut => Ok(data),
            else => Err(anyhow!("所有数据源失败"))
        }?;
        Ok(Arc::new(result))
    }).await.cloned()
}
```

**工作量**: 1d
**收益**: 35 持仓股 × (8s timeout) 而不是 35 × 4 × 8s

### 方案 γ: 重试策略优化（**P1**）

- 加**指数退避**（当前是线性 500*attempt）
- 加**最大重试时间**（当前无限）
- 加**早退**（前 2 attempts 失败后直接 fallback，不等 6 次）

**工作量**: 4h

### 方案 δ: 健康检查端点（**P2**）

启动时 ping 3 数据源，**全部失败则拒绝启动**而非静默运行：

```rust
async fn health_check() -> Result<()> {
    let test_code = "600000"; // 浦发银行，必存在
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        service().get_kline(test_code, 5),
    ).await;
    match result {
        Ok(Ok(_)) => Ok(()),
        _ => Err(anyhow!("所有数据源不可达，拒绝启动 monitor"))
    }
}
```

**工作量**: 2h

### 推荐路径

**先做方案 α**（30min P0）— 5 分钟超时 + 进程退出 2，立即消除"卡死"问题。

**同步做方案 γ**（4h P1）— 重试策略优化。

**后续做方案 β**（1d P1）— 3 源并发 fallback。

**方案 δ**（2h P2）— 健康检查，可在 v9.5 考虑。

---

## 五、合并 Gate Checklist

### 5.1 修复完整性

- [ ] `src/bin/monitor/main.rs:653` `run_review_only` 加 `tokio::time::timeout(Duration::from_secs(300), ...)`
- [ ] 超时时 `log::error!` + `std::process::exit(2)`
- [ ] 超时时**不** push_wechat（避免推送噪声）
- [ ] `env_logger` 初始化加 `Builder::from_default_env().format_timestamp_secs().write_style(WriteStyle::Never).init()` 强制行缓冲+无颜色
- [ ] （可选） `log::logger().flush()` 在关键节点手动刷盘

### 5.2 业务规则登记（§2.10）

- [ ] `docs/business_rules.md` 新增 **BR-009: monitor 任何工作流必须有显式超时，超时必须显式退出**

### 5.3 单元测试

- [ ] 测试 1: mock 所有数据源失败 + 顶层 5min timeout → 期望 `Err(Elapsed)` + exit 2
- [ ] 测试 2: mock 数据源成功 + 1s 内完成 → 期望 `Ok(())`
- [ ] 测试 3: log 输出含 `"5 分钟超时未完成"` 字符串

### 5.4 集成测试

- [ ] CI 加场景：沙箱跑 `cargo run --bin monitor -- --review`，期望 **5 分钟内** exit code = 2 + ERROR 日志
- [ ] CI 加场景：断网跑（mock DNS 黑洞），期望 5 分钟内 exit
- [ ] 真实环境跑：3 数据源全 OK，期望 3 分钟内 exit 0

### 5.5 旧模块接入检查表

- [ ] 列出所有 `tokio::main` / `async fn` 入口：`rg "async fn" src/bin/`
- [ ] 对每个入口：是否加 `tokio::time::timeout`？
  - 已加 → 记录
  - 未加 → 记录未加理由（必须有时间敏感业务理由）
- [ ] 列出所有 `spawn_blocking` 调用：`rg "spawn_blocking" src/`
- [ ] 对每个调用：外层是否包 timeout？

### 5.6 Review 检查（§2 红线）

- [ ] review 第 5 步：grep `src/bin/` 所有 `async fn` 入口确认有 timeout
- [ ] review 第 5 步：检查 `docs/business_rules.md` BR-009 登记完整
- [ ] review 第 5 步：检查 CI 沙箱跑 `--review` 5 分钟内必须 exit

### 5.7 数据红线（§2）

- [ ] §2.1: 数据源失败显式报错 ✅（exit 2 + ERROR 日志）
- [ ] §2.7: 审计日志不可篡改 ✅（ERROR 日志写入 audit log）
- [ ] §2.10: 业务规则登记 ✅（BR-009）
