# R-08 FuturesDelivery：虚拟机服务端与本地消费端交接（2026-09-27）

## 现状与责任边界

- 本地基线 `c67e4f6a`：`FuturesDeliveryGateway::cffex_contract_month` 在业务 RPC 前返回 `futures_delivery_contract_unavailable_v1`；`GrpcSource::futures_delivery_async` 也禁止绕行。R-08 不会把未知范围或空批次写成“无交割”。
- 虚拟机仓库 `C:\DevelopFile\magic-market-data-rs` 的检查基线为 `0554654`，工作树另有新闻/content-discovery 未提交修改，必须保留。`magic-market-core/src/calendar.rs` 的 `FuturesDeliveryRequest` 要求 `year/month`；`magic-market-composition/src/grpc_production.rs` 按此类型解码；`magic-exchange-rs/src/cffex.rs` 的正式固定日历只接受 2026 年，每月返回 IF/IH/IC/IM 四个合约。
- 本地公开 `client-bundle/grpc-external-api.md` 只列 request schema 名，未冻结字段、覆盖年限和空结果语义。旧本地 `{}` 请求形状与当前服务端必填年月不一致。历史 `internal` 失败尚不能仅据此归因。

## 已派发给虚拟机侧

虚拟机项目任务 `01a0d3f4-eb49-7bb1-a09e-462245d3e715` 已收到开发与部署交接：核对真实 wire，发布唯一版本化合同与脱敏 fixture，必要时修复服务端问题，完成定向测试、构建、部署和真实 RPC 验证。不得覆盖或顺带提交现有未提交改动，也不得扩展未经证实的年份或制造 `VerifiedEmpty`。

虚拟机回复需包含：

1. 完整请求形状、`preferred_provider` / profile、2026 年覆盖和 2027 年/非法月份/空请求的明确结果；规范请求及其真实响应，包含 `request_id`、批次、来源证据、`complete` 和记录 schema。
2. 服务端源码 commit、测试命令和结果、进程/产物身份（PID、可执行文件哈希、监听地址）、实际部署时间。
3. 对当前响应记录字段与本地旧 `convert::futures_delivery` 的字段差异给出一条 JSON fixture；说明空批次何时有权视为已验证为空。
4. 若部署产物身份变化，发布同源 `client-bundle/bundle-metadata.json` 与 descriptor；本地 ExternalV1 Health 门钉住这些值，不接受仅凭运行响应自称的新身份。

## 本地接收门

本地已准备 ExternalV1 的 `year/month` 请求、FuturesDelivery 方法路由和四合约响应校验，并移除会把旧数组空批次当作 `VerifiedEmpty` 的转换器；业务入口仍保持 `futures_delivery_contract_unavailable_v1`。取得上面合同与 fixture 后，将校准响应字段，接通 Gateway，并以真实请求核对 `request_id`、audit、schema 和源证据。未通过前 R-08 不投递未知交割结论。
