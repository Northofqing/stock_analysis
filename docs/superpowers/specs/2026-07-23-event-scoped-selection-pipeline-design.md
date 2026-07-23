# 事件级高质量选股影子链路设计

日期：2026-07-23

状态：Gate A — 三部分方案已确认，待文档复核

规则：数据红线 2.1、2.2、2.3、2.4、2.7、2.8、2.9、2.10；BR-092、BR-128、BR-137、BR-140、BR-152、BR-155、BR-156、BR-157

## 1. 目标与范围

建立一条单一、可追溯、只读影子的生产链路：

```text
MarketEvent
  → 事件级产业链映射
  → 精确证券关系证据
  → Magic TDX 同批市场证据
  → 数据质量与可交易性硬门
  → SelectionBatch
  → 不可篡改审计与 T0 快照
  → T0 收盘 / D+1 原始结果
```

本期采用“纵向切片 B”：先打通一条真实事件到真实证券、真实 Magic TDX 市场证据和后续结果的完整链路。输出只进入影子研究库和审计，不发送正式荐股消息、不写真实或虚拟订单、不修改账户风险、推送治理或下单阈值。

本期不承诺“新闻受益股”或“成功率”。正式影子候选的准确语义是：新闻事件明确提及、证券身份已验证、产业链已映射且市场证据完整的“事件关联证券”。板块成员只作为研究候选；AI 关系建议不具备事实资格。

## 2. 当前代码事实与根因

可复现定位命令：

```bash
rg -n "fetch_flash_titles|run_opportunity_scan|map_news_to_chains_ai|resolve_stocks|breakout_gate_candidates|run_post_close_candidates" \
  src/search_service/service.rs src/opportunity src/bin/monitor/main.rs
rg -n "prediction_tracker|news_outcome|winrate_simulator" src
```

当前路径存在以下结构性问题：

1. `run_opportunity_scan` 重新调用 `fetch_flash_titles`，只拿标题，未消费 NewsAggregator 已标准化的 `MarketEvent`。
2. `map_news_to_chains_ai` 把多个标题拼成一次输入，丢失 `event_id → chain → stock_code` 的逐事件身份关系。
3. `resolve_stocks` 把板块成分股当成事件候选，板块成员关系被误当作公司直接受益证据。
4. `breakout_gate_candidates` 在部分 K 线失败时继续处理，生产路径又使用多源 first-valid fallback，不能证明候选来自同一 Magic TDX 证据合同。
5. `run_post_close_candidates` 与候选面板没有完整生产接线，部分影子字段曾以 `0` 或本地当前时间占位。
6. `prediction_tracker` 只保存压缩后的方向/分数，无法重建当时事件、关系证据和市场特征；`winrate_simulator` 是已结算记录筛选器，不是真正的无未来数据回测。
7. `candidate_state::write_audit_jsonl` 和 `news_audit` 是普通追加文件，不具备跨进程锁、全链校验、哈希链和 `sync_data`，不能承担数据红线 2.7 的权威审计。

因此不能在旧扫描器上继续叠加分数或回测。必须先恢复事件身份、候选证据和时间切片，再讨论模型质量。

## 3. 已确认方案与备选

### 3.1 采用：事件级严格影子链路

- NewsAggregator 的标准化 `MarketEvent` 是唯一实时事件输入。
- 每条事件独立映射，不拼接多条新闻，不重新抓取标题。
- Magic TDX 是本链路唯一证券主数据、报价和 K 线来源。
- 首期只有 `DirectMention` 可以成为正式影子候选。
- 盘中记录、盘后复核和 D+1 结果使用同一候选身份。
- 只记录原始结果，不生成未经校准的成功率。

优点是身份和时间可追溯、可真实回测，并且可以在不影响交易和推送的情况下积累样本。代价是首期候选数量较少。

### 3.2 拒绝：修补旧 `run_opportunity_scan`

