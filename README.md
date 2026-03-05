# A股自选股智能分析系统 - Rust 版本

完整的股票分析系统，包含新闻搜索、趋势分析和 AI 分析功能。

## ✨ 核心功能

1. **📰 新闻搜索服务** - 多引擎搜索（Bocha/Tavily/SerpAPI）+ 自动负载均衡
2. **📊 趋势交易分析** - 基于 MA 均线的技术分析 + 买点识别
3. **🤖 AI 智能分析** - Gemini/OpenAI/豆包 驱动的决策面板生成 ⭐ 新增豆包支持
4. **💾 数据库存储层** - SQLite + Diesel ORM + 断点续传
5. **📈 大盘复盘分析** - 市场指数监控 + 板块分析 + 复盘报告
6. **🔄 多因子回测** - 量化选股 + 回测模拟 + 性能指标 ⭐ 新增
7. **📢 多渠道通知** - 企业微信/飞书/邮件推送 + 日报生成 ⭐ 新增

> 📬 **邮件通知新功能**: 支持 SMTP 邮件发送，自动转换 HTML 格式！  
> 快速配置: [EMAIL_QUICKSTART.md](EMAIL_QUICKSTART.md) | 详细文档: [EMAIL_CONFIG.md](EMAIL_CONFIG.md)

> 📊 **多因子回测新功能**: 自动化量化选股和回测，结果自动推送！  
> 详细文档: [MULTI_FACTOR_BACKTEST.md](MULTI_FACTOR_BACKTEST.md) | 实现总结: [IMPLEMENTATION_SUMMARY.md](IMPLEMENTATION_SUMMARY.md)

## 项目结构

```
stock_analysis/
├── src/
│   ├── lib.rs                    # 库入口
│   ├── main.rs                   # 主程序
│   ├── search_service.rs         # 搜索服务（Bocha/Tavily/SerpAPI）
│   ├── ai_analyzer.rs            # AI 分析器（Gemini/OpenAI）
│   ├── trend_analyzer.rs         # 趋势交易分析器
│   ├── database.rs               # 数据库管理器
│   ├── models.rs                 # 数据模型定义
│   ├── schema.rs                 # 数据库schema
│   ├── market_data.rs            # 市场数据结构
│   ├── market_analyzer.rs        # 大盘复盘分析器
│   ├── multi_factor_strategy.rs  # 多因子选股策略 ⭐ 新增
│   ├── backtest.rs               # 回测引擎 ⭐ 新增
│   └── notification.rs           # 通知服务 ⭐ 新增
├── examples/
│   ├── search_example.rs           # 搜索服务示例
│   ├── ai_example.rs               # AI 分析器示例
│   ├── trend_analysis_example.rs   # 趋势分析示例
│   ├── database_example.rs         # 数据库使用示例
│   ├── comprehensive_analysis.rs   # 综合分析流程
│   ├── full_analysis.rs            # 完整分析流程
│   ├── market_review.rs            # 大盘复盘示例
│   └── notification_example.rs     # 通知服务示例 ⭐ 新增
├── migrations/                      # 数据库迁移 ⭐ 新增
│   └── 2026-01-22-000000_create_stock_daily/
│       ├── up.sql
│       └── down.sql
├── tests/
│   └── search_test.rs      # 单元测试
├── .env.example            # 配置模板
├── diesel.toml             # Diesel 配置 ⭐ 新增
├── SEARCH_SERVICE.md       # 搜索服务文档
├── AI_ANALYZER.md          # AI 分析器文档
├── DOUBAO_CONFIG.md        # 豆包AI配置指南 ⭐ 新增
├── TREND_ANALYZER.md       # 趋势分析器文档
├── DATABASE.md             # 数据库文档
├── MARKET_ANALYZER.md      # 大盘分析器文档
├── MULTI_FACTOR_BACKTEST.md    # 多因子回测文档 ⭐ 新增
├── IMPLEMENTATION_SUMMARY.md   # 回测实现总结 ⭐ 新增
├── NOTIFICATION.md         # 通知服务文档 ⭐ 新增
├── EMAIL_CONFIG.md         # 邮件配置详细文档 ⭐ 新增
├── EMAIL_QUICKSTART.md     # 邮件快速配置指南 ⭐ 新增
├── SCHEDULE_GUIDE.md       # 定时任务完整指南 ⭐ 新增
├── SCHEDULE_QUICKSTART.md  # 定时任务5分钟上手 ⭐ 新增
├── SCHEDULE_SUMMARY.md     # 定时任务技术总结 ⭐ 新增
├── ENV_CONFIG.md           # 环境变量配置指南 ⭐ 新增
├── CHANGELOG.md            # 版本更新日志 ⭐ 新增
├── test_schedule.sh        # 定时任务测试脚本 ⭐ 新增
├── verify_backtest.sh      # 回测功能验证脚本 ⭐ 新增
└── README.md               # 本文件
```

