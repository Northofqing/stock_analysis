# A股自选股智能分析系统

基于 Rust 构建的全自动 A 股分析系统，覆盖数据采集、技术分析、AI 研判、量化回测、多渠道推送的完整闭环。

参考项目: https://github.com/ZhuLinsen/daily_stock_analysis — 感谢原项目作者

## 系统架构

```
股票代码 / 宏观 AI 推荐
        │
        ▼
  ┌─────────────────────────────────┐
  │       数据采集层                │
  │  RustDX(通达信) / Gtimg(腾讯)  │
  │  EastMoney(东方财富) / Tushare  │
  │       自动故障切换              │
  └─────────┬───────────────────────┘
            │  30日K线 + 实时行情 + 财务指标
            ▼
  ┌─────────────────────────────────────┐
  │           分析引擎层                │
  │  • 趋势分析：MA排列 / 支撑压力 /    │
  │    MACD / RSI / KDJ / 背离          │
  │  • 筹码分布 + 主力资金流            │
  │  • 日内分时 + 龙虎榜席位            │
  │  • AI研判：豆包 / OpenAI / Gemini   │
  │  • 多因子选股 + 回测引擎            │
  │  • 大盘复盘 + 龙虎榜分析            │
  └─────────┬───────────────────────────┘
            │  评分 / 操作建议 / 回测指标
            ▼
  ┌─────────────────────────────────┐
  │       输出层                    │
  │  SQLite 入库 / Markdown 报告   │
  │  PNG 图表 / 邮件 / 企业微信    │
  │  飞书 / Telegram / Pushover    │
  └─────────────────────────────────┘
```

## 核心功能

| 模块 | 说明 |
|------|------|
| **多源数据采集** | 通达信 TCP 直连 → 腾讯财经 → 东方财富 → Tushare，按优先级自动切换 |
| **趋势技术分析** | MA5/10/20/60 多头排列、乖离率、量能分析、支撑压力位识别、MACD/RSI/KDJ/背离、买点信号评分 |
| **筹码分布 & 资金面** | CYQ 衰减叠加模型重建成本分布（获利盘/主力成本/集中度） + 东方财富主力/超大单/大单净流入 + 日内分时形态 + 龙虎榜席位 |
| **AI 智能研判** | 豆包(Doubao) → OpenAI → Gemini 三级备选，输入技术面 + 筹码面 + 资金面 + 宏观新闻，输出情绪评分(0-100)与操作建议 |
| **新闻搜索** | Bocha / Tavily / SerpAPI 多引擎 + 多 Key 负载均衡，自动提取情绪与关键词 |
| **大盘复盘** | 上证/深证/创业板/科创50/沪深300 指数跟踪 + 板块分析 + AI 生成复盘报告 |
| **龙虎榜分析** | 抓取每日龙虎榜数据，评估机构/游资参与度，综合评分筛选优质标的 |
| **多因子选股** | 市值/ROE/PE/PB/换手率加权评分，自动排名并选出 Top N |
| **量化回测** | 模拟交易引擎，计算总收益率、年化收益率、最大回撤、夏普比率、胜率 |
| **多渠道通知** | 邮件(SMTP) / 企业微信 / 飞书 / 钉钉 / Telegram / Pushover / 自定义 Webhook |
| **定时调度** | 支持固定时间点 / 间隔执行 / 仅工作日模式，可通过环境变量或命令行配置 |
| **数据持久化** | SQLite + Diesel ORM，自动建表迁移，股票日线 / 分析结果 / 龙虎榜三表存储 |

## 项目结构