该方案保留标题重抓、跨事件拼接、板块成分泛化和多源证据混用，无法仅靠局部补丁恢复证据身份。

### 3.3 暂缓：直接构建全量公司产业链知识图谱

全量公司收入暴露、供应关系和客户结构可以提高受益关系精度，但数据合同、版本和维护范围显著更大。它应在本纵向切片积累真实评估样本后单独设计，不能阻塞首条可信链路。

## 4. 模块边界与公共接口

新增深模块 `src/selection/`，对生产调用方只公开一个入口和强类型结果：

```rust
pub async fn evaluate_market_events(
    batch: SelectionEventBatch,
    context: SelectionContext,
) -> SelectionRunOutcome;

pub struct SelectionEventBatch {
    pub events: Vec<MarketEvent>,
    pub source_attempts: Vec<SourceAttempt>,
    pub observed_at: DateTime<Local>,
}

pub enum SelectionRunOutcome {
    Completed(SelectionBatch),
    VerifiedEmpty(VerifiedEmptySelection),
    Unavailable(SelectionUnavailable),
}
```

`SourceAttempt` 必须逐 feed 记录成功、验证为空或结构化失败，不能只保留聚合后的事件列表。所有已知来源、质量、持久化和审计错误都必须收敛成带原因的 `Unavailable`，不能从公共入口泄漏成未归类错误或被调用方当作空结果。

内部模块：

| 模块 | 责任 |
| --- | --- |
| `selection::model` | 事件、关系证据、Magic TDX 证据、候选、拒绝和批次强类型 |
| `selection::pipeline` | 单事件编排、状态合并和结果语义 |
| `selection::relation` | 产业链配置快照、证券提及识别和关系分级 |
| `selection::magic_tdx` | 同一 blocking worker 内的 Magic TDX 证券主数据、报价、日线和 5 分钟线批次 |
| `selection::audit` | 生产/测试物理隔离的 SHA-256 hash-chain JSONL |
| `selection::outcome` | T0 收盘与 D+1 原始结果计算 |
| `database::selection` | 影子批次、不可变特征快照和追加式结果持久化 |

网络和阻塞协议只存在于 `selection::magic_tdx`。它必须使用 `tokio::task::spawn_blocking`，在 worker 内建立、使用并销毁客户端；不得在异步上下文创建或销毁嵌套 Tokio runtime。

纯关系、质量门、特征和结果计算不访问网络、数据库、日志或环境变量。测试通过窄的 `pub(crate)` 适配接口注入 `TEST_CODE_` 数据，不在生产路径加入 mock 分支。

## 5. 输入、关系证据与候选准入

### 5.1 MarketEvent 门禁

逐事件要求：

- 非空、稳定的 provider event identity。
- 非空标题和真实 provider。
- provider 发布时间存在、完整可解析、不在未来。
- 事件未被 upstream 标记 stale，且满足 BR-137 的来源日期规则。
- 本地观察时间只表示获取时间，不得替代 provider 发布时间。

当前 `MarketEvent.occurred_at` 同时承载完整 provider 时间和仅日期来源的本地观察时间，无法证明最后一条门禁。实现必须新增显式 `ProviderPublication`：保留 provider 原始发布日期，并在来源提供完整时间时额外保留时间戳；缺失或非法值保持 `None`。旧构造器和反序列化记录不得从 `occurred_at` 反推该证据。

陈旧、缺时间或非法事件在产业链映射前拒绝。`VerifiedEmpty` 只能表示完整来源确实没有命中，不能表示输入来源失败。

### 5.2 事件级产业链映射

每条事件只使用自己的标题、摘要和来源上下文匹配一次固定的产业链配置快照。配置加载必须验证：

- chain ID、名称和关键词非空。
- 同一 chain ID 不重复。
- 规则版本和完整文件 SHA-256 写入批次。
- 解析或验证失败使本轮 `Unavailable`，不得回退到内置词表。

