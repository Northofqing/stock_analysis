//! Event infrastructure — v17.1-r2 Task 1
//!
//! Provides the `DomainEvent` trait, `EventEnvelope` wrapper, and
//! `PushDeliveryEvent` for the event-seam infrastructure.

pub mod envelope;

pub use envelope::{DomainEvent, EnvelopeError, EventEnvelope, PushDeliveryEvent};