```
stock_analysis/
├── src/
│   ├── main.rs                   # 主程序入口（4种运行模式 + CLI参数）
│   ├── lib.rs                    # 库入口，导出所有模块
│   ├── pipeline.rs               # 分析流程调度器（数据→分析→回测→通知）
│   ├── data_provider/
│   │   ├── mod.rs                # 统一数据接口 + DataFetcherManager
│   │   ├── rustdx_provider.rs    # 通达信 TCP 直连（最快）
│   │   ├── gtimg_provider.rs     # 腾讯财经 HTTP API
│   │   ├── eastmoney_provider.rs # 东方财富 HTTP API
│   │   └── tushare_provider.rs   # Tushare Pro API
│   ├── analyzer.rs               # AI 分析器（豆包/OpenAI/Gemini）
│   ├── trend_analyzer.rs         # 均线趋势分析 + 买点信号
│   ├── search_service.rs         # 新闻搜索（Bocha/Tavily/SerpAPI）
│   ├── market_analyzer.rs        # 大盘复盘分析
│   ├── market_data.rs            # 市场指数与板块数据结构
│   ├── lhb_analyzer.rs          # 龙虎榜数据抓取与分析
│   ├── multi_factor_strategy.rs  # 多因子量化选股引擎
│   ├── backtest.rs               # 回测引擎（模拟交易 + 绩效指标）
│   ├── sharpe_calculator.rs      # 夏普比率计算
│   ├── chart_generator.rs        # PNG 图表生成（Plotters）
│   ├── notification.rs           # 多渠道通知服务
│   ├── database.rs               # SQLite 连接池管理
│   ├── models.rs                 # ORM 数据模型
│   ├── schema.rs                 # Diesel 表结构定义
│   ├── enums.rs                  # 枚举类型
│   └── bin/
│       └── lhb_query.rs          # 龙虎榜独立查询工具
├── migrations/                   # Diesel 数据库迁移
├── reports/                      # 生成的分析报告（Markdown）
├── data/                         # SQLite 数据库文件
├── .env.example                  # 环境变量配置模板
├── diesel.toml                   # Diesel ORM 配置
├── Cargo.toml                    # 依赖管理
└── CHANGELOG.md                  # 版本更新日志
```

## 快速开始

### 1. 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env 文件，填入 API Keys
```

最小配置只需设置：
```bash
STOCK_LIST=601179,000876,002182        # 自选股代码
GEMINI_API_KEY=your_key                # 至少配置一个 AI（豆包/OpenAI/Gemini）
SERPAPI_KEY=your_key                   # 新闻搜索
```

### 3. 编译运行

```bash
# 首次编译（自动下载依赖）
cargo build --release

# 运行分析
cargo run --release
```

## 命令行用法

### 四种运行模式

```bash
# 模式1: 单次分析（默认）- 自选股技术分析 + AI研判 + 通知
cargo run --release

# 模式2: 仅大盘复盘
cargo run --release -- --market-review

# 模式3: 龙虎榜选股分析
cargo run --release -- --lhb-mode
cargo run --release -- --lhb-mode --lhb-date 20260128 --lhb-min-score 70

# 模式4: 定时任务
cargo run --release -- --schedule --schedule-time "09:30,15:00" --weekdays 1,2,3,4,5

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

基于 Gemini/OpenAI/豆包 API 的智能股票分析系统。

- [完整文档](AI_ANALYZER.md)
- 决策仪表盘输出（JSON 结构化 + 自然语言两种模式）
- 豆包/OpenAI/Gemini 三级备选，自动重试和故障转移
- 结合新闻、宏观背景的综合分析

**注入给 Prompt 的上下文纬度**：
- 行情技术面：MA5/10/20/60、乖离率、MACD/RSI/KDJ、赋权量能、涨跌停研判、近期波动率
- **主力资金流（真实口径）**：东方财富 `fflow/daykline`——主力/超大单/大单净流入 + 近3/5日累计
- **日内分时形态**：冲高回落 / 尾盘跳水 / 尾盘拉升 / 高开低走 / 低开高走 等自动识别

- **筹码分布（CYQ 衰减模型）**：平均成本 / 主力成本（峰值） / 获利盘比例 / 90%&70% 成本区间 + 集中度 / 现价相对主力成本偏离
- **龙虎榜席位**：近30日上榜次数、机构/游资评分、席位解读与风险提示
- **宏观新闻 / 板块联动**：基于新闻搜索服务的实时消息面研判

###  数据库存储层模块 ⭐ 新增

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

## License

MIT

## 联系方式

- Issues: [GitHub Issues](https://github.com/Northofqing/stock_analysis/issues)
- Discussions: [GitHub Discussions](https://github.com/Northofqing/stock_analysis/discussions)
