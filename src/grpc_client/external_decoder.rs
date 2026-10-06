//! Explicit release decoder catalog. Test B is an independently compiled
//! additive protobuf contract, not a digest learned from a server response.
use super::external_query_transport::{wire_error, EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256};
use super::{errors::GrpcError, external_pb::magic::market::v1 as current, historical_external};
use prost::Message;

#[derive(Clone, Copy)]
pub(crate) enum ExternalDecoder {
    Current,
    ArchivedA,
    Archived20260928,
    #[cfg(test)]
    TestB,
}

impl ExternalDecoder {
    /// Code-owned decoder catalog, not authorization. Callers must separately
    /// verify the recorded/current connection's policy and descriptor binding.
    pub(crate) fn for_descriptor(descriptor: &str) -> Result<Self, GrpcError> {
        use sha2::{Digest, Sha256};
        let compiled = hex::encode(Sha256::digest(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/external_v1/descriptor.bin"
        ))));
        if descriptor == EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256 && descriptor == compiled {
            return Ok(Self::Current);
        }
        if historical_external::accepts_descriptor(descriptor) {
            return Ok(Self::ArchivedA);
        }
        if super::archived_external_20260928::accepts_descriptor(descriptor) {
            return Ok(Self::Archived20260928);
        }
        #[cfg(test)]
        if descriptor == test_b::descriptor() {
            return Ok(Self::TestB);
        }
        Err(wire_error("unsupported_external_descriptor"))
    }

    pub(crate) fn request_context(
        self,
        health: bool,
        bytes: &[u8],
    ) -> Result<(u32, String), GrpcError> {
        if matches!(self, Self::ArchivedA) {
            return historical_external::request_context(health, bytes);
        }
        if matches!(self, Self::Archived20260928) {
            return super::archived_external_20260928::request_context(health, bytes);
        }
        #[cfg(test)]
        if matches!(self, Self::TestB) {
            let context = if health {
                canonical::<test_b::HealthRequest>(bytes)?.context
            } else {
                canonical::<test_b::CapabilitiesRequest>(bytes)?.context
            }
            .ok_or_else(|| wire_error("external_request_wire_invalid"))?;
            return Ok((context.protocol_version, context.request_id));
        }
        // B is explicitly additive in responses only. Request bytes/context
        // remain identical; no response can declare request compatibility.
        let context = if health {
            canonical::<current::HealthRequest>(bytes)?.context
        } else {
            canonical::<current::CapabilitiesRequest>(bytes)?.context
        }
        .ok_or_else(|| wire_error("external_request_wire_invalid"))?;
        Ok((context.protocol_version, context.request_id))
    }

    pub(crate) fn query_request(self, bytes: &[u8]) -> Result<(), GrpcError> {
        if matches!(self, Self::ArchivedA) {
            return historical_external::query_request(bytes);
        }
        if matches!(self, Self::Archived20260928) {
            return super::archived_external_20260928::query_request(bytes);
        }
        // A→B continuation is supported only for the identical canonical
        // GlobalNews request shape. Frozen plan validation checks its identity;
        // this verifies current decoder compatibility without rewriting bytes.
        #[cfg(test)]
        if matches!(self, Self::TestB) {
            canonical::<test_b::QueryRequest>(bytes)?;
            return Ok(());
        }
        canonical::<current::QueryRequest>(bytes)?;
        Ok(())
    }

    pub(crate) fn health(self, bytes: &[u8]) -> Result<current::HealthResponse, GrpcError> {
        match self {
            Self::ArchivedA => historical_external::health(bytes),
            Self::Archived20260928 => super::archived_external_20260928::health(bytes),
            Self::Current => canonical(bytes),
            #[cfg(test)]
            Self::TestB => {
                canonical::<test_b::HealthResponse>(bytes)?;
                decode(bytes)
            }
        }
    }
    pub(crate) fn capabilities(
        self,
        bytes: &[u8],
    ) -> Result<current::CapabilitiesResponse, GrpcError> {
        match self {
            Self::ArchivedA => historical_external::capabilities(bytes),
            Self::Archived20260928 => super::archived_external_20260928::capabilities(bytes),
            Self::Current => canonical(bytes),
            #[cfg(test)]
            Self::TestB => {
                canonical::<test_b::CapabilitiesResponse>(bytes)?;
                decode(bytes)
            }
        }
    }
    pub(crate) fn query(self, bytes: &[u8]) -> Result<current::QueryResponse, GrpcError> {
        // Decode first so the durable unary attempt can retain exact response
        // bytes. The caller rejects forbidden wire fields after capture.
        match self {
            Self::ArchivedA => historical_external::query(bytes),
            Self::Archived20260928 => super::archived_external_20260928::query(bytes),
            Self::Current => decode(bytes),
            #[cfg(test)]
            Self::TestB => {
                decode::<test_b::QueryResponse>(bytes)?;
                decode(bytes)
            }
        }
    }
    pub(crate) fn error_detail(self, bytes: &[u8]) -> Option<current::ErrorDetail> {
        match self {
            Self::ArchivedA => historical_external::error_detail(bytes),
            Self::Archived20260928 => super::archived_external_20260928::error_detail(bytes),
            Self::Current => current::ErrorDetail::decode(bytes).ok(),
            #[cfg(test)]
            Self::TestB => {
                test_b::ErrorDetail::decode(bytes).ok()?;
                current::ErrorDetail::decode(bytes).ok()
            }
        }
    }
}

