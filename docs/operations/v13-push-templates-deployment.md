# v13 推送模板 生产部署指南

> **目的**：v13 spec 实施完成 (177/177 tests) 后，生产环境部署步骤
> **创建日期**：2026-07-06
> **适用版本**：v13.7 (commit `22780ff`)
> **关联文档**：
> - 设计 spec: `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md`
> - 实施日志: `docs/superpowers/specs/2026-07-06-v13-push-templates-impl-log.md`
> - 实施 plan: `docs/superpowers/plans/2026-07-06-v13-push-templates-impl.md`

---

## 0. 部署前 Checklist

| 项 | 验证 | 命令 |
|---|---|---|
| 编译通过 | ✅ | `cargo build --release` |
| 测试通过 | ✅ | `cargo test --bin monitor` (177/177) |
| 合规检查 | ✅ | `bash tools/compliance/check.sh` |
| DB 已迁移 | ⏳ 待生产 | `diesel migration run` (如有新表) |
| 配置 sync | ⏳ 待生产 | `git pull && cargo build --release` |
| 推送通道 | ⏳ 待生产 | `magiclaw` 守护进程运行 + 凭证有效 |
| 监控数据源 | ⏳ 待生产 | chain_daily / sector_monitor / candidate_panel / virtual_observation 全部就绪 |

---

## 1. 部署步骤

### 1.1 拉取最新代码

```bash
cd /path/to/repo
git pull origin master_07_04    # 拉取 v13.7
git log --oneline -5             # 验证最新 commit
# 期望: 22780ff feat(v13.7): dispatcher_log 可观测性
```

### 1.2 编译发布版本

```bash
cargo build --release --bin monitor
# 输出: target/release/monitor (~80MB stripped)
```

### 1.3 准备数据目录

```bash
mkdir -p data/virtual_observation
# 数据源 DB (Diesel):
# - chain_daily (主链 + stocks JSON)
# - account_mode_log (T-01)
# - paper_trades (T-10 虚拟盘)
# - agent_logs (T-12)
# 由 diesel migration run 初始化
```

### 1.4 测试 dry-run

```bash
# 验证 6 dispatcher 都能加载 (不实际推送)
./target/release/monitor --push-dry-run
# 期望: 6 行 log_dispatcher_attempt (success=true 或 error 非空)
tail data/dispatcher_log.jsonl
```

### 1.5 单次实际推送 (09:00 测试)

```bash
# 09:00 触发 P-01
./target/release/monitor --push

# 验证推送成功
# - 飞书/bark 收到推送
# - dispatcher_log.jsonl 记录 success=true
tail -1 data/dispatcher_log.jsonl | jq .
# {"ts":"2026-07-06T09:00:00.123","kind":"P-01","success":true,"snapshot_size":3,"error":""}
```

---

## 2. cron 调度配置

### 2.1 crontab 示例

```bash
# 编辑 crontab
crontab -e

# 添加 (工作日 09:00-19:00 每 90 分钟推送一次)
# 0 9 * * 1-5  /path/to/monitor --push
# 30 10 * * 1-5 /path/to/monitor --push
# 0 11 * * 1-5 /path/to/monitor --push
# 30 14 * * 1-5 /path/to/monitor --push
# 0 19 * * 1-5 /path/to/monitor --push
```

### 2.2 log 轮转 (按日)

```bash
# /etc/logrotate.d/monitor_push
/path/to/repo/data/dispatcher_log.jsonl {
    daily
    rotate 7
    compress
    missingok
    notifempty
    create 0644 user group
    postrotate
        # (可选) 推送到日志服务
    endscript
}
```

---

## 3. 监控查询

### 3.1 实时查看

```bash
# 实时跟踪
tail -f data/dispatcher_log.jsonl | jq '.'

# 统计
echo "=== 今日推送统计 ==="
TODAY=$(date +%Y-%m-%d)
grep "\"ts\":\"$TODAY" data/dispatcher_log.jsonl | \
  jq -r '"\(.kind): \(if .success then "✓" else "✗" end) (\(.snapshot_size) items)"' | \
  sort | uniq -c
```

### 3.2 失败告警

