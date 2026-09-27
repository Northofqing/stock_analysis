# D17 / D20：证券交易资格与停复牌权威事实交接（2026-09-27）

## 交接结论

本地已为证券、有效日期、上市状态、当日限价制度和停复牌状态建立字段级 fail-closed 边界，但**尚无可调用的权威事实合同**。本交接请 `magic-market-data-rs` 核实可用的权威来源，发布可复现的请求/响应合同与 fixture；若事实源不能证明某字段或日期，须明确返回该字段的缺失、覆盖不足或失败，不能用名称、代码前缀、K 线断档、空数组或服务端当前时间补造。

这是合同与数据前置交接，不表示 VM 已实施、已部署或本地生产已具备 D17 / D20 能力。本地检查基线为 `1616c881`；服务端源码、运行制品和真实 RPC 身份均待 VM 回填。

## 已确认的本地边界

| 范围 | 当前代码事实 | 不能据此推断 |
| --- | --- | --- |
| D17 资格化事实 | `src/data_gateway/qualified_trading_facts.rs` 的请求绑定 `InstrumentId + effective_on`，生命周期、限价制度、停牌各有独立 `Available/Unavailable`；生产 `QualifiedTradingFactsGateway::acquire` 当前统一返回 `ContractNotDelivered`。`src/data_provider/limit_status.rs` 与 `src/bin/monitor/market_data.rs` 的生产价格门要求 Listed、Trading 和精确价格区间。 | 本地类型、测试 fixture 或旧 `LimitStatusCalculator` 的名称/ST 与代码前缀推算，不是权威业务事实。 |
| D20 日线断档 | `src/monitor/data_quality.rs::validate_daily_kline_quality_with_evidence` 枚举所有缺失交易日，要求 `QualifiedSuspensionEvidence` 对每一天给停牌覆盖，并要求复牌日为 Trading；没有证据时返回 `suspension_evidence_unavailable_v1`。证据只解释缺行，之后仍执行涨跌幅和 BR-171 人工确认门。`src/data_provider/halt_status.rs` 的 K 线 gap 只作诊断 candidate。 | 1 日、5 日或更长断档不能按长度认定停牌；有停牌证据也不自动批准复牌日价格跳变。 |
| 现有公开接口 | 当前 `client-bundle/bundle-metadata.json` 为 `2026-09-17.1`、源 revision `0980214...` 的公开快照；`grpc-external-api.md` 的 RPC 清单有 `SecurityMetadata` 和 `CorporateActions`，无独立的资格化交易事实/停复牌 RPC。公开文档“TDX 当前准入状态”称腾讯 `SecurityMetadata` 的板块为派生，未证明的上市日、涨跌停规则和规则版本为 unavailable；“当前实现状态”称扶摇 `SecurityMetadata` 为身份解析批次，板块/ST/上市日/涨跌停规则均未由该端点发布。 | `SecurityMetadata` 的 `ADMITTED`、姓名、可选上市日或公司行动，不等于完整的上市/退市、当日 ST、价位、限价上下界及停复牌覆盖。公开 bundle 的历史部署身份也不是当前 VM 进程证明。 |
| 已知候选案例 | `grpc_handoffs/share/2026-09-24-688277-suspension-gap.md` 记录了 688277 在 2026-07-16 至 07-29 的日线断档、07-30 复牌，以及公告线索；同文记录 688561 作为对照。 | 该历史记录可作为待核验 fixture 输入，不能代替 VM 本次的权威原件、逐日覆盖、当前服务响应或部署证明。 |

## VM 需要交付的合同

先确认现有权威来源是否能逐项证明以下语义，再决定复用某个 versioned RPC 或发布新 RPC。**字段名、枚举数字、JSON schema、默认 Provider 和服务端数据源均由 VM 从实现与原始证据给出；本交接不预设它们。**