首期生产链只接受可复验的规则映射。`AiProposed` 类型保留给未来语义提议，但首期没有 AI producer；未来即使启用，也必须先取得独立真实公司关系证据，否则只能拒绝或进入研究候选。

### 5.3 证券关系分级

```rust
pub enum RelationEvidence {
    DirectMention(DirectMentionEvidence),
    BoardMembership(BoardMembershipEvidence),
    AiProposed(AiProposalEvidence),
}
```

- `DirectMention`：事件中出现带边界的证券代码，或出现与当日 Magic TDX 证券主数据完全一致的公司名称；代码和名称必须相互验证。这表示“事件明确关联该公司”，不自动声明“受益”。
- `BoardMembership`：证券只因 Magic TDX 真实板块成员关系被发现。该关系仅进入 `research_only`，不进入正式影子候选。
- `AiProposed`：缺少独立公司事实时不得进入正式或研究候选。

同一事件内以 `(event_id, chain_id, stock_code, relation_version)` 形成候选身份。重复身份幂等；不同事件提及同一证券不得互相去重。正式候选按 provider 发布时间、event ID、chain ID、stock code 稳定排序，首期不增加二次 Top-N 截断。

## 6. Magic TDX 市场证据合同

### 6.1 单一主源与批次

`selection::magic_tdx` 在一次 blocking worker 中：

1. 获取并验证 Magic TDX 证券主数据快照。
2. 解析每条事件的直接证券提及。
3. 对候选获取未复权日线；交易时段额外获取实时行情和已完成 5 分钟线。
4. 为每条记录保留 provider、provider time（上游存在时）、本地观察时间、原始协议批次身份和稳定 batch ID。
5. 返回完整记录与逐票结构化拒绝，不调用新浪、腾讯、东财、Baostock 或旧 fallback。

证券主数据若没有 provider time，字段保持缺失并保留真实 `observed_at`；禁止用观察时间冒充 provider time。进程内缓存最多跨同一交易日，交易日变化后必须重新获取。

### 6.2 数据质量

正式影子候选至少需要 21 根已结算未复权日线，以计算 MA5/10/20、5 日收益和前 20 日量能基线。所有日线必须通过：

- OHLC 和成交量/额有限，价格大于 0，量额非负。
- `low <= min(open, close) <= max(open, close) <= high`。
- 日期严格递增、无重复，交易日连续性通过。
- 相邻有效收盘变化超过 ±20% 时显式拒绝并要求人工确认。
- 复权状态明确，拆分/分红造成的无法解释跳变不得静默计算。
- 最新已结算日线满足一个交易日新鲜度。

交易时段还必须具备：

- Magic TDX 实时报价及 provider time，年龄不超过 5 秒。
- 正价且合法的现价、昨收和当日高低。
- 已完成的 5 分钟线；时间戳无重复、处于交易会话且 OHLC/量额合法。

闭市不要求伪造实时行情。此时只形成 `PostClose` 市场窗口，并要求最新已结算日线满足新鲜度。盘中实时证据不完整的候选逐票拒绝，不用收盘价、成本价或其他来源顶替。

### 6.3 首期特征

首期只保存可复验原始特征和技术位置：

- 当前/结算价格。
- MA5、MA10、MA20。
- 5 日收益。
- 最新结算日成交量相对前 5 日、前 20 日均量的比值。
- 交易时段已完成 5 分钟累计量、同时间槽历史基线和量能节奏比。
- 价格相对 MA5/10/20 的距离。

缺少分母或历史样本时对应特征保持缺失，并使要求该特征的正式候选拒绝；不得写 `0`、`1` 或默认趋势。首期不根据这些特征计算预测概率或买卖分数，它们用于后续真实样本校准。

## 7. 运行状态与失败语义

### 7.1 批次状态