## 快速开始

### 1. 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env 文件，填入你的 API Keys
```

### 3. 运行示例

```bash
# 新闻搜索示例
cargo run --example search_example

# 趋势技术分析示例 ⭐ 新增
cargo run --example trend_analysis_example

# AI 分析器示例
cargo run --example ai_example

# 综合分析流程（搜索+技术+AI）⭐ 新增
cargo run --example comprehensive_analysis

# 数据库使用示例 ⭐ 新增
cargo run --example database_example

# 大盘复盘分析示例 ⭐ 新增
cargo run --example market_review

# 通知服务示例 ⭐ 新增
cargo run --example notification_example

# 带日志输出
RUST_LOG=info cargo run --example market_review
```

## 命令行使用

### 基本用法

```bash
# 1. 立即执行一次分析（默认模式）
cargo run --release

# 2. 仅大盘复盘
cargo run --release -- --market-review

# 3. 指定自选股代码
cargo run --release -- --stocks "000001,600519,300750"

# 4. 调整并发数
cargo run --release -- --workers 10

# 5. 干跑模式（不发送通知）
cargo run --release -- --dry-run

# 6. 单条汇总邮件
cargo run --release -- --single-notify
```

### ⏰ 定时任务模式 ⭐ 新增

强大的定时任务功能，支持多种执行策略。详细文档: [SCHEDULE_GUIDE.md](SCHEDULE_GUIDE.md)
**配置方式1: 使用环境变量（推荐日常使用）**

编辑 `.env` 文件：
```bash
SCHEDULE_ENABLED=true
SCHEDULE_TIME=09:30,15:30
```

然后直接运行：
```bash
cargo run --release
```

**配置方式2: 使用命令行参数（更灵活）**
#### 1️⃣ 间隔执行模式

按固定时间间隔重复执行：

```bash
# 每30分钟执行一次
cargo run --release -- --schedule --interval 30

# 每2小时执行一次
cargo run --release -- --schedule --interval 120

# 立即执行一次，然后每小时执行
cargo run --release -- --schedule --interval 60 --run-now
```

#### 2️⃣ 指定时间点模式

每天在固定时间执行：

```bash
# 每天早上9:30执行
cargo run --release -- --schedule --schedule-time "09:30"

# 每天两次：开盘前和收盘后
cargo run --release -- --schedule --schedule-time "09:15,15:30"

# 三个时间点：开盘前、午盘、收盘后
cargo run --release -- --schedule --schedule-time "09:15,12:00,15:30"
```

#### 3️⃣ 工作日执行模式（推荐）

只在交易日执行，跳过周末：

```bash
# 每周一到周五的9:30执行
cargo run --release -- --schedule \
  --schedule-time "09:30" \
  --weekdays 1,2,3,4,5

# 工作日两次执行：开盘和收盘
cargo run --release -- --schedule \
  --schedule-time "09:15,15:30" \
  --weekdays 1,2,3,4,5 \
  --run-now
```

星期代码: `1`=周一, `2`=周二, `3`=周三, `4`=周四, `5`=周五, `6`=周六, `7`=周日

#### 定时任务参数

```
--schedule              启用定时任务模式
--schedule-time <TIME>  指定时间点 (格式: HH:MM 或 "HH:MM,HH:MM")
--interval <MINUTES>    执行间隔 (分钟)
--weekdays <DAYS>       指定星期 (逗号分隔, 1=周一, 7=周日)
--run-now               立即执行一次，不等待定时
```

#### 后台运行（生产环境）

macOS/Linux:
```bash
# 使用 nohup 后台运行
nohup cargo run --release -- --schedule \
  --schedule-time "09:30,15:30" \
  --weekdays 1,2,3,4,5 \
  > schedule.log 2>&1 &

# 查看日志
tail -f schedule.log
```

更多后台运行方式（systemd, launchd, Docker）见 [SCHEDULE_GUIDE.md](SCHEDULE_GUIDE.md)

#### 实用场景示例

**场景1: 盘前快速扫描**
```bash
cargo run --release -- --schedule \
  --schedule-time "09:15" \
  --weekdays 1,2,3,4,5 \
  --run-now
```

**场景2: 收盘后完整分析**
```bash
cargo run --release -- --schedule \
  --schedule-time "15:30" \
  --weekdays 1,2,3,4,5
