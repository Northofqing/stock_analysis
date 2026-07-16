//! Event infrastructure — v17.1-r2 Task 1+2
//!
//! Provides the `DomainEvent` trait, `EventEnvelope` wrapper, and
//! `PushDeliveryEvent` for the event-seam infrastructure, plus a bounded
//! `EventBus` for broadcast distribution.

pub mod bus;
pub mod envelope;

pub use bus::{EventBus, EventBusMetrics, PublishOutcome, RejectReason};
pub use envelope::{DomainEvent, EnvelopeError, EventEnvelope, PushDeliveryEvent};