- `Completed`：至少一个输入事件完成全链评估，结果包含正式候选、研究候选和逐项拒绝。
- `VerifiedEmpty`：所有本轮相关 feed 都有结构化成功/验证为空证据，且配置、证券主数据和所需 Magic TDX 批次完整，但没有任何产业链或精确证券关系命中。
- `Unavailable`：事件批次身份、配置、Magic TDX 核心批次、持久化或审计不可用。

单票行情/K线失败可以隔离，其他完整候选继续；但批次必须声明排除数和原因。若所有潜在正式候选都因来源错误被隔离，结果是 `Unavailable` 或 `Completed` 加来源拒绝，不能写成 `VerifiedEmpty`。

### 7.2 结构化拒绝

每条拒绝至少包含：

- phase、reason code、rule IDs、retryable。
- event identity hash、chain identity hash、security identity hash。
- provider、provider time（真实存在时）和 observed_at。
- Magic TDX batch ID（已建立时）。

日志只输出批次数量和脱敏身份，不输出新闻正文、完整外部身份或账户信息。

## 8. T0、盘后与 D+1 结果

### 8.1 T0 不可变快照

正式影子候选写入 `selection_runs`、`selection_candidates` 和 `selection_feature_snapshots`。T0 快照包含当时事件、关系和市场证据的身份及原始特征，写入后不更新、不删除。

候选身份固定为：

```text
SHA256(
  domain
  + event_id
  + chain_id
  + stock_code
  + relation_version
  + feature_version
  + evaluation_market_date
)
```

重复相同事实幂等返回原记录；相同身份但内容哈希不同必须失败。

### 8.2 T0 收盘复核

复用 BR-139/BR-140 的 `post_session_review_scheduler`，不新增盘后调度器。交易日收盘数据完成后，以 Magic TDX 已结算未复权日线追加 `T0Close` outcome：

- 收盘相对 T0 价格收益。
- 收盘量与 T0 已完成量、前 5/20 日均量。
- T0 技术位置在收盘时是否仍保持。

结果写入新行，不覆盖 T0 快照。未结算返回 `ExpectedWait`。

### 8.3 D+1 原始结果

下一交易日收盘数据完成后追加 `D1Settled` outcome：

- 开盘、收盘、最高、最低相对 T0 基准的收益。
- 最大有利波动 MFE 和最大不利波动 MAE。
- D+1 成交量相对 T0、前 5/20 日均量的变化。
- D+1 收盘相对 MA5/10/20 的位置。

交易日由真实交易日历确定。休市、未到结算时间或数据未落定返回 `ExpectedWait`；来源失败返回 `Failed` 并保留重试资格。首期不把“盘中最高价曾上涨”单独定义为成功，也不更新旧 `prediction_tracker.hit`。

### 8.4 后续回测

回测只能连接 T0 不可变特征与其后追加的真实 outcome，按 provider、chain、relation type 和技术特征分组输出样本数、收益分布、MFE/MAE 和量能变化。样本不足时只显示原始统计，不显示成功率或上线结论。

## 9. 持久化与权威审计

### 9.1 影子研究库

`database::selection` 使用参数绑定和单事务追加：

- `selection_event_inbox`
- `selection_event_completions`
- `selection_runs`
- `selection_candidates`
- `selection_feature_snapshots`
- `selection_outcomes`
- `selection_visibility_receipts`

`selection_event_inbox` 保存通过基础 provider 门禁的规范化不可变事件和来源批次证据。聚合器已将事件标记 seen 后，选股链路必须先把事件写入 inbox，再做产业链和 Magic TDX 评估；临时来源失败不得使事件从待处理集合消失。进程若在聚合返回与 inbox 写入之间退出，聚合器的内存 seen 状态也随进程退出，来源可重新拉取；若写入失败但进程仍运行，调用方必须保留同一不可变批次并重试，不能把它计为已完成。

