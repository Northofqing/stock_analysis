# 业务规则清单

> **MUST** (AGENTS.md §2.10) 任何"去重 / 互斥 / 过滤 / 排序 / 限额"的业务规则必须登记在本文件。每条含编号、规则描述、代码位置、测试位置、最后审核日期。
> **MUST** 任何新代码涉及上述类别, 必须先登记再写实现。

| 编号 | 类别 | 规则 | 代码位置 | 测试位置 | 末审 |
|------|------|------|---------|---------|------|
| BR-001 | 去重 | 同一只票近 3 个日历日最多推送 1 次 (注: 实现用日历日而非交易日, 简化 YAGNI — 周末不推送, 跳过周期可接受) | `src/opportunity/discover.rs::discover` | `tests/e2e_dedup.rs` | 2026-06-29 |
| BR-002 | 互斥 | 一条快讯最多命中 1 条产业链。例外: AI 推理明确给出 ≥2 条**独立**产业链 (chain 名之间无包含关系、关键词不重叠) 可保留。 | `src/opportunity/chain_mapper.rs::map_news_to_chains` | `tests/chain_exclusive.rs` | 2026-06-29 |
| BR-003 | 过滤 | 宏观新闻 (美联储/美股/汇率/大宗) 入 macro 通道, 不入 chain_mapper | `src/search_service/service.rs::fetch_flash_titles` | `tests/flash_filter.rs` | 2026-06-29 |
| BR-004 | 排序 | 推送 TopN 按 final_score 降序, 同分按发布时间升序 | `src/opportunity/mod.rs::run_post_close_candidates` | (已存在 `tests/ranking.rs` 验证) | 2026-06-29 |
| BR-005 | 限额 | 每天推送机会数 ≤ 5, 超过入候选池 | `src/bin/monitor/main.rs::run_opportunity_scan` | (e2e 覆盖) | 2026-06-29 |
| BR-006 | 过滤 | 基于真实胜率关停 0% 主题。`chain_rules.toml` 中 `enabled = false` 的规则在 `chain_mapper::map_chain_rules` 阶段被过滤, 后续推送不再命中。当前关停清单 (14 天回测): **6 个 enabled=false** = 5 真 0% (AI硬件-液冷 6/0, 半导体-先进封装 12/0, 消费电子 6/0, 新能源-电池 10/0, 稀土永磁 15/0) + 1 极低 (稀有金属 89/1 = 1.1%, 留观察). winrate_simulator 默认 blacklist 自动从 config 读 (修复 I-3 2026-06-29 codex review: 之前 spec 写 "5 关停" 是稀有金属 1.1% 仍 >0% 不算 "真 0%", 但仍差到关停). **BR-006 决策修订 (v9.2 PR-3 数据回灌后)**: 早期一轮关停的 6 个主题 (半导体-制造代工 / 创新药-CXO / AI硬件-MLCC / Rubin / AI算力 / AI硬件-CPO) 在 PR-2 修复 R-3 数据断层后, 真实胜率显现非 0%, 已重新开启并按真实胜率加权 (半导体-制造代工 64.6% → priority 88, 创新药-CXO 36.8% → priority 85)。教训: 早期 0% 是数据缺失假象, 不是真信号, BR-006 触发必须以 "真 verify 完成的样本" 为前提。再启条件: 跑 `winrate_simulator` 验证 ≥10% 胜率。 | `src/opportunity/chain_mapper.rs::map_chain_rules` + `config/chain_rules.toml` | `src/opportunity/chain_mapper.rs::test_br006_disabled_chains_excluded` + `src/bin/winrate_simulator.rs` | 2026-06-29 |
| BR-007 | 限额 | 季度 winrate review 流程。`tools/one_shot/winrate_review.sh` 每季度自动跑 (建议 crontab: `0 9 1 */3 *`), 跑 backfill + simulator + 输出 markdown 报告到 `reports/winrate_review_YYYY-MM-DD.md`。报告含: 关键指标 (胜率/推送数/pending) + simulator 决策建议 + 历史对比表 + 下一步 action items。AGENTS §2.4 数据驱动决策循环。 | `tools/one_shot/winrate_review.sh` | (脚本本身是测试 — 跑一次无副作用) | 2026-06-29 |
| BR-008 | 排序 | 数据驱动加权主题 (priority ≥ 90)。基于真实胜率, 当前加权: AI硬件-PCB 95 (66.7% / 120 推送), 半导体-设备 96 (73.2% / 41 推送), 新能源-固态电池 96 (100% / 7 推送), 新能源-钠离子电池 92 (33.3% / 21 推送), 新能源-锂电池 80 (45.5% / 11 推送)。priority 越大越优先 (BR-002 互斥下高 priority 先匹配)。新增加权: 跑 `winrate_simulator` 输出 ≥30% 主题 → 评估样本量 (≥15 推荐加权, 5-15 保守加权) → 改 `chain_rules.toml` priority + 加 AGENTS §2.9 边界证明注释 + commit. | `config/chain_rules.toml` (priority 字段) | `src/bin/winrate_simulator.rs` 输出 | 2026-06-29 |
