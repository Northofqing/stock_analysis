//! Bounded, observed-only veto diagnostics. Serialization grants no risk authority.
use super::veto_chain::{VetoChainConfig, VetoContext, VetoMode, VetoOutcome, VetoVerdict};
use super::veto_rules_live::{BiasRateRule, FundamentalDeteriorationRule, MainFlowRule};
use serde::Serialize;

pub const VETO_REPORT_SCHEMA_VERSION_V1: u16 = 1;
pub const VETO_CATALOG_VERSION_V1: u16 = 1;
pub const VETO_CONFIG_VERSION_V1: u16 = 1;
pub const VETO_MODEL_VERSION_V1: &str = "veto-builtin-model-v1";
pub const MAX_REPORT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BuiltinRuleIdV1 {
    #[serde(rename = "BiasRateRule")]
    BiasRate,
    #[serde(rename = "MainFlowRule")]
    MainFlow,
    #[serde(rename = "FundamentalDeteriorationRule")]
    FundamentalDeterioration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VetoInputFieldV1 {
    IsBuySignal,
    SignalScore,
    BiasMa5,
    IsBearish,
    MoneyFlowLastRecord,
    MoneyFlowLastMainNet,
    MoneyFlowLastPctChg,
    PeRatio,
    NetProfitYoy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredWhenV1 {
    RuleEnabled,
    BiasRateEnabled,
    BearishAlignmentEnabled,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RequiredInputDescriptorV1 {
    pub field: VetoInputFieldV1,
    pub required_when: RequiredWhenV1,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VetoSubconditionIdV1 {
    HighBias,
    BearishAlignment,
    HeavyOutflow,
    PriceRiseWithOutflow,
    FundamentalDeterioration,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SubconditionDescriptorV1 {
    pub id: VetoSubconditionIdV1,
    pub required_inputs: &'static [VetoInputFieldV1],
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BuiltinRuleDescriptorV1 {
    pub id: BuiltinRuleIdV1,
    pub version: u16,
    pub priority: u8,
    pub required_inputs: &'static [RequiredInputDescriptorV1],
    pub subconditions: &'static [SubconditionDescriptorV1],
}
const fn required(
    field: VetoInputFieldV1,
    required_when: RequiredWhenV1,
) -> RequiredInputDescriptorV1 {
    RequiredInputDescriptorV1 {
        field,
        required_when,
    }
}
use RequiredWhenV1 as W;
use VetoInputFieldV1 as F;
use VetoSubconditionIdV1 as S;
static CATALOG: [BuiltinRuleDescriptorV1; 3] = [
    BuiltinRuleDescriptorV1 {
        id: BuiltinRuleIdV1::BiasRate,
        version: 1,
        priority: 10,
        required_inputs: &[
            required(F::IsBuySignal, W::RuleEnabled),
            required(F::SignalScore, W::RuleEnabled),
            required(F::BiasMa5, W::BiasRateEnabled),
            required(F::IsBearish, W::BearishAlignmentEnabled),
        ],
        subconditions: &[
            SubconditionDescriptorV1 {
                id: S::HighBias,
                required_inputs: &[F::BiasMa5],
            },
            SubconditionDescriptorV1 {
                id: S::BearishAlignment,
                required_inputs: &[F::IsBearish],
            },
        ],
    },
    BuiltinRuleDescriptorV1 {
        id: BuiltinRuleIdV1::MainFlow,
        version: 1,
        priority: 20,
        required_inputs: &[
            required(F::IsBuySignal, W::RuleEnabled),
            required(F::SignalScore, W::RuleEnabled),
            required(F::MoneyFlowLastRecord, W::RuleEnabled),
            required(F::MoneyFlowLastMainNet, W::RuleEnabled),
            required(F::MoneyFlowLastPctChg, W::RuleEnabled),
        ],
        subconditions: &[
            SubconditionDescriptorV1 {
                id: S::HeavyOutflow,
                required_inputs: &[F::MoneyFlowLastRecord, F::MoneyFlowLastMainNet],
            },
            SubconditionDescriptorV1 {
                id: S::PriceRiseWithOutflow,
                required_inputs: &[
                    F::MoneyFlowLastRecord,
                    F::MoneyFlowLastMainNet,
                    F::MoneyFlowLastPctChg,
                ],
            },
        ],
    },
    BuiltinRuleDescriptorV1 {
        id: BuiltinRuleIdV1::FundamentalDeterioration,
        version: 1,
        priority: 30,
        required_inputs: &[
            required(F::IsBuySignal, W::RuleEnabled),
            required(F::SignalScore, W::RuleEnabled),
            required(F::PeRatio, W::RuleEnabled),
            required(F::NetProfitYoy, W::RuleEnabled),
        ],
        subconditions: &[SubconditionDescriptorV1 {
            id: S::FundamentalDeterioration,
            required_inputs: &[F::PeRatio, F::NetProfitYoy],
        }],
    },
];
pub fn builtin_rule_catalog_v1() -> &'static [BuiltinRuleDescriptorV1; 3] {
    &CATALOG
}
#[derive(Debug, Clone, Serialize)]
pub struct ConfiguredRequiredInputV1 {
    pub field: VetoInputFieldV1,
    pub required: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ConfiguredSubconditionV1 {
    pub id: VetoSubconditionIdV1,
    pub enabled: bool,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub enum BuiltinThresholdsV1 {
    BiasRate {
        bias_threshold: f64,
    },
    MainFlow {
        outflow_threshold: f64,
        lure_pct_threshold: f64,
        lure_outflow_threshold: f64,
    },
    FundamentalDeterioration {
        pe_upper: f64,
        profit_decline_threshold: f64,
    },
}
impl BuiltinThresholdsV1 {
    pub(super) fn bits(&self) -> Vec<u64> {
        match *self {
            Self::BiasRate { bias_threshold } => vec![bias_threshold.to_bits()],
            Self::MainFlow {
                outflow_threshold,
                lure_pct_threshold,
                lure_outflow_threshold,
            } => vec![
                outflow_threshold.to_bits(),
                lure_pct_threshold.to_bits(),
                lure_outflow_threshold.to_bits(),
            ],
            Self::FundamentalDeterioration {
                pe_upper,
                profit_decline_threshold,
            } => vec![pe_upper.to_bits(), profit_decline_threshold.to_bits()],
        }
    }
    pub(super) fn finite(&self) -> bool {
        match self {
            Self::BiasRate { bias_threshold } => bias_threshold.is_finite(),
            Self::MainFlow {
                outflow_threshold,
                lure_pct_threshold,
                lure_outflow_threshold,
            } => [
                outflow_threshold,
                lure_pct_threshold,
                lure_outflow_threshold,
            ]
            .iter()
            .all(|n| n.is_finite()),
            Self::FundamentalDeterioration {
                pe_upper,
                profit_decline_threshold,
            } => pe_upper.is_finite() && profit_decline_threshold.is_finite(),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ConfiguredBuiltinRuleV1 {
    pub id: BuiltinRuleIdV1,
    pub version: u16,
    pub priority: u8,
    pub enabled: bool,
    pub required_inputs: Vec<ConfiguredRequiredInputV1>,
    pub subconditions: Vec<ConfiguredSubconditionV1>,
    pub thresholds: BuiltinThresholdsV1,
}
pub(super) fn configured_rule(
    id: BuiltinRuleIdV1,
    enabled: bool,
    bias: bool,
    bearish: bool,
    thresholds: BuiltinThresholdsV1,
) -> ConfiguredBuiltinRuleV1 {
    let descriptor = CATALOG
        .iter()
        .find(|d| d.id == id)
        .expect("fixed builtin catalog");
    ConfiguredBuiltinRuleV1 {
        id,
        version: descriptor.version,
        priority: descriptor.priority,
        enabled,
        required_inputs: descriptor
            .required_inputs
            .iter()
            .map(|d| ConfiguredRequiredInputV1 {
                field: d.field,
                required: enabled
                    && match d.required_when {
                        W::RuleEnabled => true,
                        W::BiasRateEnabled => bias,
                        W::BearishAlignmentEnabled => bearish,
                    },
            })
            .collect(),
        subconditions: descriptor
            .subconditions
            .iter()
            .map(|d| ConfiguredSubconditionV1 {
                id: d.id,
                enabled: enabled
                    && match d.id {
                        S::HighBias => bias,
                        S::BearishAlignment => bearish,
                        _ => true,
                    },
            })
            .collect(),
        thresholds,
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct VetoConfigurationSnapshotV1 {
    schema_version: u16,
    catalog_version: u16,
    config_version: u16,
    model_version: &'static str,
    enabled: bool,
    raw_mode: String,
    effective_mode: VetoMode,
    bias_rate_enabled: bool,
    bearish_alignment_enabled: bool,
    main_flow_enabled: bool,
    fundamental_enabled: bool,
    rules: [ConfiguredBuiltinRuleV1; 3],
}
impl VetoConfigurationSnapshotV1 {
    pub fn from_config(c: &VetoChainConfig) -> Self {
        let b = BiasRateRule::default();
        let m = MainFlowRule::default();
        let f = FundamentalDeteriorationRule::default();
        Self {
            schema_version: 1,
            catalog_version: VETO_CATALOG_VERSION_V1,
            config_version: VETO_CONFIG_VERSION_V1,
            model_version: VETO_MODEL_VERSION_V1,
            enabled: c.enabled,
            raw_mode: if c.mode == VetoMode::Live {
                "live"
            } else {
                "dry_run"
            }
            .into(),
            effective_mode: c.mode,
            bias_rate_enabled: c.bias_rate_enabled,
            bearish_alignment_enabled: c.bearish_alignment_enabled,
            main_flow_enabled: c.main_flow_enabled,
            fundamental_enabled: c.fundamental_enabled,
            rules: [
                configured_rule(
                    BuiltinRuleIdV1::BiasRate,
                    c.bias_rate_enabled || c.bearish_alignment_enabled,
                    c.bias_rate_enabled,
                    c.bearish_alignment_enabled,
                    BuiltinThresholdsV1::BiasRate {
                        bias_threshold: b.bias_threshold,
                    },
                ),
                configured_rule(
                    BuiltinRuleIdV1::MainFlow,
                    c.main_flow_enabled,
                    true,
                    true,
                    BuiltinThresholdsV1::MainFlow {
                        outflow_threshold: m.outflow_threshold,
                        lure_pct_threshold: m.lure_pct_threshold,
                        lure_outflow_threshold: m.lure_outflow_threshold,
                    },
                ),
                configured_rule(
                    BuiltinRuleIdV1::FundamentalDeterioration,
                    c.fundamental_enabled,
                    true,
                    true,
                    BuiltinThresholdsV1::FundamentalDeterioration {
                        pe_upper: f.pe_upper,
                        profit_decline_threshold: f.profit_decline_threshold,
                    },
                ),
            ],
        }
    }
    pub fn from_live_config(
        c: &crate::config::LiveVetoConfig,
    ) -> Result<Self, VetoReportFailureV1> {
        if c.mode.len() > 64 {
            return Err(VetoReportFailureV1::ResourceLimit {
                resource: "raw_mode",
            });
        }
        let mut s = Self::from_config(&VetoChainConfig {
            enabled: c.enabled,
            mode: if c.mode == "live" {
                VetoMode::Live
            } else {
                VetoMode::DryRun
            },
            bias_rate_enabled: c.bias_rate_enabled,
            bearish_alignment_enabled: c.bearish_alignment_enabled,
            main_flow_enabled: c.main_flow_enabled,
            fundamental_enabled: c.fundamental_enabled,
        });
        s.raw_mode = c.mode.clone();
        Ok(s)
    }
    pub fn rules(&self) -> &[ConfiguredBuiltinRuleV1; 3] {
        &self.rules
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn raw_mode(&self) -> &str {
        &self.raw_mode
    }
    pub fn effective_mode(&self) -> VetoMode {
        self.effective_mode
    }
    pub fn bias_rate_enabled(&self) -> bool {
        self.bias_rate_enabled
    }
    pub fn bearish_alignment_enabled(&self) -> bool {
        self.bearish_alignment_enabled
    }
    pub fn main_flow_enabled(&self) -> bool {
        self.main_flow_enabled
    }
    pub fn fundamental_enabled(&self) -> bool {
        self.fundamental_enabled
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleExecutionStatusV1 {
    GloballyDisabled,
    RuleDisabled,
    NotApplicable,
    InputMissing,
    InputInvalid,
    LegacyInputContractUnknown,
    EvaluatedClear,
    EvaluatedVeto,
    Panicked,
    ReportFailed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputStatusV1 {
    Present,
    Missing,
    EmptyCollection,
    ZeroSentinel,
    NonFinite,
    OutOfRange,
    NotRequired,
}
#[derive(Debug, Clone, Serialize)]
pub struct RequiredInputObservationV1 {
    pub field: VetoInputFieldV1,
    pub required: bool,
    pub status: InputStatusV1,
}
#[derive(Debug, Clone, Serialize)]
pub struct SubconditionObservationV1 {
    pub id: VetoSubconditionIdV1,
    pub enabled: bool,
    pub status: RuleExecutionStatusV1,
}
#[derive(Debug, Clone, Serialize)]
pub struct RuleInputObservationV1 {
    pub builtin: Option<ConfiguredBuiltinRuleV1>,
    /// Exact values, including nonfinite custom builtin thresholds that JSON f64 cannot represent.
    pub threshold_bits: Vec<u64>,
    pub status: RuleExecutionStatusV1,
    pub inputs: Vec<RequiredInputObservationV1>,
    pub subconditions: Vec<SubconditionObservationV1>,
}
impl RuleInputObservationV1 {
    pub fn legacy_unknown() -> Self {
        Self {
            builtin: None,
            threshold_bits: vec![],
            status: RuleExecutionStatusV1::LegacyInputContractUnknown,
            inputs: vec![],
            subconditions: vec![],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VetoReportFailureV1 {
    PenaltyOverflow,
    ResourceLimit { resource: &'static str },
    EncodingLimit,
    EncodingFailed,
}
#[derive(Debug, Clone, Serialize)]
pub struct PanicObservationV1 {
    pub payload: String,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct RuleExecutionObservationV1 {
    pub rule_name: String,
    pub version: Option<u16>,
    pub priority: u8,
    pub status: RuleExecutionStatusV1,
    pub contract: RuleInputObservationV1,
    pub original_verdict: Option<VetoVerdict>,
    pub panic: Option<PanicObservationV1>,
    pub failure: Option<VetoReportFailureV1>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ObservedFlowRecordV1 {
    pub date: String,
    pub main_net_bits: u64,
    pub xl_net_bits: u64,
    pub big_net_bits: u64,
    pub main_pct_bits: u64,
    pub pct_chg_bits: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ObservedVetoContextV1 {
    pub code: String,
    pub current_price_bits: u64,
    pub signal_score: i32,
    pub is_buy_signal: bool,
    pub bias_ma5_bits: u64,
    pub is_bearish: bool,
    pub money_flow_count: Option<usize>,
    pub last_money_flow: Option<ObservedFlowRecordV1>,
    pub pct_chg_bits: Option<u64>,
    pub pe_ratio_bits: Option<u64>,
    pub net_profit_yoy_bits: Option<u64>,
}
impl ObservedVetoContextV1 {
    pub(super) fn capture(c: &VetoContext) -> Result<Self, VetoReportFailureV1> {
        if c.code.len() > 256 {
            return Err(VetoReportFailureV1::ResourceLimit { resource: "code" });
        }
        let last = c.money_flow_days.as_ref().and_then(|d| d.last());
        if last.is_some_and(|d| d.date.len() > 128) {
            return Err(VetoReportFailureV1::ResourceLimit {
                resource: "flow_date",
            });
        }
        Ok(Self {
            code: c.code.clone(),
            current_price_bits: c.current_price.to_bits(),
            signal_score: c.signal_score,
            is_buy_signal: c.is_buy_signal,
            bias_ma5_bits: c.bias_ma5.to_bits(),
            is_bearish: c.is_bearish,
            money_flow_count: c.money_flow_days.as_ref().map(Vec::len),
            last_money_flow: last.map(|d| ObservedFlowRecordV1 {
                date: d.date.clone(),
                main_net_bits: d.main_net.to_bits(),
                xl_net_bits: d.xl_net.to_bits(),
                big_net_bits: d.big_net.to_bits(),
                main_pct_bits: d.main_pct.to_bits(),
                pct_chg_bits: d.pct_chg.map(f64::to_bits),
            }),
            pct_chg_bits: c.pct_chg.map(f64::to_bits),
            pe_ratio_bits: c.pe_ratio.map(f64::to_bits),
            net_profit_yoy_bits: c.net_profit_yoy.map(f64::to_bits),
        })
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VetoObservationSemanticsV1 {
    ObservedOnly,
}
#[derive(Debug, Clone, Serialize)]
pub struct VetoExecutionReportV1 {
    pub schema_version: u16,
    pub semantics: VetoObservationSemanticsV1,
    pub configuration: Option<VetoConfigurationSnapshotV1>,
    pub context: Option<ObservedVetoContextV1>,
    pub rules: Vec<RuleExecutionObservationV1>,
    pub aggregate: VetoOutcome,
    pub failures: Vec<VetoReportFailureV1>,
    pub blocked_by_report_failure: bool,
}
impl VetoExecutionReportV1 {
    pub fn failed(failure: VetoReportFailureV1) -> Self {
        Self {
            schema_version: VETO_REPORT_SCHEMA_VERSION_V1,
            semantics: VetoObservationSemanticsV1::ObservedOnly,
            configuration: None,
            context: None,
            rules: vec![],
            aggregate: VetoOutcome::default(),
            failures: vec![failure],
            blocked_by_report_failure: false,
        }
    }
    pub fn has_report_failure(&self) -> bool {
        !self.failures.is_empty() || self.aggregate.aggregation_failure.is_some()
    }
    pub fn encode_bounded(&self) -> Result<Vec<u8>, VetoReportFailureV1> {
        struct Writer {
            bytes: Vec<u8>,
            exceeded: bool,
        }
        impl std::io::Write for Writer {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                if b.len() > MAX_REPORT_BYTES.saturating_sub(self.bytes.len()) {
                    self.exceeded = true;
                    return Err(std::io::Error::other("veto report limit"));
                }
                self.bytes.extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut w = Writer {
            bytes: Vec::new(),
            exceeded: false,
        };
        if serde_json::to_writer(&mut w, self).is_err() {
            return Err(if w.exceeded {
                VetoReportFailureV1::EncodingLimit
            } else {
                VetoReportFailureV1::EncodingFailed
            });
        }
        Ok(w.bytes)
    }
}

pub struct ConfiguredVetoChainV1 {
    pub(super) configuration: VetoConfigurationSnapshotV1,
    pub(super) chain: super::veto_chain::VetoChain,
}
impl ConfiguredVetoChainV1 {
    pub fn new(configuration: VetoConfigurationSnapshotV1) -> Self {
        let chain = super::veto_rules_live::build_configured_chain(&configuration);
        Self {
            configuration,
            chain,
        }
    }
    pub fn evaluate(&self, ctx: &VetoContext) -> VetoExecutionReportV1 {
        super::veto_execution_engine_v1::evaluate(&self.chain, ctx, Some(&self.configuration))
    }
}

#[cfg(test)]
#[path = "veto_execution_report_v1_tests.rs"]
mod tests;