`selection_event_completions` 只追加终态：正式完成、经完整证据验证为空或永久拒绝。可重试的 `Unavailable` 不写 completion，后续轮次从 inbox 继续处理。每次评估尝试由 `selection_runs` 留痕；相同事件内容幂等，相同身份但内容哈希不同必须失败。

可空来源字段写 SQL `NULL`。所有价格、比例和收益在写入前验证有限性及业务范围。生产与测试数据库物理隔离；测试证券必须使用 `TEST_CODE_`。

批次、候选、特征和 outcome 行本身保持不可变。`selection_visibility_receipts` 是独立追加的可见性凭证；所有生产查询必须内连接该凭证，不能读取仅完成暂存、尚未完成权威审计的行。

### 9.2 Hash-chain JSONL

新 `selection::audit` 复用 BR-140 的审计合同，不复用普通 append writer：

- production/test 强制命名空间和路径隔离。
- 跨进程独占锁覆盖全链读取、验证、追加和 `sync_data`。
- 每行包含 schema version、固定 hash domain、previous hash、canonical record hash。
- 启动和每次追加前验证整个现有链；半行、尾部缺换行、未知字段或哈希异常均拒绝续写。
- 保存不少于五年，回滚不删除或重写历史。

执行顺序为：

1. 对通过基础 provider 门禁的规范化事件追加并同步 `Ingested` 审计，再幂等写入 inbox。
2. 在内存完成纯评估。
3. 追加并同步 `Prepared` 审计。
4. 以单个 SQLite 事务暂存不可变批次、候选和特征；这些行尚不可被生产查询消费。
5. 追加并同步带暂存内容哈希的 `Committed` 审计。
6. 以独立事务追加绑定 committed audit hash 的 `selection_visibility_receipts`。
7. 只有凭证写入成功才写终态 completion，并向调用方返回可消费的 `Completed/VerifiedEmpty`。

任一审计失败都返回 `Unavailable`。步骤 4 之后失败可以留下不可见的暂存行，但不得形成可消费候选；步骤 6 失败可以留下 committed audit，但生产查询仍不可见。重试必须按相同内容哈希幂等完成剩余步骤，不重复调用正式 sink，也不修改历史行。

## 10. 生产集成与旧模块关系

生产接线位于 NewsAggregator 每轮取得带逐 feed 状态的 `SelectionEventBatch` 后。NewsAggregator 新增强类型 batch API，旧 `Vec<MarketEvent>` API 只作为兼容包装，不能供选股链路判断 `VerifiedEmpty`。来源事实推送继续走 BR-137；选股影子链路独立消费同一不可变事件批次，不改变 critical/aggregate 推送和 seen 语义。

```text
NewsAggregator tick
  ├─ existing BR-137 / NewsFlashGate governance
  └─ selection shadow durable inbox + evaluate_market_events
       ├─ no sink
       ├─ no TradingBus
       ├─ no paper_trades
       └─ selection audit + shadow database only
```

| 旧模块 | 处置 | 原因 |
| --- | --- | --- |
| `news::aggregator::MarketEvent` | adopt and harden | 保留真实事件身份；新增显式 provider publication 证据，禁止从 observed time 反推 |
| `news::aggregator::NewsAggregator` | adopt and harden | 新增逐 feed 状态 batch，选股链路据此判断完整性并持久化待处理事件 |
| `opportunity::run_opportunity_scan` | remove production caller, retain temporarily | 阻断双轨；保留代码以便小提交迁移和历史工具兼容 |
| `search_service::fetch_flash_titles` | reject for selection | 标题重抓丢失标准事件身份 |
| `chain_mapper::map_news_to_chains_ai` | reject for production slice | 跨事件拼接且 AI 结果不是公司事实 |
| `chain_mapper::resolve_stocks` | reject for formal candidates | 板块成员不能证明事件直接关联 |
| `data_provider::fallback` | reject for selection evidence | 本链路要求 Magic TDX 单一主源、同批证据 |
| `prediction_tracker` | leave unchanged, no new writes | 旧 schema 不能保存完整 T0 证据 |
| `news_outcome` / `winrate_simulator` | leave unchanged | 不冒充本链路的 D+1 原始结果和回测 |
| `post_session_review_scheduler` | adopt | 复用唯一盘后耗时入口，满足 BR-152 |
| `candidate_state::write_audit_jsonl` | reject | 不满足 2.7 |
| BR-153 未提交的做T改动 | preserve untouched | 与本选股切片职责独立，避免覆盖用户工作树 |