```bash
# 1 小时内失败 > 3 次 → 告警
RECENT_FAILS=$(grep "\"success\":false" data/dispatcher_log.jsonl | \
  jq -r 'select(.ts > (now - 3600 | todate))' | wc -l)
if [ "$RECENT_FAILS" -gt 3 ]; then
    echo "ALERT: $RECENT_FAILS dispatcher failures in last 1h" | \
        mail -s "monitor push failures" ops@example.com
fi
```

### 3.3 快照大小监控 (数据源健康度)

```bash
# snapshot_size = 0 → 数据源异常
grep "\"snapshot_size\":0" data/dispatcher_log.jsonl | \
  jq -r '.kind + " at " + .ts' | tail -20
```

---

## 4. 6 dispatcher 调度矩阵

| 时点 | 触发 dispatcher | 数据源 | snapshot 期望 |
|---|---|---|---|
| 09:00 | P-01 盘前热点 | chain_daily | clusters.len() > 0 |
| 10:30 | I-01 盘中轮动 | sector_monitor + sector_score | 3 (tech/power/robot) |
| 10:30 | I-02 新闻催化 | chain_daily + GtimgProvider | stocks.len() > 0 |
| 10:30 | I-03 涨停扩散 | chain_daily (简化) | supplements + 1 |
| 10:30 | D-01 新闻驱动 | chain_daily + 5 P5 源 | source_count=5 |
| 19:00 | A-01 虚拟仓复盘 | virtual_observation JSON | 1 |

---

## 5. 回滚方案

### 5.1 立即回滚 (推送停止)

```bash
# 1. 注释掉 cron
crontab -e
# (注释所有 monitor --push 行)

# 2. 清空 dispatcher_log (可选, 重置)
# rm data/dispatcher_log.jsonl
```

### 5.2 版本回滚 (代码回退)

```bash
# 回滚到上一个稳定版本 (例如 v13.4)
git log --oneline -10
git revert <unstable-commit-sha>
# 或硬回滚
git reset --hard <stable-commit-sha>
cargo build --release
```

### 5.3 数据修复 (PushKind 注册修复)

如发现某 PushKind 行为异常:
```bash
# 1. 查 dispatcher_log 找失败模式
grep "I-01" data/dispatcher_log.jsonl | jq 'select(.success==false)'

# 2. 临时禁用该 dispatcher (改 main.rs run_daily_pushes)
# 注释对应 dispatch_*_daily() 调用

# 3. 提 PR 修复 + 重启 cron
```

---

## 6. 故障排查指南

| 现象 | 原因 | 解决 |
|---|---|---|
| 全部 dispatcher success=false | 数据源 DB 异常 | 查 diesel 连接 + DB 状态 |
| snapshot_size=0 | 数据源空表 | 查上游 fetcher (chain_monitor / news_monitor) |
| 部分 dispatcher 失败 | 单源异常 (如 GtimgProvider 网络) | 查 dispatcher_log 错误字段 |
| Cron 不执行 | crontab 错误 / 路径错误 | `crontab -l` + 路径验证 |
| 推送通道失败 | magiclaw 守护进程 / 凭证过期 | 查 magiclaw 日志 |

---

## 7. 监控告警建议

```bash
# 部署 grafana + prometheus (高级)
# 数据源: 解析 dispatcher_log.jsonl → prometheus pushgateway

# 简单告警 (低运维成本)
*/5 * * * * /path/to/monitor --check-health  # 5 分钟健康检查
```

---

## 8. 部署后验证

| 验证项 | 通过条件 |
|---|---|
| 9:00 推送 | dispatcher_log 含 P-01 success=true |
| 10:30 推送 | 含 4 个盘中 dispatcher success=true |
| 19:00 推送 | 含 A-01 success=true |
| 用户收到推送 | 飞书/bark 通道正常 |
| 日志轮转 | logrotate.d 配置生效 |

---

## 9. 关联 PR 模板

生产部署前需 PR 合并：
- v13 spec 实施 (已完成)
- cron 配置 (运维 PR)
- logrotate 配置 (运维 PR)
- 监控告警 (可选, 运维 PR)

**部署后第一周**：每日检查 dispatcher_log.jsonl，统计 success rate 与 snapshot_size 分布。
**第二周起**：根据数据优化 6 dispatcher 触发时间窗与 snapshot_size 阈值。
