# R-08 FuturesDelivery：虚拟机服务端与本地消费端交接（2026-09-27）

## 当前切换状态（2026-09-28）

VM 隔离树已提交 `69c6ef198f411753143363a413a272156dcfc331` 并部署到 `10.211.55.3:50051`，运行进程 PID `18008`。公开 bundle 版本 `2026-09-27.1` 的 `manifest.sha256` 为 `d77b979205fcf0c65f6f92ad1cb245f00d58881917a8895bc667f28ad43dc11e`；实际运行服务二进制 SHA-256 `b22766603c8371a8ad4685c38872f7536ddc36519d4321ecfd3f8f72feea8dac`、raw descriptor SHA-256 `0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`，与 Health 及公开 metadata 相符。本地只同步 manifest 列出的九个公开文件，`shasum -a 256 -c manifest.sha256` 九项均 OK；原有连接凭据留在本地，不来自公开包，也未纳入提交。

本地已提交 `1cc28f42`：独立的 `cffex_planned_contract_month` 只读入口、四产品与批次证据校验、单独的 `R-08-cffex-planned-calendar` 审计身份，以及 `grpc_bundle_probe --native-operation futures-delivery --year 2026 --month 9`。R-08 消费端定向测试 5/5、release `monitor` 与 `grpc_bundle_probe` 构建均通过；本地真实探针得到 Health 身份匹配及四条 `Planned` 记录，`confirmed_delivery=false`。原 `cffex_contract_month` 确认型 EventCalendar 入口仍在 RPC 前返回 `cffex_confirmed_delivery_authority_unavailable_v2`，不授权“官方交割通知”发送。

构建复现依赖同版公开 bundle：`client-bundle/` 整目录被 Git 忽略，`build.rs` 会逐字节编译其中的 `market.proto`。此版客户端 descriptor SHA-256 为 `59158661146ff429f092c49584601147080b9e631e941e4acb2d96fe7a22bf7b`，与冻结历史解码器的 `5ba0fa3b2fa450e74bdcc8cb5f163348a6ca90df3f3626d1d8f2ec27137f5edb` 不同。新机器/CI 应从 VM 的 `C:\DevelopFile\magic-market-data-rs\.worktrees\r08-futures-delivery\target\r08-client-bundle` 仅复制 manifest 所列九个公开文件及 manifest，先在 `client-bundle/` 目录运行 `shasum -a 256 -c manifest.sha256`，再编译；不要用旧 bundle 编译新 pin，也不要复制凭据文件。

## 现状与责任边界

- 本地基线 `c67e4f6a`：`FuturesDeliveryGateway::cffex_contract_month` 在业务 RPC 前返回 `futures_delivery_contract_unavailable_v1`；`GrpcSource::futures_delivery_async` 也禁止绕行。R-08 不会把未知范围或空批次写成“无交割”。
- 虚拟机仓库 `C:\DevelopFile\magic-market-data-rs` 的检查基线为 `0554654`，工作树另有新闻/content-discovery 未提交修改，必须保留。`magic-market-core/src/calendar.rs` 的 `FuturesDeliveryRequest` 要求 `year/month`；`magic-market-composition/src/grpc_production.rs` 按此类型解码；`magic-exchange-rs/src/cffex.rs` 的正式固定日历只接受 2026 年，每月返回 IF/IH/IC/IM 四个合约。
- 本地公开 `client-bundle/grpc-external-api.md` 只列 request schema 名，未冻结字段、覆盖年限和空结果语义。旧本地 `{}` 请求形状与当前服务端必填年月不一致。历史 `internal` 失败尚不能仅据此归因。

## 已派发给虚拟机侧

当前上游执行任务是远程 Codex `01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`，在被忽略的 `.worktrees/r08-futures-delivery` 隔离树工作；原任务 `01a0d3f4-eb49-7bb1-a09e-462245d3e715` 因托管工作树审批挂起，已收到停止指令。当前任务负责核对真实 wire，发布唯一版本化合同与脱敏 fixture，必要时修复服务端问题，完成定向测试、构建、部署和真实 RPC 验证。不得覆盖或顺带提交主工作树新闻改动，也不得扩展未经证实的年份或制造 `VerifiedEmpty`。

### 2026-09-27 阶段性 RPC 与来源异常

部署前的真实 `FuturesDelivery` RPC 使用 ExternalV1 `{"year":2026,"month":9}`、`preferred_provider=Cffex`，返回 `ADMITTED`、`complete=true`、IF/IH/IC/IM 四条、`batch_id=cffex-equity-index-delivery-2026-v1:09`；单条 schema 为 `magic.market.futures_delivery_event` v1，`observed_at` 是 Unix 秒小数字符串，`source_at` 缺失。2027-09 真实请求返回 `UNIMPLEMENTED`，明确仅覆盖 2026 年。这些是**旧运行进程的部署前证据**，不能代替最终提交的 Health 身份和部署后 RPC。

