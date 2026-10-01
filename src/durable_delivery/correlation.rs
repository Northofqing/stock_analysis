use super::model::{
    stable_identity, validate_business_date, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryError, PushKind, Result,
};

const OBSERVATION_DOMAIN: &str = "durable-delivery-correlation-observation-v1";
pub(crate) const OBSERVATION_IDENTITY_VERSION: i64 = 1;
pub(crate) const OBSERVATION_ROLE_ORIGIN: &str = "Origin";

/// The only producer labels accepted by the first correlation slice.
/// The monitor must separately verify catalog membership at its own call site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P01OriginProducer {
    Scheduled,
    Compensation,
}

impl P01OriginProducer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "p01-scheduled",
            Self::Compensation => "p01-compensation",
        }
    }
}

impl TryFrom<&str> for P01OriginProducer {
    type Error = DurableDeliveryError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "p01-scheduled" => Ok(Self::Scheduled),
            "p01-compensation" => Ok(Self::Compensation),
            _ => Err(DurableDeliveryError::InvalidEnvelope(
                "unregistered P01 origin producer".to_owned(),
            )),
        }
    }
}

/// One immutable producer-to-decision edge, not one row per invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CorrelationObservationV1 {
    producer: P01OriginProducer,
    occurrence_identity: String,
}

impl CorrelationObservationV1 {
    pub(crate) fn p01_origin(
        producer: P01OriginProducer,
        occurrence_identity: impl Into<String>,
    ) -> Result<Self> {
        let occurrence_identity = occurrence_identity.into();
        let date = occurrence_identity.strip_prefix("p01:").ok_or_else(|| {
            DurableDeliveryError::InvalidEnvelope(
                "P01 origin occurrence must be p01:YYYY-MM-DD".to_owned(),
            )
        })?;
        let parsed = validate_business_date(date)?;
        if date.len() != 10 || parsed.format("%Y-%m-%d").to_string() != date {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "P01 origin occurrence must be p01:YYYY-MM-DD".to_owned(),
            ));
        }
        Ok(Self {
            producer,
            occurrence_identity,
        })
    }

    pub(crate) fn validate_for_envelope(&self, envelope: &DeliveryEnvelope) -> Result<()> {
        if envelope.push_kind != PushKind::PreopenNewsHot
            || envelope.sub_kind != DeliverySubKind::None
            || self.occurrence_identity != envelope.schedule_occurrence_identity
            || self.occurrence_identity != format!("p01:{}", envelope.business_date)
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "P01 origin observation does not match the delivery envelope".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn producer_id(&self) -> &'static str {
        self.producer.as_str()
    }

    pub(crate) fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }

    pub(crate) fn identity_for_decision(&self, decision_identity: &str) -> String {
        // stable_identity encodes the domain and each tuple field with a u64
        // big-endian byte length before hashing, avoiding concatenation aliases.
        stable_identity(
            OBSERVATION_DOMAIN,
            &[
                decision_identity,
                self.producer_id(),
                self.occurrence_identity(),
                OBSERVATION_ROLE_ORIGIN,
                "1",
            ],
        )
    }
}