fn decode<T: Message + Default>(bytes: &[u8]) -> Result<T, GrpcError> {
    T::decode(bytes).map_err(|_| wire_error("external_response_wire_invalid"))
}
fn canonical<T: Message + Default>(bytes: &[u8]) -> Result<T, GrpcError> {
    let message: T = decode(bytes)?;
    if message.encode_to_vec() != bytes {
        return Err(wire_error("external_response_wire_invalid"));
    }
    Ok(message)
}

/// Control codec preserves the exact received bytes before mapping stable
/// fields into domain types; prost re-encoding A would silently erase B fields.
pub(super) async fn control_call<Q, R>(
    channel: tonic::transport::Channel,
    request: tonic::Request<Q>,
    path: &'static str,
    decoder: ExternalDecoder,
    decode: fn(ExternalDecoder, &[u8]) -> Result<R, GrpcError>,
) -> Result<(R, Vec<u8>), tonic::Status>
where
    Q: Message + Default + Send + 'static,
    R: Send + Sync + 'static,
{
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|_| tonic::Status::unavailable("external control transport unavailable"))?;
    client
        .unary(
            request,
            tonic::codegen::http::uri::PathAndQuery::from_static(path),
            ControlCodec::<Q, R> {
                decoder,
                decode,
                marker: std::marker::PhantomData,
            },
        )
        .await
        .map(tonic::Response::into_inner)
}
struct ControlCodec<Q, R> {
    decoder: ExternalDecoder,
    decode: fn(ExternalDecoder, &[u8]) -> Result<R, GrpcError>,
    marker: std::marker::PhantomData<Q>,
}
struct ControlDecoder<R> {
    decoder: ExternalDecoder,
    decode: fn(ExternalDecoder, &[u8]) -> Result<R, GrpcError>,
}
impl<Q: Message + Default + Send + 'static, R: Send + 'static> tonic::codec::Codec
    for ControlCodec<Q, R>
{
    type Encode = Q;
    type Decode = (R, Vec<u8>);
    type Encoder = tonic_prost::ProstEncoder<Q>;
    type Decoder = ControlDecoder<R>;
    fn encoder(&mut self) -> Self::Encoder {
        tonic_prost::ProstEncoder::new(tonic::codec::BufferSettings::default())
    }
    fn decoder(&mut self) -> Self::Decoder {
        ControlDecoder {
            decoder: self.decoder,
            decode: self.decode,
        }
    }
}
impl<R> tonic::codec::Decoder for ControlDecoder<R> {
    type Item = (R, Vec<u8>);
    type Error = tonic::Status;
    fn decode(
        &mut self,
        source: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Self::Error> {
        use prost::bytes::Buf;
        let bytes = source.copy_to_bytes(source.remaining()).to_vec();
        let message = (self.decode)(self.decoder, &bytes)
            .map_err(|_| tonic::Status::data_loss("unsupported external control wire"))?;
        Ok(Some((message, bytes)))
    }
}
#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_b {
    include!(concat!(
        env!("OUT_DIR"),
        "/external_test_upgrade_b/magic.market.v1.rs"
    ));
    pub(crate) fn descriptor() -> &'static str {
        use sha2::{Digest, Sha256};
        static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        HASH.get_or_init(|| {
            hex::encode(Sha256::digest(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/external_test_upgrade_b/descriptor.bin"
            ))))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::external_query_transport::{
        compiled_descriptor_sha256, ExternalQueryMethod, ExternalWireEvidenceV1,
        ExternalWireMaterialV1, EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
    };
    use super::*;
    use sha2::{Digest, Sha256};

    fn archived() -> ExternalDecoder {
        ExternalDecoder::for_descriptor(super::super::archived_external_20260928::DESCRIPTOR_SHA256)
            .unwrap()
    }

    fn with_unknown_field(mut bytes: Vec<u8>) -> Vec<u8> {
        // Unknown varint field 127, retaining the former query/status contract.
        bytes.extend_from_slice(&[0xf8, 0x07, 0x01]);
        bytes
    }