```

**场景3: 开盘+收盘双重分析**
```bash
cargo run --release -- --schedule \
  --schedule-time "09:20,15:30" \
  --weekdays 1,2,3,4,5
```

**场景4: 盘中每小时监控**
```bash
cargo run --release -- --schedule \
  --interval 60 \
  --run-now
```

## 模块文档

### 📰 搜索服务模块

提供统一的新闻搜索接口，支持多个搜索引擎和负载均衡。

- [完整文档](SEARCH_SERVICE.md)
- 支持的引擎: Bocha, Tavily, SerpAPI
- 自动故障转移和负载均衡
- 结果格式化和缓存

### 📊 趋势分析器模块 ⭐ 新增

基于均线系统的技术分析模块，实现严进策略和趋势交易理念。

- [完整文档](TREND_ANALYZER.md)
- 趋势判断: MA5/MA10/MA20 多头排列检测
- 乖离率控制: 不追高原则（< 5%）
- 量能分析: 缩量回调识别
- 买点识别: 回踩支撑位检测
- 综合评分: 0-100分智能评分系统

**核心特性**:
- ✅ 7种趋势状态识别
- ✅ 5种量能状态分析
- ✅ 支撑压力位自动计算
- ✅ 买入/卖出信号生成
- ✅ 风险因素提示

### 🤖 AI 分析器模块

基于 Gemini/OpenAI API 的智能股票分析系统。

- [完整文档](AI_ANALYZER.md)
- 决策仪表盘输出
- 双 API 支持（Gemini + OpenAI）
- 自动重试和故障转移
- 结合新闻的综合分析

### 💾 数据库存储层模块 ⭐ 新增

基于 SQLite + Diesel ORM 的数据持久化层。

- [完整文档](DATABASE.md)
- 单例模式数据库管理
- UPSERT 策略（存在则更新）
- 断点续传逻辑
- 分析上下文生成
- 均线形态自动判断
- 连接池优化

**核心特性**:
- ✅ 股票日线数据存储
- ✅ 智能断点续传（避免重复请求）
- ✅ 分析上下文自动生成
- ✅ 均线形态判断（多头/空头排列）
- ✅ 批量操作支持
- ✅ 完整的索引优化

### 📈 大盘复盘分析器模块 ⭐ 新增

提供A股市场整体行情监控和复盘报告生成功能。

- [完整文档](MARKET_ANALYZER.md)
- 实时指数行情: 上证/深证/创业板等6大指数
- 市场统计: 涨跌家数、涨停跌停统计
- 板块分析: 领涨/领跌板块排名
- 新闻搜索: 自动搜索市场相关新闻
- 复盘报告: 自动生成markdown格式报告（可选AI增强）

**核心特性**:
- ✅ 6大主要指数实时行情
- ✅ 全市场涨跌统计（5000+只股票）
- ✅ 行业板块涨跌榜
- ✅ 市场新闻自动搜索
- ✅ 模板报告自动生成
- ✅ AI增强复盘（可选）
- ✅ 报告保存为Markdown文件

**数据来源**:
- 指数行情: 新浪财经API
- A股行情: 东方财富API
- 板块数据: 东方财富行业板块API

### 📢 通知服务模块 ⭐ 新增

多渠道推送服务，将分析结果生成精美报告并推送到各类平台。

- [完整文档](NOTIFICATION.md)
- 报告生成: Markdown格式日报
- 多渠道推送: 企业微信/飞书/Telegram/邮件等
- 智能分批: 自动处理消息长度限制
- 渠道检测: 自动识别已配置渠道

**核心特性**:
- ✅ 完整版日报（详细分析）
- ✅ 精简版日报（企业微信适配 4KB）
- ✅ 企业微信 Webhook（支持Markdown）
- ✅ 飞书 Webhook（支持交互卡片）
- ✅ 自动分批发送（智能段落分割）
- ✅ 格式转换（飞书Markdown适配）
- ✅ Emoji评分映射（💚🟢🟡⚪🟠🔴）
- ⏳ Telegram Bot（计划中）
- ✅ 邮件SMTP ⭐ 已实现
- ⏳ Pushover推送（计划中）

**支持渠道**:
- 企业微信: 限制4KB，自动分批
- 飞书: 限制20KB，交互卡片
- 邮件: SMTP发送，HTML格式 ⭐ 已实现
- Telegram: Markdown格式（计划中）
- Pushover: 1KB限制（计划中）
- 自定义Webhook: Dingtalk/Discord/Slack（计划中）

## 完整使用示例

### 示例：完整的股票分析流程

```rust
use stock_analysis::{
    search_service::get_search_service,
    ai_analyzer::get_analyzer,
};
use serde_json::{json, Value};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化
    dotenv::dotenv().ok();
    env_logger::init();

    let stock_code = "600519";
    let stock_name = "贵州茅台";

    // === 步骤 1: 搜索新闻 ===
    println!("🔍 步骤 1: 搜索股票新闻...");
    
    let search_service = get_search_service();
    let news_response = search_service
        .search_stock_news(stock_code, stock_name, 5)
        .await;

    if news_response.success {
        println!("✓ 找到 {} 条新闻", news_response.results.len());
        for (i, result) in news_response.results.iter().enumerate() {
            println!("  {}. {}", i + 1, result.title);
        }
    }

    // === 步骤 2: 准备技术面数据 ===
    println!("\n📊 步骤 2: 准备技术面数据...");
    
    let mut context: HashMap<String, Value> = HashMap::new();
    context.insert("code".to_string(), json!(stock_code));
    context.insert("stock_name".to_string(), json!(stock_name));
    context.insert("date".to_string(), json!("2026-01-22"));
    context.insert("today".to_string(), json!({
        "open": 1800.0,
        "high": 1850.0,
        "low": 1780.0,
        "close": 1820.0,
        "volume": 10000000.0,
        "amount": 18200000000.0,
        "pct_chg": 1.5,
        "ma5": 1810.0,
        "ma10": 1800.0,
        "ma20": 1790.0,
    }));

    // === 步骤 3: AI 分析 ===
    println!("\n🤖 步骤 3: AI 智能分析...");
    
    let analyzer_mutex = get_analyzer();
    let mut analyzer = analyzer_mutex.lock().unwrap();

    if !analyzer.is_available() {
        println!("❌ AI 分析器未配置 API Key");
        return Ok(());
    }

    let news_text = news_response.to_context(5);
    let result = analyzer
        .analyze(&context, Some(&news_text))
        .await;

    // === 步骤 4: 输出分析结果 ===
    println!("\n" + "=".repeat(50));
    println!("【{}({}) 分析报告】", stock_name, stock_code);
    println!("=".repeat(50));
    
    println!("\n📈 核心指标:");
    println!("  综合评分: {}/100", result.sentiment_score);
    println!("  趋势预测: {}", result.trend_prediction);
    println!("  操作建议: {} {}", result.get_emoji(), result.operation_advice);
    println!("  置信度: {} {}", result.confidence_level, result.get_confidence_stars());

    println!("\n💡 核心结论:");
    println!("  {}", result.get_core_conclusion());

    if !result.risk_warning.is_empty() {
        println!("\n⚠️  风险提示:");
        println!("  {}", result.risk_warning);
    }

    if !result.key_points.is_empty() {
        println!("\n🎯 核心看点:");
        println!("  {}", result.key_points);
    }

    println!("\n" + "=".repeat(50));

    Ok(())
}
```

运行:
```bash
cargo run --example full_analysis
```

## API 文档

### 搜索服务 API

```rust
// 获取搜索服务
let service = get_search_service();

