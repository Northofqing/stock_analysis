# schema14 隔离副本只读核验（2026-10-07）

## 用途与范围

`durable_schema14_snapshot_check` 是平台新版持久投递扩展的发布预检。调用方必须提供已完成、稳定的隔离 SQLite 副本；没有默认生产路径。源码检查入口为 `durable_delivery::inspect_schema14_extensions`，仅接受主库真正只读、无附加数据库、无临时对象且未处于调用方事务的连接。

核验复用 coordinator 的既有 schema12/13/14 校验器：精确 G5b/P05 扩展目录、原内容哈希与规范字节、原 owner、修订事件链、完成回执和 baseline 关联，并执行 SQLite quick_check 与外键检查。输出包括 17 张扩展表的行数、扩展目录摘要和明确的观察范围。它不打开生产 coordinator、不初始化或迁移数据库、不调用 provider/sink、不发行执行能力或生产批准。

## 运行

从已验证的候选源码构建该目标，再对受控副本运行：

```bash
cargo run --locked --offline --bin durable_schema14_snapshot_check -- \
  --isolated-snapshot /absolute/path/to/isolated/durable_delivery.sqlite3
```

参数的文件必须是 regular file；叶子链接与 `-wal`、`-shm`、`-journal`（含悬空链接）均拒绝。CLI 使用 `mode=ro&immutable=1`，避免创建 WAL/SHM；前后检查文件身份、大小、时间与全部文件 SHA-256，漂移或缺少内容即返回失败。`immutable` 不读取 WAL，因此不能把运行中的 WAL 主文件直接当副本。应通过受控、完整 SQLite 快照取得副本，保留原件与获取记录，不能只拷贝主文件或删除其 WAL。

扩展目录摘要对按 `(type,name,tbl_name)` 排序的原 SQL 字段逐项使用 8 字节大端长度加 UTF-8 字节计算 SHA-256；只包含有 SQL 的 `g5b_*`、`p05_*` 对象。摘要用于比较实际检查输入，没有批准语义。

## 验收与后继

定向回归覆盖：实际 counted 子项已接受且完成的非空历史、SQL 合法但缺实际修订事件、错版本、扩展目录缺失/额外对象、临时 core 表遮蔽/附加数据库、可写 query-only 连接和调用方已有事务，以及 CLI 的副本链接/sidecar 拒绝。实际执行结果另记 rollout 文档与本机原始回执；本页的测试范围不是通过声明。

本检查不替代 `tools/release/verify_br194_review_join.py` 的外部不可变审计、counted push 与 delivery audit 完整关联。原 BR-194 工具保持 schema9 的明确限制；当前源码的扩展检查通过也不能直接扩大该限制。新 schema14 的完整 join、旧非空生产历史迁移、v1/v2 财务兼容回退、精确激活审阅、动态数据/资金/Uncertain 门禁及自然观察仍分别验收。
