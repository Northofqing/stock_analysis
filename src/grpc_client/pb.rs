//! tonic-prost-build 生成的 magic.market.v1 代码 (OUT_DIR, 不提交)。
//! tonic 0.14 实测: tonic-prost-build 生成的 magic.market.v1.rs 是**扁平**结构
//! (message struct + service 模块全在顶层, 无 `mod magic` 嵌套 — 与计划注释的
//! 0.14 前 prost-build generate-modules 默认行为不同), 所以这里手写包嵌套包装。
//! 注意: 本文件是 grpc_client/mod.rs `pub mod pb;` 挂载的**文件本身**就是 pb 模块
//! (mod.rs 声明即模块入口, 这里不能再写 `pub mod pb { }` 否则双重嵌套 pb::pb)。
//! 用法: `use crate::grpc_client::pb::magic::market::v1::QueryRequest;`
pub mod magic {
    pub mod market {
        pub mod v1 {
            tonic::include_proto!("magic.market.v1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::magic::market::v1::{AdmissionState, CanonicalPayload, QueryResponse};
    use prost::Message; // encode_to_vec/decode 是 prost::Message trait 方法

    #[test]
    fn generated_types_roundtrip() {
        let payload = CanonicalPayload {
            schema: "market.realtime_quotes".to_string(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".to_string(),
            data: b"[]".to_vec(),
        };
        let resp = QueryResponse {
            request_id: "r-1".to_string(),
            operation: 3, // OPERATION_REALTIME_QUOTES
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "2026-08-13T10:00:00+08:00".to_string(),
            source_at: "2026-08-13T10:00:00+08:00".to_string(),
            records: vec![payload],
            source: "tdx".to_string(),
        };
        let bytes = prost::Message::encode_to_vec(&resp);
        let decoded = QueryResponse::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.request_id, "r-1");
        assert_eq!(decoded.records.len(), 1);
        assert_eq!(decoded.records[0].schema, "market.realtime_quotes");
    }
}
