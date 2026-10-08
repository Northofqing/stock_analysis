# H16 独立复审 P2 修复验证

已读主任务 `.planning/2026-10-08-retained-scope-closeout/weekly-review.md`，修其中两项 Spec P2，并纳入主任务真实只读源行发现的 extent 显示边界修正。

1. `classify` 在有效原 pred_date 核验后立即确定 weekly_origin；随后坏 target/id/direction 仍归该原周 invalid。新增回归覆盖三类坏字段、三种 horizon、本周/历史计数与原日期保留。修前本周 rows 为 0（应为 3），修后通过。
2. 原 paper/audit 以完整上海时间和 observed_at 比较；未来原行独立保留 `future_timestamp_rows` 与 `future_timestamps`（原 row ID/上海时间），排除本期状态/未成交/退出/尝试。全量原 Filled 与最新时间明确含未来行、非截至观察时刻。effective 保留现有按日全账本读端；任一返回有效成交晚于观察时刻使整个可靠 paper unavailable，原因 `future_effective_fill`，不自行裁剪经济账本。新增同日 18:00 对 16:00 回归，覆盖原成交、终态未成交、audit、未来诊断和可靠费用/净收益不可用。修前 Filled 本期计入 2（应为 1），修后通过。

audit 原诊断使用故意缺链的原记录，不制造完整审计链；未来 effective 拒绝则用独立有效 LegacyNoTerminal 买/卖原件核验。首次联合 fixture 被既有缺链完整性先拒绝，已按这两种真实证据边界拆分 fixture，未修改领域完整性规则。

主任务真实源行发现状态表原件是日线推断状态，原行数不等于独立资格。仅将 Markdown extent 标签改为日线表/状态表原始行（未核验资格），补 `source_extent_boundary` JSON 字段：原表行数/可读性不授予独立资格，推断状态/OHLC 不能证实生命周期或历史可用时刻/PIT。在既有 JSON/Markdown 回归增加边界断言；未更改窗口资格、统计或接纳推断状态。本工作树未访问真实库。

最终执行：

```bash
cargo test --offline --jobs 2 \
  --target-dir /Users/zhangzhen/.local/share/stock-analysis-candidates/retained-scope-closeout-20261008/weekly-test-target \
  --bin weekly_outcome_review -- --test-threads=1
git diff --cached --check
```

**15/15 通过，0.39 秒，exit 0**；结果见 `rust-tests.log`。两项修前失败分别保存在 `p2-origin-red.log`、`p2-future-red.log`，第一项单独通过为 `p2-origin-green.log`。最终源码逐文件/集合 SHA-256 见 `p2-tested-source.json`。源码集合包含 CLI、报告模块、测试；共享库保持工作树基线不变。staged diff 检查通过。

Python wrapper、launcher 与 plist 没有修改，复用原 `/usr/bin/python3` 3.9.6 的 8/8 测试与语法/plist 证据，不重复运行。资格/null/情景费用/只读与 30 秒 backup 边界保持原实现。没有普通 build、release、部署、provider/sink、正式 DB 或消息动作；最终普通制品和真实库报告仍由主任务验证。