// 搜索股票新闻
let response = service
    .search_stock_news("600519", "贵州茅台", 5)
    .await;

// 搜索特定事件
let events_response = service
    .search_stock_events("600519", "贵州茅台", 
        Some(vec!["年报预告", "减持公告"]))
    .await;

// 多维度情报搜索
let intel = service
    .search_comprehensive_intel("600519", "贵州茅台", 3)
    .await;
```

### AI 分析器 API

```rust
// 获取分析器
let analyzer_mutex = get_analyzer();
let mut analyzer = analyzer_mutex.lock().unwrap();

// 执行分析
let result = analyzer
    .analyze(&context, news_context)
    .await;

// 提取结果
let conclusion = result.get_core_conclusion();
let advice = result.get_position_advice(has_position);
let emoji = result.get_emoji();
```

### 趋势分析器 API ⭐ 新增

```rust
use stock_analysis::{StockTrendAnalyzer, StockData, analyze_stock};

// 方法1: 创建分析器实例
let analyzer = StockTrendAnalyzer::new();
let result = analyzer.analyze(&stock_data, "000001");

// 方法2: 使用便捷函数
let result = analyze_stock(&stock_data, "000001");

// 准备股票数据
let stock_data: Vec<StockData> = vec![
    StockData {
        date: "2025-01-01".to_string(),
        open: 10.5,
        high: 10.8,
        low: 10.3,
        close: 10.7,
        volume: 1000000.0,
        ma5: None,  // 分析器会自动计算
        ma10: None,
        ma20: None,
        ma60: None,
    },
    // ... 更多数据（建议至少60天）
];

