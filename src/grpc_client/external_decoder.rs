//! Explicit release decoder catalog. Test B is an independently compiled
//! additive protobuf contract, not a digest learned from a server response.
use super::external_query_transport::{wire_error, EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256};
use super::{errors::GrpcError, external_pb::magic::market::v1 as current, historical_external};
use prost::Message;

#[derive(Clone, Copy)]
pub(crate) enum ExternalDecoder {
    Current,
    ArchivedA,
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
            Self::Current => canonical(bytes),
            #[cfg(test)]
            Self::TestB => {
                canonical::<test_b::CapabilitiesResponse>(bytes)?;
                decode(bytes)
            }
        }
    }
    pub(crate) fn query(self, bytes: &[u8]) -> Result<current::QueryResponse, GrpcError> {
        super::external_query_transport::admit_external_payload(bytes)?;
        match self {
            Self::ArchivedA => historical_external::query(bytes),
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
