//! BR-174 receipt-verified schema-v2 selection.

pub mod acquisition_v2;
pub mod activation_runtime;
pub mod admission;
pub mod audit;
pub mod config_activation_v2;
pub mod features;
pub mod ingress_v2;
pub mod model;
pub(crate) mod outcome_session_gate;
pub mod outcome_v2;
pub mod persistence_v2;
mod process_bootstrap;
pub mod quality;
pub mod relation;
pub mod schema_v2;
pub mod trading_calendar_v2;

pub use process_bootstrap::{
    bootstrap_selection_process, SelectionProcessBootstrapError, VerifiedParsedSelectionCli,
};
