# 北交所 92 代码段日 K 跳变阈值一致性设计

日期：2026-07-23

状态：Gate A 已批准（全量测试失败后的根因回退修复）

规则：数据红线 2.3、2.5、2.9、2.10；BR-092、BR-131

## 1. 问题与根因

全量测试唯一失败为：

```text
monitor::data_quality::tests::test_max_gap_for_by_code_prefix
max_gap_for("TEST_CODE_920001") != 30.5
```

`max_gap_for` 已剥离强制测试前缀并识别历史北交所 `8/4` 代码段，但遗漏当前 `92` 代码段，因此 `92xxxx` 错误落入普通主板 `10.5` 分支。仓库的 BR-131 和 `market_data::infer_limit_pct` 已明确覆盖 `8/4/92`，当前失败是两条数据质量路径实现不一致。

## 2. 修复

只把 `code.starts_with("92")` 加入 `max_gap_for` 的北交所分支：

```rust
} else if code.starts_with('8') || code.starts_with('4') || code.starts_with("92") {
    30.5
}
```

不修改任何配置值、数值阈值、provider、数据内容或测试/生产身份规则。`TEST_CODE_` 仍只用于物理隔离测试身份；剥离后与生产代码执行同一映射。

## 3. 失败边界

- 未识别代码仍落入普通主板 `10.5`，保持现有行为。
- 该容差只允许真实板块涨跌停范围，不跳过 BR-092 的 OHLC、连续性、重复日期、来源涨跌幅和更大异常跳变校验。
- 不对缺失或坏行情补值。

## 4. 验证与回滚

```bash
cargo test --lib monitor::data_quality::tests::test_max_gap_for_by_code_prefix --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

回滚使用 `git revert <bse-fix-commit>`，不修改行情、数据库或审计数据。
