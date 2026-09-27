# R-08 FuturesDelivery：虚拟机服务端与本地消费端交接（2026-09-27）

## 现状与责任边界

- 本地基线 `c67e4f6a`：`FuturesDeliveryGateway::cffex_contract_month` 在业务 RPC 前返回 `futures_delivery_contract_unavailable_v1`；`GrpcSource::futures_delivery_async` 也禁止绕行。R-08 不会把未知范围或空批次写成“无交割”。
- 虚拟机仓库 `C:\DevelopFile\magic-market-data-rs` 的检查基线为 `0554654`，工作树另有新闻/content-discovery 未提交修改，必须保留。`magic-market-core/src/calendar.rs` 的 `FuturesDeliveryRequest` 要求 `year/month`；`magic-market-composition/src/grpc_production.rs` 按此类型解码；`magic-exchange-rs/src/cffex.rs` 的正式固定日历只接受 2026 年，每月返回 IF/IH/IC/IM 四个合约。
- 本地公开 `client-bundle/grpc-external-api.md` 只列 request schema 名，未冻结字段、覆盖年限和空结果语义。旧本地 `{}` 请求形状与当前服务端必填年月不一致。历史 `internal` 失败尚不能仅据此归因。

## 已派发给虚拟机侧

当前上游执行任务是远程 Codex `01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`，在被忽略的 `.worktrees/r08-futures-delivery` 隔离树工作；原任务 `01a0d3f4-eb49-7bb1-a09e-462245d3e715` 因托管工作树审批挂起，已收到停止指令。当前任务负责核对真实 wire，发布唯一版本化合同与脱敏 fixture，必要时修复服务端问题，完成定向测试、构建、部署和真实 RPC 验证。不得覆盖或顺带提交主工作树新闻改动，也不得扩展未经证实的年份或制造 `VerifiedEmpty`。

### 2026-09-27 阶段性 RPC 与来源异常

部署前的真实 `FuturesDelivery` RPC 使用 ExternalV1 `{"year":2026,"month":9}`、`preferred_provider=Cffex`，返回 `ADMITTED`、`complete=true`、IF/IH/IC/IM 四条、`batch_id=cffex-equity-index-delivery-2026-v1:09`；单条 schema 为 `magic.market.futures_delivery_event` v1，`observed_at` 是 Unix 秒小数字符串，`source_at` 缺失。2027-09 真实请求返回 `UNIMPLEMENTED`，明确仅覆盖 2026 年。这些是**旧运行进程的部署前证据**，不能代替最终提交的 Health 身份和部署后 RPC。

该响应所有记录的 `notice_url` 都是 `https://www.cffex.com.cn/jystz/20251217/46425.html`。2026-09-27 本地 HTTP HEAD 对同一路径得到 302 → 404；[标注该原始链接的交易所通知转载](https://www.citicsf.com/e-futures/content/000509/819778)显示内容是《关于2026年部分节假日休市安排的通知》，并非交割公告。[中金所官方中证1000合约细则](https://www.cffex.com.cn/cn/ssxz/20220718/43093.html)规定到期月第三个星期五为最后交易日/交割日，法定假日或异常停市顺延；它能支持规则推导，却不能把休市通知冒充逐月交割公告。VM 已收到来源错误交接：核实官方原件、说明“细则+休市日历计算”还是“月度公告”合同、修正 URL/版本/异常停市语义并重做 fixture；证据修复前不部署 R-08。本地接线代码仍待最终合同校准，不发布生产能力。

VM 已确认新合同定位为 `FuturesDelivery` v2 的 `Planned` 规则预排：请求和记录均升 v2，记录要有产品规则及假期依据 URL，旧 v1 显式拒绝。该预排只能供明确标注的只读日历查询，不能证明“今日已交割”或存在逐月公告；异常停市和之后的交易所调整必须由新事实及修订处理。当前 EventCalendar 生产模板仍要求公告 `source_at`、`notice_url` 且文案为“官方通知”，因此确认型发送保持 fail-closed，不把 v2 Planned 塞入旧 v1 持久绑定。

部署顺序已交接：本地 ExternalV1 Health 将上游 `source_revision`、二进制和 descriptor 哈希精确钉在当前客户端。VM 先完成 v2 提交、构建、同源公开 bundle/fixture 并交付精确身份，待本地消费者编译和切换准备完成后再替换线上服务；不能先更换线上进程使现运行 monitor 的其他 gRPC 路由失去资格。

虚拟机回复需包含：

1. 完整请求形状、`preferred_provider` / profile、2026 年覆盖和 2027 年/非法月份/空请求的明确结果；规范请求及其真实响应，包含 `request_id`、批次、来源证据、`complete` 和记录 schema。
2. 服务端源码 commit、测试命令和结果、进程/产物身份（PID、可执行文件哈希、监听地址）、实际部署时间。
3. 对当前响应记录字段与本地旧 `convert::futures_delivery` 的字段差异给出一条 JSON fixture；说明空批次何时有权视为已验证为空。
4. 若部署产物身份变化，发布同源 `client-bundle/bundle-metadata.json` 与 descriptor；本地 ExternalV1 Health 门钉住这些值，不接受仅凭运行响应自称的新身份。

## 本地接收门

本地已准备 ExternalV1 的 `year/month` 请求、FuturesDelivery 方法路由和四合约响应校验，并移除会把旧数组空批次当作 `VerifiedEmpty` 的转换器；工作树正在实现 Gateway 接线与脱敏探针，但尚未提交或部署。取得已修正的同源合同、fixture 和部署身份后，将校准响应字段、notice 证据，核对真实 `request_id`、audit、schema 和来源，再决定生产放行。未通过前 R-08 不投递未知交割结论。