该响应所有记录的 `notice_url` 都是 `https://www.cffex.com.cn/jystz/20251217/46425.html`。2026-09-27 本地 HTTP HEAD 对同一路径得到 302 → 404；[标注该原始链接的交易所通知转载](https://www.citicsf.com/e-futures/content/000509/819778)显示内容是《关于2026年部分节假日休市安排的通知》，并非交割公告。[中金所官方中证1000合约细则](https://www.cffex.com.cn/cn/ssxz/20220718/43093.html)规定到期月第三个星期五为最后交易日/交割日，法定假日或异常停市顺延；它能支持规则推导，却不能把休市通知冒充逐月交割公告。VM 已据此将合同改为明确标记的 v2 计划日历；本地确认型生产能力继续关闭。

VM 已确认新合同定位为 `FuturesDelivery` v2 的 `Planned` 规则预排：请求和记录均升 v2，记录要有产品规则及假期依据 URL，旧 v1 显式拒绝。该预排只能供明确标注的只读日历查询，不能证明“今日已交割”或存在逐月公告；异常停市和之后的交易所调整必须由新事实及修订处理。当前 EventCalendar 生产模板仍要求公告 `source_at`、`notice_url` 且文案为“官方通知”，因此确认型发送保持 fail-closed，不把 v2 Planned 塞入旧 v1 持久绑定。

隔离树已发布 `docs/integrations/grpc-futures-delivery-v2.md` 与四条脱敏 fixture：ExternalV1 payload schema `magic.market.futures_delivery.request` v2，JSON `{ "year": 2026, "month": 9 }`；记录 schema `magic.market.futures_delivery_event` v2，四条 IF/IH/IC/IM，`schedule_status=Planned`、`date_basis=CffexRuleAndPublishedHolidays`、各产品 `rule_url`、政府 2026 假期 `holiday_calendar_url`，无 `notice_url`，批次及记录 `source_at` 均缺失。`complete=true` 仅表示四条预排齐全。fixture 是固定观测时刻的**合成样例**；实际部署后的 2026-09 RPC 与全年 12 个月、48 条线上探针已单独通过。本地已建立 `cffex_planned_contract_month` 只读入口，旧 `cffex_contract_month` 确认型入口继续零 RPC 拒绝。

部署顺序已交接：本地 ExternalV1 Health 将上游 `source_revision`、二进制和 descriptor 哈希精确钉在当前客户端。VM 先完成 v2 提交、构建、同源公开 bundle/fixture 并交付精确身份，待本地消费者编译和切换准备完成后再替换线上服务；不能先更换线上进程使现运行 monitor 的其他 gRPC 路由失去资格。
公开 bundle 同步以其 `manifest.sha256` 列出的完整文件集为准，包括其他公开 profile/route 文档和新增 v2 合同、脱敏 fixture；不得只复制预先猜测的文件子集，也不得把证书、token、连接凭据写进仓库。

虚拟机回复需包含：

1. 完整请求形状、`preferred_provider` / profile、2026 年覆盖和 2027 年/非法月份/空请求的明确结果；规范请求及其真实响应，包含 `request_id`、批次、来源证据、`complete` 和记录 schema。
2. 服务端源码 commit、测试命令和结果、进程/产物身份（PID、可执行文件哈希、监听地址）、实际部署时间。
3. 对当前响应记录字段与本地旧 `convert::futures_delivery` 的字段差异给出一条 JSON fixture；说明空批次何时有权视为已验证为空。
4. 若部署产物身份变化，发布同源 `client-bundle/bundle-metadata.json` 与 descriptor；本地 ExternalV1 Health 门钉住这些值，不接受仅凭运行响应自称的新身份。

## 本地接收门

本地已接入 ExternalV1 v2 的 `year/month` 请求、FuturesDelivery 方法路由、四合约响应校验和脱敏探针，并移除会把旧数组空批次当作 `VerifiedEmpty` 的转换器。VM 真实 Health 的 `source_revision`、二进制及 raw descriptor 哈希均已与公开 metadata 对账；2026-09 返回 IF/IH/IC/IM 四条 v2 Planned、同一个 2026-09-18 预排日期，2027-09 返回 `UNIMPLEMENTED`，旧 v1 返回 `INVALID_ARGUMENT`。本地探针 `--opening` 返回 `opening_static_ready=true`、8/9 静态路由可用；GlobalNews-Eastmoney 为 `internal` / `Unadmitted`，已交 VM 单独排查，不按空批次接收。

本地 release `monitor` SHA-256 为 `e8e1e6665d3f4d2121e05768d64668b47a829c6b163e41cf2e0502c6754bddca`；`--test --push-dry-run` 退出 0。首次 launchd 重启时当前用户 TCC 服务卡住受保护目录文件打开，采样见 dyld `__open`，旧/新二进制均复现；终止并重启当前用户 TCC 后普通目录访问恢复。09:12 再次 bootstrap 的 launchd 进程仍卡在 dyld `__open`，已 bootout，避免重复进程。新版 monitor 现由独立 Terminal 进程 PID `23247` 运行，已打开生产数据库及投递锁。**这不是 launchd KeepAlive 验收**；需在安全时段查明该 launchd 专属启动阻塞、重新接管并核对新 PID、连续日志和业务时段结果。旧本地二进制保留在 `/private/tmp/stock-analysis-r08-cutover-20260927/monitor`，VM 回退包保留在 `target/runtime/archive/r08-predeploy-69c6ef1`。本轮未改 `src/config`，无需重发 activation。
