#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverySettlement {
    Pushed,
    SinkError { reason_code: String },
    PhysicallyDeliveredAuditFailed { failures: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityAction {
    Commit,
    Release,
}

pub fn settle(
    sink_accepted: bool,
    audit_succeeded: bool,
    failures: Vec<String>,
) -> (DeliverySettlement, IdentityAction) {
    match (sink_accepted, audit_succeeded) {
        (true, true) => (DeliverySettlement::Pushed, IdentityAction::Commit),
        (true, false) => (
            DeliverySettlement::PhysicallyDeliveredAuditFailed { failures },
            IdentityAction::Commit,
        ),
        (false, _) => (
            DeliverySettlement::SinkError {
                reason_code: "sink_rejected".into(),
            },
            IdentityAction::Release,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_sink_audit_failure_commits_identity() {
        let (outcome, action) = settle(true, false, vec!["audit_poisoned".into()]);
        assert_eq!(action, IdentityAction::Commit);
        assert!(matches!(
            outcome,
            DeliverySettlement::PhysicallyDeliveredAuditFailed { .. }
        ));
    }
}