// 获取分析结果
println!("趋势状态: {}", result.trend_status);
println!("操作建议: {}", result.buy_signal);
println!("综合评分: {}/100", result.signal_score);

// 格式化输出
println!("{}", analyzer.format_analysis(&result));
```

详细文档请参考: [TREND_ANALYZER.md](TREND_ANALYZER.md)

## 配置说明

### 环境变量

在 `.env` 文件中配置:

```bash
# 搜索引擎 API Keys
BOCHA_API_KEYS=key1,key2
TAVILY_API_KEYS=key1,key2
SERPAPI_KEYS=key1,key2

# AI 分析 API Keys
GEMINI_API_KEY=your_key
GEMINI_MODEL=gemini-2.0-flash-exp

# 豆包 API (字节跳动) ⭐ 新增
DOUBAO_API_KEY=your_doubao_key
DOUBAO_BASE_URL=https://ark.cn-beijing.volces.com/api/v3
DOUBAO_MODEL=ep-20241230184254-j6pvd

# OpenAI 兼容 API (备选)
OPENAI_API_KEY=your_key
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_MODEL=gpt-4

# 通知渠道配置 ⭐ 新增
# 企业微信 Webhook
WECHAT_WEBHOOK_URL=https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx

# 飞书 Webhook
FEISHU_WEBHOOK_URL=https://open.feishu.cn/open-apis/bot/v2/hook/xxx

# 邮件 SMTP ⭐ 新增
EMAIL_SENDER=your_email@gmail.com
EMAIL_PASSWORD=your_app_password
EMAIL_RECEIVERS=receiver1@example.com,receiver2@example.com
SMTP_SERVER=smtp.gmail.com
SMTP_PORT=587
```

### 获取 API Keys

- **Bocha**: https://bocha.ai/
- **Tavily**: https://tavily.com/
- **SerpAPI**: https://serpapi.com/
- **Gemini**: https://makersuite.google.com/app/apikey
- **豆包 (Doubao)**: https://console.volcengine.com/ark ⭐ 新增
- **OpenAI**: https://platform.openai.com/api-keys

> 💡 **豆包配置指南**: 详细配置说明请参考 [DOUBAO_CONFIG.md](DOUBAO_CONFIG.md)

## 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test search_test

# 带输出的测试
cargo test -- --nocapture

# 运行被忽略的测试（需要真实 API Key）
cargo test -- --ignored
```

## 性能优化

### 并发搜索

```rust
use tokio::try_join;

// 并发搜索多只股票
let (result1, result2, result3) = try_join!(
    service.search_stock_news("600519", "贵州茅台", 5),
    service.search_stock_news("000001", "平安银行", 5),
    service.search_stock_news("300750", "宁德时代", 5),
)?;
```

### 批量分析

```rust
let stocks = vec![
    ("600519", "贵州茅台"),
    ("000001", "平安银行"),
];

let results = service
    .batch_search(stocks, 3, Duration::from_secs(1))
    .await;
```

## 故障排查

### 常见问题

1. **编译错误**
   ```bash
   cargo clean
   cargo build
   ```

2. **API Key 无效**
   - 检查 `.env` 文件
   - 确认 Key 没有过期
   - 查看日志输出

3. **网络问题**
   - 检查网络连接
   - 配置代理（如需要）
   - 增加超时时间

### 日志级别

```bash
# 调试模式
RUST_LOG=debug cargo run

# 只看错误
RUST_LOG=error cargo run

# 只看某个模块
RUST_LOG=stock_analysis::search_service=debug cargo run
```

## Python vs Rust 迁移指南

### 主要差异

| Python | Rust |
|--------|------|
| `dict` | `HashMap` |
| `list` | `Vec` |
| `None` | `Option<T>` |
| `try/except` | `Result<T, E>` |
| `async def` | `async fn` |
| `self` | `&self` / `&mut self` |

### 异步代码

Python:
```python
async def search():
    result = await api.call()
    return result
```

Rust:
```rust
async fn search() -> Result<Response> {
    let result = api.call().await?;
    Ok(result)
}
```

### 错误处理

Python:
```python
try:
    result = risky_operation()
except Exception as e:
    handle_error(e)
```

Rust:
```rust
match risky_operation() {
    Ok(result) => use_result(result),
    Err(e) => handle_error(e),
}

// 或使用 ?
let result = risky_operation()?;
```

## 贡献指南

欢迎贡献代码！请遵循以下步骤：

1. Fork 本仓库
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## License

MIT

## 联系方式

- Issues: [GitHub Issues](https://github.com/your-repo/issues)
- Discussions: [GitHub Discussions](https://github.com/your-repo/discussions)
