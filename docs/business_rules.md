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
| BR-006 | 过滤 | 基于真实胜率关停 0% 主题。`chain_rules.toml` 中 `enabled = false` 的规则在 `chain_mapper::map_chain_rules` 阶段被过滤, 后续推送不再命中。当前关停清单 (14 天回测, 0% 胜率): AI硬件-液冷 (6/0), AI硬件-CPO (11/0), AI硬件-MLCC (130/0), AI算力 (76/0), Rubin (32/0), 半导体-制造代工 (51/0), 创新药-CXO (15/0)。再启条件: 关键词集扩充或市场环境变化后, 跑 `winrate_simulator` 验证 ≥10% 胜率。 | `src/opportunity/chain_mapper.rs::map_chain_rules` + `config/chain_rules.toml` | `src/opportunity/chain_mapper.rs::test_br006_disabled_chains_excluded` + `src/bin/winrate_simulator.rs` | 2026-06-29 |
