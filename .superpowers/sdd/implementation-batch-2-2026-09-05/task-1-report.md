# Task 1 实施报告：不可变设计来源与校验接口

## 结果

- 实现提交：`53f3c87`（`docs: add immutable design source catalog`）。
- 新增九份不可变 v18/v19 设计来源、批准决策表、`design-source-catalog.v1.json`、Ruby 2.6 标准库校验模块、公开 CLI 与 CLI 驱动测试。
- 仅在 `docs/v19.x/README.md` 增加退役规则指针和目录/自声明版本标签冲突注记；未修改九份历史来源正文。
- 公共接口为 `ArchitectureDocs::SourceCatalog.validate(root)`，返回字符串错误数组。CLI 成功为 0、校验失败为 1、参数错误为 2。

## RED / GREEN 证据

1. 正常 fixture + 字节漂移：RED 为 CLI 文件不存在，1 run / 1 assertion / 1 failure；GREEN 为 1 run / 4 assertions / 0 failures。
2. 重复 source id/path：RED 为重复项仍 exit 0，1 run / 1 assertion / 1 failure；GREEN 为 1 run / 6 assertions / 0 failures。
3. 绝对路径、`..`、文件 symlink 越界：RED 返回 `source_missing` 而非 `source_path_invalid`，1 run / 3 assertions / 1 failure；GREEN 为 1 run / 9 assertions / 0 failures。
4. schema/status/必需字段与字段类型：RED 缺 `catalog_schema_invalid`，1 run / 3 assertions / 1 failure；增加 SHA 类型案后再次 RED，1 run / 21 assertions / 1 failure；GREEN 为 1 run / 21 assertions / 0 failures。
5. 批准表 Q1–108 缺失/重复：RED 为修改题号后仍 exit 0，1 run / 1 assertion / 1 failure；GREEN 为 1 run / 6 assertions / 0 failures。Q71 没有因 superseded 标记被删除，重复 Q71 明确失败。
6. source 缺失、批准表 SHA 漂移/缺失：RED 中缺失批准表错误误报为 `catalog_missing`，1 run / 9 assertions / 1 failure；GREEN 为 1 run / 9 assertions / 0 failures。
7. CLI 未知参数：RED 中位置参数错误返回 0，1 run / 2 assertions / 1 failure；GREEN 为 1 run / 2 assertions / 0 failures。
8. 缺失叶节点位于越界 symlink 目录下：RED 返回 `source_missing`，1 run / 12 assertions / 1 failure；GREEN 为 1 run / 12 assertions / 0 failures。

最终完整 Ruby suite：

```text
$ ruby scripts/architecture-docs/test/source_catalog_test.rb
........
8 runs, 66 assertions, 0 failures, 0 errors, 0 skips
Finished in 6.052424s
```

项目目录校验与语法：

```text
$ ruby scripts/architecture-docs/check-sources.rb --root .
source_catalog_valid
$ ruby -c scripts/architecture-docs/source_catalog.rb
Syntax OK
$ ruby -c scripts/architecture-docs/check-sources.rb
Syntax OK
$ ruby -c scripts/architecture-docs/test/source_catalog_test.rb
Syntax OK
```

## 校验覆盖

- fixture 不硬编码项目 source 数量，使用最小一项 source 和完整 Q1–108 两段表；所有 CLI 测试通过 `Open3.capture3`，所有输入位于独立 `Dir.mktmpdir` 根。
- 覆盖 source 缺失/漂移、批准表缺失/漂移、Q1–108 缺失/重复、重复 id/path、绝对/遍历/现存 symlink/缺失叶节点 symlink 越界、catalog schema/status/provenance、批准表和 source 必需字段/类型、未知及位置参数、公共 errors-array 接口。
- 项目级 catalog 共 9 项 source；批准表声明并实际覆盖 108 题。
- 未使用网络、gem、全局 cwd/env/logger，校验器不写输入文件。

## 导入哈希

