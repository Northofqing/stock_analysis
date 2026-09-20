//! Unmodified public ExternalV1 contract, generated separately from LocalBridgeV1.

/// Descriptor emitted by the same build invocation as the generated types below.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/external_v1/descriptor.bin"));

pub mod magic {
    pub mod market {
        pub mod v1 {
            include!(concat!(
                env!("OUT_DIR"),
                "/external_v1/magic.market.v1.rs"
            ));
        }
    }
}