影子链路使用默认开启的 `STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE` kill switch，仅控制该影子消费者。设为 `0` 后只停止新评估，不删除 T0、outcome 或审计，也不改变其他 NewsAggregator 消费者。

## 11. 测试与验收

### Gate B：实现与失败路径

测试必须覆盖：

- 每条事件独立映射，多事件不能串链或互相去重。
- stale、缺 provider time、未来时间和非法配置显式失败。
- 证券代码/名称不一致、名称非完整匹配被拒绝。
- 只有 `DirectMention` 成为正式影子候选；板块成员只能研究态。
- Magic TDX 核心批次失败、逐票失败、stale quote、坏日线、重复/断裂 K 线和缺量额的状态语义。
- 缺特征保持缺失，不出现静默 `0`。
- `Completed/VerifiedEmpty/Unavailable/ExpectedWait` 不互相冒充。
- 相同候选和 outcome 幂等；冲突内容拒绝。
- 审计锁、半行、坏哈希、`sync_data`/数据库失败均 fail closed。
- T0 快照不被 T0Close/D1Settled 覆盖。
- Magic TDX 客户端只在 blocking worker 生命周期内创建和销毁，不再触发 Tokio runtime drop panic。

### Gate C：仓库检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

任何 freshness FAIL 必须按 2.4.1 先执行：

```bash
bash tools/one_shot/backfill_daily.sh
bash tools/compliance/check.sh
```

### Gate D：覆盖率与真实只读证据

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo run --bin monitor -- --review
cargo run --bin monitor
```

常驻 monitor 验证必须有明确时间上限并优雅停止。真实验收只读 Magic TDX，不提交订单、不写 `paper_trades`、不调用正式荐股 sink。证据至少证明：

- 一条真实 `MarketEvent` 到候选/拒绝的事件身份连续。
- Magic TDX provider、batch ID、真实时间和特征完整。
- hash-chain selection audit 可全链复验。
- T0 影子快照与后续 outcome 使用同一候选身份。
- 缺数据和来源失败没有被补零或报告为“无候选”。

覆盖率门槛遵守仓库规定：总体至少 80%，核心选择、市场证据、持久化和审计链至少 95%。真实 D+1 尚未结算时只能报告 `ExpectedWait`，不能宣称 Gate D 完成。

## 12. PR 证据与回滚

PR 必须包含：

- `Refs: 本设计 §1-§12`
- `Data-Redlines: [2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.9, 2.10]`
- `OldModules:` 使用 §10 表格
- `Threshold-Proof:` 首期不修改推送/交易配置；21 根日线仅是本设计 §6.2 的特征完整性最低样本
- `Business-Rules: [BR-155, BR-156, BR-157]`
- 验证、覆盖率和真实只读 evidence
- `Rollback:` 如下

实现分支从设计提交开始创建 draft PR；Gate B、C、D 证据持续追加到同一 PR。Gate D 未完成或任一硬门失败时保持 draft/blocked，不得合并。

回滚：

1. 先关闭 selection shadow kill switch，停止新评估。
2. `git revert <implementation-commit>`，重新执行 build/test/compliance。
3. 仅停止经 PID 验证的 monitor，部署并启动一个 master 进程。
4. 不删除、截断、更新或重哈希 `selection_*` 表、selection audit、新闻、行情、账户、持仓、推送或交易证据。