| 路径 | 原 SHA-256 | 目标 SHA-256 |
| --- | --- | --- |
| `docs/v18.x/v18.1-strategic-gap-analysis.md` | `7f74b5abe4c20d6be099239878f27483a52d44cfa6ac126e1c750e916567c3f2` | 相同 |
| `docs/v18.x/v18.2-backtest-direction.md` | `946770a2bb66f1e9eeb90e87d4cd4f7b13434e1f377409ef891be84c5a5178f2` | 相同 |
| `docs/v18.x/v18.3-backtest-implementation.md` | `b63393561c17252524cbafabf322a5e4624b9e31b60a668af5b1334bc11283c2` | 相同 |
| `docs/v18.x/v18.4-factor-zoo-design.md` | `7cf4040e11698c4f67aef54d2471d855d00cd9f31d178b7dfc1676c762977ce7` | 相同 |
| `docs/v18.x/v18.5-production-readiness-design.md` | `7265e80311074f774ba2194c3728622bbf3e6b4f5f26499bdca8d76b7fff067f` | 相同 |
| `docs/v19.x/push-template-catalog.md` | `c6e0fc8bce6d4fe668222837052425a07a6db6918c68b5fc8121efe1beced4d2` | 相同 |
| `docs/v19.x/v19.0-operational-clarity-design.md` | `da8f141e2c5aee942ea29539e80ff267dac339ca678c4fe7b764df133dc69284` | 相同 |
| `docs/v19.x/v19.1-review-enhancement.md` | `26ca82982ebdc8c6cab9251dc00f250b57a33e5e90821db25705c87be23ad2ef` | 相同 |
| `docs/v19.x/v19.2-ai-analysis-improvement.md` | `ac7b2430ee5cf043314bd37aea622fbc16e42c27e4cea6bb9e50ef787b3b99ea` | 相同 |
| `docs/push-system/grill-decisions-2026-09-02.md` | `55354916a4b03401afa771e2f4e149bc1189222fc3c76d89aeb5ad79c086e794` | 相同 |

`v19.0` 原文件没有尾部 LF（21,728 bytes）。`apply_patch` 首次导入自动增加一个 LF，得到 21,729 bytes / `86edebf3665e90a1f3e33efbfc541f64a9f73480aed8f6cd7909ebded3138a4d`。按 controller 批准的单文件机械格式归一化例外，只截去隔离目标的该 LF；随后目标恢复 21,728 bytes / `da8...`，且 `cmp -s` 返回 0。原工作区文件未改动；此例外未用于其他文件。

## 文件与自审

- 提交包含 15 个任务文件：catalog、10 份导入件（九份 source + 决策表）、v19 README、模块、CLI、测试。
- 自审确认 source catalog 如实记录 v18.2–v18.4 自声明 v20.x、v18.5 自声明 v20.0；措辞仅称版本标签冲突。
- `push-template-catalog` 明确裁决为 `master@97f28b9` 的 57-kind 历史快照，不冒充当前 65-kind 目录。
- catalog 保持 `PROVISIONAL`，没有写 Foundation Ready 或生产就绪声明。
- controller 修改的 `docs/push-system/implementation-batch-2-2026-09-05.md` 未暂存、未提交。

## 关注与剩余门禁

- 两份原始不可变来源自身带行尾空格：v18.4 两处、v18.5 四处。原样导入要求禁止清理这些字节，因此裸 `git diff --check` 会报告它们。使用只关闭 `blank-at-eol` 检查的 `git -c core.whitespace=-blank-at-eol diff --cached --check` 返回 0；其余 whitespace 错误仍受检查。若整体门禁必须裸命令零输出，需要 controller 在不改变来源字节的前提下决定 attributes/门禁策略。
- 本任务没有运行 Rust tests/build（brief 明确不需要），也没有交付严格目录、RFC/WBS/HTML/CI/生产门禁；这些仍属于后续任务，不能由本地 source 校验通过替代。
