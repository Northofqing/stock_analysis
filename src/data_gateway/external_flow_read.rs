//! Isolated ExternalV1 flow acquisition and admission. Production routes do
//! not use this client until the revision-bound upstream acceptance gate passes.

use super::external_flow::{
    admit_board_flows, admit_money_flow, ExternalBoardFlowRow, ExternalMoneyFlowPoint,
};
use super::{BoardKind, GatewayBatch, GatewayError};
use crate::grpc_client::client::external_flow_read::{
    ExternalFlowObservation, ExternalFlowReadClient,
};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::external_pb::magic::market::v1::Operation as ExternalOperation;
use crate::market_domain::{FlowInterval, InstrumentId};
use chrono::{DateTime, Utc};
use std::path::Path;

pub(crate) struct ExternalFlowGateway {
    reader: ExternalFlowReadClient,
}

/// The original observation is retained on every post-call outcome. A
/// qualification or request failure before data RPC is returned by the
/// gateway method as GrpcError because the reader has no observation yet.
pub(crate) struct ExternalFlowRead<T> {
    pub(crate) observation: ExternalFlowObservation,
    pub(crate) admission: ExternalFlowAdmission<T>,
}

pub(crate) enum ExternalFlowAdmission<T> {
    Admitted(GatewayBatch<T>),
    /// The classified GrpcError remains in `observation.result`.
    QueryRejected,
    EvidenceRejected(GatewayError),
}

impl ExternalFlowGateway {
    pub(crate) async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        Ok(Self {
            reader: ExternalFlowReadClient::connect_client_bundle(path).await?,
        })
    }

    pub(crate) async fn money_flow_once(
        &mut self,
        requested: &InstrumentId,
        now: DateTime<Utc>,
    ) -> Result<ExternalFlowRead<ExternalMoneyFlowPoint>, GrpcError> {
        let observation = self
            .reader
            .query_once(
                ExternalOperation::MoneyFlows,
                serde_json::json!({"instruments": [requested]}),
            )
            .await?;
        Ok(project_money_flow(observation, requested, now))
    }

    pub(crate) async fn board_flows_once(
        &mut self,
        category: BoardKind,
        interval: FlowInterval,
        limit: u32,
        now: DateTime<Utc>,
    ) -> Result<ExternalFlowRead<ExternalBoardFlowRow>, GrpcError> {
        let category_name = match category {
            BoardKind::Industry => "Industry",
            BoardKind::Concept => "Concept",
            BoardKind::Region => "Region",
        };
        let observation = self
            .reader
            .query_once(
                ExternalOperation::BoardFlows,
                serde_json::json!({"category": category_name, "interval": interval, "limit": limit}),
            )
            .await?;
        Ok(project_board_flows(
            observation,
            category,
            interval,
            limit,
            now,
        ))
    }
}

fn project_money_flow(
    observation: ExternalFlowObservation,
    requested: &InstrumentId,
    now: DateTime<Utc>,
) -> ExternalFlowRead<ExternalMoneyFlowPoint> {
    let admission = match &observation.result {
        Ok(response) => match admit_money_flow(requested, response, now) {
            Ok(batch) => ExternalFlowAdmission::Admitted(batch),
            Err(error) => ExternalFlowAdmission::EvidenceRejected(error),
        },
        Err(_) => ExternalFlowAdmission::QueryRejected,
    };
    ExternalFlowRead {
        observation,
        admission,
    }
}

fn project_board_flows(
    observation: ExternalFlowObservation,
    category: BoardKind,
    interval: FlowInterval,
    limit: u32,
    now: DateTime<Utc>,
) -> ExternalFlowRead<ExternalBoardFlowRow> {
    let admission = match &observation.result {
        Ok(response) => match admit_board_flows(category, interval, limit, response, now) {
            Ok(batch) => ExternalFlowAdmission::Admitted(batch),
            Err(error) => ExternalFlowAdmission::EvidenceRejected(error),
        },
        Err(_) => ExternalFlowAdmission::QueryRejected,
    };
    ExternalFlowRead {
        observation,
        admission,
    }
}

#[cfg(test)]
#[path = "external_flow_read_tests.rs"]
mod tests;