1. **精确请求与身份。** 公布 RPC method、operation/schema/version、完整请求示例和响应 fixture。请求必须能表达交易所、六位证券代码、资产类别及 `effective_on`；若 D20 使用范围请求，须表达 inclusive `covered_from/covered_through`。说明是否支持历史日期、北交所、停牌期间、退市后与未来日期，以及超出范围的 typed 结果。响应必须绑定原请求的 instrument、日期/范围与 `request_id`，拒绝 B 票、A+B、重复 A 或无身份记录冒充 A。
2. **生命周期（D17）。** 对请求日给出可证明的上市前、已上市、已退市状态，上市生效日、退市首个失效日（若有）、证明“没有退市事件”的覆盖截止日及权威来源。只有上市日而没有覆盖到请求日的退市事实，不能推出 Listed。
3. **当日限价制度（D17）。** 提供该证券/有效日期的板块、ST 状态、最小价位、**权威发布或按权威规则精确确定**的上/下限价、规则版本/生效区间、币种与数值单位，以及计算/舍入依据。若来源只给参考比例、名称或派生板块，须保留该字段 unavailable；不得由下游猜上市初期、ST 变更、无涨跌幅限制日和板块转换的规则。源只证明一部分字段时应逐字段标注覆盖，不能把整个记录升级为可交易。
4. **停复牌（D20，同时供 D17 交易门使用）。** 对每个请求交易日或闭合日期范围给出权威停牌/复牌事件及起止边界；实际事件要有公告/事件稳定 ID、可校验原件摘要、来源时刻、采集时刻。无事件的日期也要有独立的查询覆盖证明。零停牌窗口只有在来源明确证明整个请求范围已查询完整时，才能作为“覆盖期内 Trading”；零记录、上游错误、分页未尽、只有公告标题或超出覆盖期，都应为 unavailable。复牌首日必须明确在覆盖内且为 Trading，不能仍落在停牌闭区间。
5. **共享批次与失败语义。** 回填 Provider、真实 source、`source_at`（允许明确不可得，但不能用 `observed_at` 代填）、`observed_at`、batch ID、contract/build identity、`complete` 的含义和所有分页/来源覆盖证据。明确区分已验证空、字段未发布、源不可达、权限不足、部分页、过期、相互冲突、身份不符。若不同字段来自不同权威源，要保留每个字段的来源与时效，不得包装成同一个无差别 complete。

## 可执行的请求/响应与 fixture 验收

VM 在发布合同后，用**合同中原样的请求 bytes/JSON**执行下表，保存脱敏请求、原始响应、解码结果及对应权威原件。下面的证券和日期是测试输入，不是对现有 wire 字段的猜测。

| Fixture | 请求输入 / 需要核对的响应 | 验收点 |
| --- | --- | --- |
| F1 精确身份 | A=688277、B=688561，分别请求相同有效日；另在测试夹具模拟“请求 A，返回 B / A+B / 重复 A / 空批”。 | `request_id`、证券交易所+代码+资产类别及日期完全匹配；错误 cardinality/identity 不得降为 A 的成功事实。空批须有独立、可验证的覆盖语义，否则 unavailable。 |
| F2 权威停复牌 | 候选 688277：请求覆盖 2026-07-15 至 07-30，并核对 07-16 至 07-29 的缺失交易日、07-30 复牌；以 688561 同范围作对照。 | 每个缺失交易日都有权威停牌覆盖；07-30 在覆盖内且为 Trading；提供公告/事件 ID、原件 hash、来源时间。若 VM 源只能证明部分日期，返回 partial/unavailable，不扩充窗口。此案例以既有手工线索为候选，须重新核实。 |
| F3 覆盖与空结果 | 对同一证券分别请求：全覆盖但无停牌、只覆盖断档一部分、请求日在覆盖外、上游返回零行/分页不完整。 | 只有明确全覆盖且源证明无事件才能认定 Trading；部分覆盖/零行无证明/缺页均不得解释日线缺口。 |
| F4 价格与生命周期 | 各选择一只具原始证据的主板、创业板、科创板、北交所证券，并覆盖 ST/非 ST、上市前/已上市/退市、无涨跌幅限制或规则转换日（源支持时）。 | 对每个证券+日期比对权威原件中的状态、tick、上下限、规则版本和覆盖日；不可得的板块/日期/制度字段保持字段级 unavailable，绝不从代码/名称推断。 |
| F5 证据冲突与时效 | 固定 fixture 注入源时间晚于采集时间、过期覆盖、重复/重叠停牌窗、相互冲突事件或不同证券。 | 返回 typed conflict/stale/identity-mismatch；不填默认值、不隐藏失败、不产生可交易或可解释断档的成功结果。 |
| F6 端到端接收 | 从 VM 实际监听地址发一次合同内真实 RPC；记录 Health 的部署身份、请求/响应、分页完整性和服务日志/trace，再交本仓复测。 | 源码 commit、descriptor/bundle、运行 binary hash 与 Health 身份相符；本地下游只在 identity、日期、覆盖、证据都通过时接线。`complete=true` 不能仅指“已解码源声明的行”，须另证明所请求事实的覆盖。 |

VM 回执至少包含：权威数据源及其可用范围、唯一合同包位置和 hash、每个 fixture 的实际结果、定向测试命令及结果、服务端 commit、构建/部署时间、监听地址、PID、binary hash、Health build identity，以及仍不可交付的字段/日期清单。若目前没有足够权威来源，请明确给出 `unavailable` 的字段与原因；本地安全门继续保持关闭。收到合同后，本仓负责消费端映射、字段级 admission、BR-092/BR-171 链路与真实 RPC 复核，不要求 VM 修改本地业务规则。