    #[test]
    fn current_65_rpc_descriptor_matches_compiled_bytes() {
        assert_eq!(
            EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
            "14fe7134ba6b9018d773c88d71744cdd52381081e70dd04a558c3147b4a9ea06"
        );
        assert_eq!(
            compiled_descriptor_sha256(),
            EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
        );
        assert!(matches!(
            ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256),
            Ok(ExternalDecoder::Current)
        ));
        assert!(matches!(archived(), ExternalDecoder::Archived20260928));
        assert!(ExternalDecoder::for_descriptor(&"0".repeat(64)).is_err());
    }

    #[test]
    fn sep28_control_raw_decodes_known_fields_and_rejects_unknown_fields() {
        let decoder = archived();
        let context = current::RequestContext {
            protocol_version: 1,
            request_id: "TEST_CODE_OLD_CONTROL".into(),
        };
        let health_request = current::HealthRequest {
            context: Some(context.clone()),
        }
        .encode_to_vec();
        let cap_request = current::CapabilitiesRequest {
            context: Some(context.clone()),
        }
        .encode_to_vec();
        let query_request = current::QueryRequest {
            context: Some(context.clone()),
            ..Default::default()
        }
        .encode_to_vec();
        assert_eq!(
            decoder.request_context(true, &health_request).unwrap(),
            (1, context.request_id.clone())
        );
        assert_eq!(
            decoder.request_context(false, &cap_request).unwrap(),
            (1, context.request_id)
        );
        decoder.query_request(&query_request).unwrap();
        assert!(decoder
            .request_context(true, &with_unknown_field(health_request))
            .is_err());
        assert!(decoder
            .request_context(false, &with_unknown_field(cap_request))
            .is_err());
        assert!(decoder
            .query_request(&with_unknown_field(query_request))
            .is_err());
        let health = super::super::build_identity::test_archived_20260928_health();
        let raw_health = health.encode_to_vec();
        assert_eq!(decoder.health(&raw_health).unwrap(), health);
        assert_eq!(
            decoder.health(&raw_health).unwrap().encode_to_vec(),
            raw_health
        );
        assert!(decoder.health(&with_unknown_field(raw_health)).is_err());
        let caps = current::CapabilitiesResponse {
            request_id: "TEST_CODE_OLD_CONTROL".into(),
            capabilities: vec![current::Capability {
                operation: current::Operation::GlobalNews as i32,
                provider: "Jin10".into(),
                runtime_available: true,
                ..Default::default()
            }],
        };
        let raw_caps = caps.encode_to_vec();
        assert_eq!(decoder.capabilities(&raw_caps).unwrap(), caps);
        assert_eq!(
            decoder.capabilities(&raw_caps).unwrap().encode_to_vec(),
            raw_caps
        );
        assert!(decoder.capabilities(&with_unknown_field(raw_caps)).is_err());
    }

    #[test]
    fn sep28_query_and_status_project_known_fields_while_retaining_raw_evidence() {
        let decoder = archived();
        let query = current::QueryResponse {
            request_id: "TEST_CODE_OLD_QUERY".into(),
            operation: current::Operation::GlobalNews as i32,
            selected_provider: "Jin10".into(),
            complete: true,
            records: vec![current::CanonicalPayload {
                schema: "magic.market.global_news".into(),
                schema_version: 2,
                content_type: "application/json".into(),
                data: b"[]".to_vec(),
            }],
            ..Default::default()
        };
        let raw_query = with_unknown_field(query.encode_to_vec());
        assert_eq!(decoder.query(&raw_query).unwrap(), query);
        assert_ne!(query.encode_to_vec(), raw_query);
        let evidence = ExternalWireEvidenceV1 {
            material: "external-unary-response-evidence-v1".into(),
            profile: "ExternalV1".into(),
            method: ExternalQueryMethod::GlobalNews,
            client_descriptor_sha256: super::super::archived_external_20260928::DESCRIPTOR_SHA256
                .into(),
            evidence: ExternalWireMaterialV1::Payload {
                payload_sha256: hex::encode(Sha256::digest(&raw_query)),
                protobuf_payload: raw_query.clone(),
                decode_limit_bytes: EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
            },
        };
        let restored: ExternalWireEvidenceV1 =
            serde_json::from_slice(&serde_json::to_vec(&evidence).unwrap()).unwrap();
        restored
            .validate_descriptor(
                ExternalQueryMethod::GlobalNews,
                super::super::archived_external_20260928::DESCRIPTOR_SHA256,
            )
            .unwrap();
        assert_eq!(restored.payload().unwrap(), raw_query.as_slice());
        assert!(restored.validate(ExternalQueryMethod::GlobalNews).is_err());
        let detail = current::ErrorDetail {
            request_id: "TEST_CODE_OLD_QUERY".into(),
            operation: current::Operation::GlobalNews as i32,
            provider: "Jin10".into(),
            reason_code: "TEST_CODE_OLD_STATUS".into(),
            retryable: true,
            ..Default::default()
        };
        let raw_detail = with_unknown_field(detail.encode_to_vec());
        let status = tonic::Status::with_details(
            tonic::Code::Unavailable,
            "old status",
            raw_detail.clone().into(),
        );
        assert_eq!(decoder.error_detail(status.details()).unwrap(), detail);
        assert_eq!(status.details(), raw_detail.as_slice());
        assert_ne!(detail.encode_to_vec(), raw_detail);
    }
}
