# M0 生产投递只读样本（2026-09-29 01:08 CST）

范围：从 Desktop 外运行根的 `data/durable_delivery.sqlite3` 以 `sqlite3 -readonly`、`PRAGMA query_only=ON` 读取聚合状态；旧 monitor PID `17089` 仍运行。未读取消息正文、账户或证券明细，未写库、裁定、重发或变更物理 owner。本样本属于**运行中旧制品**，不是新 release 切换后的验收。

| 2026-09-28 PushKind | `Delivered` 决策 | 权威 `Accepted` sink result | 渠道数 |
| --- | ---: | ---: | ---: |
| AttributionDaily | 1 | 1 | 1 |
| CatalystReview | 1 | 1 | 1 |
| DataMode | 6 | 6 | 1 |
| NewsAiAnalysis | 26 | 26 | 1 |
| PositionReview | 1 | 1 | 1 |
| PreopenNewsHot | 1 | 1 | 1 |
| ReviewLhb | 1 | 1 | 1 |
| ReviewProviderTopN | 1 | 1 | 1 |
| TomorrowWatch | 1 | 1 | 1 |
| **合计** | **39** | **39** | 每 kind 各 1 |

查询按 `delivery_decisions.business_date='2026-09-28' AND state='Delivered'` 分组，并关联 `sink_results.result_kind='Accepted' AND authoritative_for_state=1`。这些行证明当日九种 counted kind 有持久化的渠道接受结果；没有逐条核对 provider 外部回执、业务 finalizer、冻结 52 Unit 的 producer 归属与 exact bytes，也不能推断其他 kind 已上线。

全库按 `delivery_decisions.state` 聚合：`Delivered=841`、`RejectedDurable=3987`、`ManualResolvedRejected=6`、`UncertainManualReview=78`。78 条未裁定项中 73 条为 2026-09-24 的 `DataMode`，其余为 2026-08-24 的 WatchlistTracking 1 条、2026-08-25 的 CloseCall 3 条和 2026-09-10 的 T0Advice 1 条。已有不确定状态保留原样，需外部处置证据及人工裁定；本次代码发布不改变它们。

进一步按 `delivery_attempts`、`sink_results` 只读聚合：78 条各有一次 attempt 和一次 `Uncertain` sink result，库内没有同决策的 `Accepted`/`Rejected` result；这些 `Uncertain` result 的 channel、provider、message ID、platform message ID 和 delivery audit ref 均为空。因此数据库自身无法判定外部是否已送达。M2 清账必须先取得目标渠道的独立处置证据，再走人工裁定；重启、日期已过或随后同类消息成功都不构成自动重发或拒绝旧决策的证据。

重跑命令（在生产运行根执行）：

```sh
sqlite3 -readonly data/durable_delivery.sqlite3 "PRAGMA query_only=ON; SELECT d.push_kind,COUNT(DISTINCT d.decision_identity),COUNT(s.result_event_identity),COUNT(DISTINCT s.channel) FROM delivery_decisions d LEFT JOIN sink_results s ON s.decision_identity=d.decision_identity AND s.result_kind='Accepted' AND s.authoritative_for_state=1 WHERE d.business_date='2026-09-28' AND d.state='Delivered' GROUP BY d.push_kind ORDER BY d.push_kind;"
sqlite3 -readonly data/durable_delivery.sqlite3 "PRAGMA query_only=ON; SELECT state,COUNT(*) FROM delivery_decisions GROUP BY state ORDER BY state;"
```

下一步：新 monitor 单实例切换后，对同一 DB 重查状态水位，并按每个 Unit 的 `producer → decision → sink result → finalizer` 做映射；真实外部回执与观察窗按路线图 M1/M2 另行验收。
