# CI 映射式触发识别修复

日期：2026-09-08。状态：**本工具修复完成**；源码提交 `80d0fb2872e2b91a088621bebbfbaf59c6861c6c`，最终定向验证及独立复审通过。

## 修复的问题

仓库真实 `.github/workflows/ci.yml` 使用映射式 `pull_request` 与 `push.branches: [main, master]`。文档 RFC 校验器原来只接受标量或序列触发器，因此即使存在正确检查步骤，也会错误返回 `ci_rfc_gate_missing`。本次修改仅影响识别，不修改真实 CI 的触发范围、权限或步骤。

`scripts/architecture-docs/rfc_spec.rb` 保留 AST 字面 `on`、唯一键和原有执行安全检查，增加受限映射配置校验：识别三类已有事件的空配置，以及 push/pull_request 的正向字面 branches 列表；排除组合、未知过滤器、错误类型、重复、表达式、标签、别名及执行绕过继续拒绝。

## 实际验证

- 通过现有 `check-rfc.rb --check` CLI、真实 CI 的临时副本取得行为反例；副本只增加检查步骤。首次失败确为额外的 `ci_rfc_gate_missing`，不是编译/fixture 错误。
- 最小修复后同一测试通过，并验证真实 CI 字节未变。
- 初版独立审查发现正向分支可被后置排除全部取消，却仍被视为有效门禁。两个实际 CLI 反例先得到 2 项失败，再收紧为仅支持正向过滤；当前真实 main/master 配置仍被识别。
- 修复后最终相关测试合批 **95 runs / 614 assertions / 0 failures / 0 errors / 0 skips**，41.541205 秒。覆盖新增映射、取消匹配反例和原标量/序列/字面 on、重复键、runner、条件、命令及执行绕过。
- 两份 Ruby 语法、diff 检查通过；八份冻结输入检查返回 `rfc_inputs_valid`。
- 独立限定复审确认 I1 已关闭，无新增 Critical / Important / Minor，结论 Approved。提交字节与最终审查快照一致。

完整输出与冻结差异保留于 `.superpowers/sdd/2026-09-08-ci-mapping-trigger/`。未运行整份 SQL/RFC suite，也未执行 GitHub CI。

## 仍未交付

此修复不提供通用离线 HTML builder、RFC HTML、统一 `check.rb`、真实 CI 安装/运行或生产批准；PROVISIONAL 等原有阻断不被移除。不因局部识别修复关闭整个文档工具或上线目标。
