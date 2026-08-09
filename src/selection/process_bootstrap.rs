//! BR-179/BR-183 process-owned selection bootstrap.
//!
//! This module is the only selection startup authority allowed to read the
//! process argv. It publishes an opaque proof for storage-free terminal and
//! service-disabled states plus operational core dispatch. Until the private
//! mode-bound database/lease/catalog/receipt factory exists, selection-v2 is
//! explicitly disabled without blocking independent core business. No caller
//! can provide argv, a mode, or a database path.

use crate::event::cli::EventCommand;
use std::ffi::OsString;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static BOOTSTRAP_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static BOUND_SELECTION_PROCESS: OnceLock<BoundSelectionProcess> = OnceLock::new();

/// Opaque proof that the real process argv was parsed exactly once.
///
/// The value intentionally does not implement `Clone` or serialization.  Its
/// accessors expose dispatch facts only, never raw argv, mode enums, paths, or
/// the private process binding.
///
/// ```compile_fail
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<stock_analysis::selection::VerifiedParsedSelectionCli>();
/// ```
///
/// ```compile_fail
/// let _forged = stock_analysis::selection::VerifiedParsedSelectionCli {
///     generation: 1,
///     _private: (),
/// };
/// ```
#[must_use = "selection startup must retain the parsed CLI proof"]
pub struct VerifiedParsedSelectionCli {
    generation: u64,
    _private: (),
}

impl fmt::Debug for VerifiedParsedSelectionCli {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedParsedSelectionCli")
            .finish_non_exhaustive()
    }
}

impl VerifiedParsedSelectionCli {
    pub fn is_help(&self) -> bool {
        matches!(self.parsed().terminal, Some(TerminalState::Help))
    }

    pub fn is_version(&self) -> bool {
        matches!(self.parsed().terminal, Some(TerminalState::Version))
    }

    pub fn is_test(&self) -> bool {
        self.parsed().is_test()
    }

    pub fn is_review(&self) -> bool {
        self.parsed().review
    }

    pub fn is_e2e(&self) -> bool {
        self.parsed().e2e
    }

    pub fn is_v13_diag(&self) -> bool {
        self.parsed().v13_diag
    }

    pub fn is_push(&self) -> bool {
        self.parsed().push
    }

    pub fn is_push_dry_run(&self) -> bool {
        self.parsed().push_dry_run
    }

    pub fn is_backfill_st_type(&self) -> bool {
        self.parsed().backfill_st_type
    }

    pub fn is_backfill_chain_name(&self) -> bool {
        self.parsed().backfill_chain_name
    }

    pub fn event_command(&self) -> Option<EventCommand> {
        self.parsed().event_command.clone()
    }

    pub fn requires_service_enablement(&self) -> bool {
        self.parsed().explicit_argument_count == 0
    }

    pub fn is_service_disabled(&self) -> bool {
        matches!(self.bound(), BoundSelectionProcess::Disabled { .. })
    }

    pub fn selection_v2_disabled_reason_code(&self) -> Option<&'static str> {
        match self.bound() {
            BoundSelectionProcess::Operational {
                selection: SelectionCapabilityState::Disabled { reason_code },
                ..
            } => Some(*reason_code),
            BoundSelectionProcess::Operational {
                selection: SelectionCapabilityState::Enabled,
                ..
            }
            | BoundSelectionProcess::Terminal { .. }
            | BoundSelectionProcess::Disabled { .. } => None,
            BoundSelectionProcess::Rejected { .. } => {
                unreachable!("a rejected bootstrap cannot create a verified CLI proof")
            }
        }
    }

    /// Validate one selection-facing symbol against the process-bound mode.
    ///
    /// This exposes only the isolation decision. It never reveals or accepts a
    /// mode, path, namespace or database handle.
    pub fn validate_selection_symbol(
        &self,
        symbol: &str,
    ) -> Result<(), SelectionProcessBootstrapError> {
        self.parsed().validate_symbol(symbol)
    }

    fn bound(&self) -> &'static BoundSelectionProcess {
        let bound = BOUND_SELECTION_PROCESS
            .get()
            .expect("verified CLI proof requires installed process binding");
        let generation = match bound {
            BoundSelectionProcess::Terminal { generation, .. }
            | BoundSelectionProcess::Disabled { generation, .. }
            | BoundSelectionProcess::Operational { generation, .. } => *generation,
            BoundSelectionProcess::Rejected { .. } => {
                unreachable!("a rejected bootstrap cannot create a verified CLI proof")
            }
        };
        assert_eq!(
            self.generation, generation,
            "verified CLI proof belongs to a different bootstrap generation"
        );
        bound
    }

    fn parsed(&self) -> &'static ParsedSelectionCli {
        match self.bound() {
            BoundSelectionProcess::Terminal { parsed, .. }
            | BoundSelectionProcess::Disabled { parsed, .. }
            | BoundSelectionProcess::Operational { parsed, .. } => parsed,
            BoundSelectionProcess::Rejected { .. } => {
                unreachable!("a rejected bootstrap cannot create a verified CLI proof")
            }
        }
    }
}

#[derive(Debug)]
pub struct SelectionProcessBootstrapError {
    code: &'static str,
    detail: String,
}

impl SelectionProcessBootstrapError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for SelectionProcessBootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for SelectionProcessBootstrapError {}

/// Parse and bind the actual process invocation exactly once.
///
/// Help/version and service-disabled bare startup are storage-free states.
/// Invalid argv is installed as a storage-free rejected state. Operational
/// argv receives a core dispatch proof while selection-v2 remains explicitly
/// disabled until its private global database/lease/catalog/receipt factory is
/// fully verified and released.
pub fn bootstrap_selection_process(
) -> Result<VerifiedParsedSelectionCli, SelectionProcessBootstrapError> {
    if BOOTSTRAP_ATTEMPTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(repeated_bootstrap_error(BOUND_SELECTION_PROCESS.get()));
    }

    BOUND_SELECTION_PROCESS
        .set(closed_state_from_real_argv())
        .map_err(|_| {
            SelectionProcessBootstrapError::new(
                "selection_bootstrap_install_conflict",
                "selection process binding already contains a closed state",
            )
        })?;
    let bound = BOUND_SELECTION_PROCESS
        .get()
        .expect("successful bootstrap installation must be readable");
    match bound {
        BoundSelectionProcess::Terminal { generation, .. }
        | BoundSelectionProcess::Disabled { generation, .. }
        | BoundSelectionProcess::Operational { generation, .. } => Ok(VerifiedParsedSelectionCli {
            generation: *generation,
            _private: (),
        }),
        BoundSelectionProcess::Rejected { code, detail } => {
            Err(SelectionProcessBootstrapError::new(code, detail.clone()))
        }
    }
}

fn closed_state_from_real_argv() -> BoundSelectionProcess {
    let argv = std::env::args_os().collect::<Vec<_>>();
    let parsed = match parse_process_args(&argv) {
        Ok(parsed) => parsed,
        Err(error) => {
            return BoundSelectionProcess::Rejected {
                code: error.code,
                detail: error.detail,
            };
        }
    };

    classify_parsed_invocation(parsed, service_enabled_from_environment())
}

fn service_enabled_from_environment() -> bool {
    std::env::var("MONITOR_ENABLED")
        .unwrap_or_default()
        .to_lowercase()
        == "true"
}

fn classify_parsed_invocation(
    parsed: ParsedSelectionCli,
    service_enabled: bool,
) -> BoundSelectionProcess {
    if parsed.terminal.is_some() {
        return BoundSelectionProcess::Terminal {
            generation: 1,
            parsed,
        };
    }
    if parsed.explicit_argument_count == 0 && !service_enabled {
        return BoundSelectionProcess::Disabled {
            generation: 1,
            parsed,
        };
    }
    BoundSelectionProcess::Operational {
        generation: 1,
        parsed,
        selection: match crate::selection::activation_gate::evaluate_production_selection_v2_activation()
        {
            crate::selection::activation_gate::SelectionV2ActivationVerdict::Enabled => {
                SelectionCapabilityState::Enabled
            }
            crate::selection::activation_gate::SelectionV2ActivationVerdict::Disabled {
                reason_code,
            } => SelectionCapabilityState::Disabled { reason_code },
        },
    }
}

fn repeated_bootstrap_error(
    bound: Option<&BoundSelectionProcess>,
) -> SelectionProcessBootstrapError {
    let detail = match bound {
        Some(BoundSelectionProcess::Terminal { .. }) => {
            "terminal parsed-CLI state is already installed".to_owned()
        }
        Some(BoundSelectionProcess::Disabled { .. }) => {
            "service-disabled parsed-CLI state is already installed".to_owned()
        }
        Some(BoundSelectionProcess::Operational { .. }) => {
            "operational core state is already installed".to_owned()
        }
        Some(BoundSelectionProcess::Rejected { code, detail }) => {
            format!("prior bootstrap was rejected with {code}: {detail}")
        }
        None => {
            "another bootstrap attempt is in progress or terminated before installation".to_owned()
        }
    };
    SelectionProcessBootstrapError::new("selection_bootstrap_repeated", detail)
}

enum BoundSelectionProcess {
    Terminal {
        generation: u64,
        parsed: ParsedSelectionCli,
    },
    Disabled {
        generation: u64,
        parsed: ParsedSelectionCli,
    },
    Operational {
        generation: u64,
        parsed: ParsedSelectionCli,
        selection: SelectionCapabilityState,
    },
    Rejected {
        code: &'static str,
        detail: String,
    },
}

enum SelectionCapabilityState {
    Enabled,
    Disabled { reason_code: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalState {
    Help,
    Version,
}

#[derive(Debug)]
struct ParsedSelectionCli {
    terminal: Option<TerminalState>,
    mode: SelectionProcessMode,
    review: bool,
    e2e: bool,
    v13_diag: bool,
    push: bool,
    push_dry_run: bool,
    backfill_st_type: bool,
    backfill_chain_name: bool,
    event_command: Option<EventCommand>,
    explicit_argument_count: usize,
}

impl ParsedSelectionCli {
    fn is_test(&self) -> bool {
        matches!(self.mode, SelectionProcessMode::Test)
    }

    fn validate_symbol(&self, symbol: &str) -> Result<(), SelectionProcessBootstrapError> {
        let accepted = match self.mode {
            SelectionProcessMode::Production => is_six_ascii_digits(symbol),
            SelectionProcessMode::Test => symbol
                .strip_prefix("TEST_CODE_")
                .is_some_and(is_six_ascii_digits),
        };
        if accepted {
            Ok(())
        } else {
            Err(SelectionProcessBootstrapError::new(
                "selection_symbol_mode_mismatch",
                "symbol does not satisfy the opaque process-mode contract",
            ))
        }
    }
}

fn is_six_ascii_digits(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_process_args(
    argv: &[OsString],
) -> Result<ParsedSelectionCli, SelectionProcessBootstrapError> {
    if argv.is_empty() {
        return Err(SelectionProcessBootstrapError::new(
            "selection_cli_empty",
            "process argv is empty",
        ));
    }
    let args = argv
        .iter()
        .map(|argument| {
            argument.clone().into_string().map_err(|_| {
                SelectionProcessBootstrapError::new(
                    "selection_cli_non_utf8",
                    "monitor argv must be valid UTF-8",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if args
        .iter()
        .skip(1)
        .any(|argument| matches!(argument.as_str(), "--buy" | "--sell"))
    {
        return Err(SelectionProcessBootstrapError::new(
            "legacy_manual_trade_bypass_rejected",
            "--buy/--sell bypasses are permanently disabled",
        ));
    }

    let explicit_args = &args[1..];
    let terminal = match explicit_args {
        [argument] if matches!(argument.as_str(), "--help" | "-h") => Some(TerminalState::Help),
        [argument] if matches!(argument.as_str(), "--version" | "-V") => {
            Some(TerminalState::Version)
        }
        _ if explicit_args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V")) =>
        {
            return Err(SelectionProcessBootstrapError::new(
                "selection_terminal_combination_invalid",
                "help/version must be the process's only explicit argument",
            ));
        }
        _ => None,
    };
    let event_command = match terminal {
        Some(TerminalState::Help) => Some(EventCommand::Help),
        Some(TerminalState::Version) => None,
        _ => {
            let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            crate::event::cli::parse_args(&refs).map_err(|error| {
                SelectionProcessBootstrapError::new("selection_cli_invalid", error.to_string())
            })?
        }
    };
    let test = has_exact_flag(&args, "--test");
    let mode = if test {
        SelectionProcessMode::Test
    } else {
        SelectionProcessMode::Production
    };
    let e2e = has_exact_flag(&args, "--e2e");
    let v13_diag = has_exact_flag(&args, "--v13-diag");
    if has_exact_flag(&args, "--e2e") && !test {
        return Err(SelectionProcessBootstrapError::new(
            "selection_e2e_requires_test",
            "--e2e requires the explicit --test process binding",
        ));
    }
    if v13_diag && !test {
        return Err(SelectionProcessBootstrapError::new(
            "selection_diag_requires_test",
            "--v13-diag requires the explicit --test process binding",
        ));
    }

    Ok(ParsedSelectionCli {
        terminal,
        mode,
        review: has_exact_flag(&args, "--review"),
        e2e,
        v13_diag,
        push: has_exact_flag(&args, "--push"),
        push_dry_run: has_exact_flag(&args, "--push-dry-run"),
        backfill_st_type: has_exact_flag(&args, "--backfill-st-type"),
        backfill_chain_name: has_exact_flag(&args, "--backfill-chain-name"),
        event_command,
        explicit_argument_count: args.len().saturating_sub(1),
    })
}

fn has_exact_flag(args: &[String], flag: &str) -> bool {
    args.iter().skip(1).any(|argument| argument == flag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionProcessMode {
    Production,
    Test,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParsedSelectionCli, SelectionProcessBootstrapError> {
        parse_process_args(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn strict_parser_binds_test_review_without_exposing_raw_argv() {
        let parsed = parse(&["monitor", "--test", "--review"]).expect("strict test review");
        assert_eq!(parsed.mode, SelectionProcessMode::Test);
        assert!(parsed.review);
        assert!(!parsed.e2e);
        assert_eq!(parsed.explicit_argument_count, 2);
    }

    #[test]
    fn test_push_dry_run_selects_the_complete_template_e2e_path() {
        let parsed =
            parse(&["monitor", "--test", "--push-dry-run"]).expect("complete template dry-run");
        assert_eq!(parsed.mode, SelectionProcessMode::Test);
        assert!(!parsed.e2e);
        assert!(parsed.push_dry_run);

        let review = parse(&["monitor", "--test", "--review", "--push-dry-run"])
            .expect("isolated review dry-run");
        assert!(!review.e2e);
        assert!(review.review);
    }

    #[test]
    fn process_bound_symbol_contracts_are_exact_and_disjoint() {
        let production = parse(&["monitor", "--review"]).expect("production mode");
        production
            .validate_symbol("600519")
            .expect("canonical production symbol");
        for symbol in ["TEST_CODE_600519", "sh600519", "60051", "６００５１９"] {
            let error = production
                .validate_symbol(symbol)
                .expect_err("production symbol must be rejected");
            assert_eq!(error.code(), "selection_symbol_mode_mismatch");
        }

        let test = parse(&["monitor", "--test", "--review"]).expect("test mode");
        test.validate_symbol("TEST_CODE_600519")
            .expect("canonical TEST_CODE symbol");
        for symbol in [
            "600519",
            "TEST_CODE600519",
            "TEST_CODE_60051",
            "TEST_CODE_６００５１９",
        ] {
            let error = test
                .validate_symbol(symbol)
                .expect_err("test symbol must be rejected");
            assert_eq!(error.code(), "selection_symbol_mode_mismatch");
        }
    }

    #[test]
    fn strict_parser_rejects_test_only_modes_without_test_binding() {
        for flag in ["--e2e", "--v13-diag"] {
            let error = parse(&["monitor", flag]).expect_err("test-only mode must fail");
            assert!(matches!(
                error.code(),
                "selection_e2e_requires_test" | "selection_diag_requires_test"
            ));
        }
    }

    #[test]
    fn help_is_terminal_and_requires_no_operational_binding() {
        let parsed = parse(&["monitor", "--help"]).expect("help");
        assert_eq!(parsed.terminal, Some(TerminalState::Help));
        assert!(matches!(parsed.event_command, Some(EventCommand::Help)));
    }

    #[test]
    fn version_is_an_exact_storage_free_terminal_request() {
        for flag in ["--version", "-V"] {
            let parsed = parse(&["monitor", flag]).expect("version");
            assert_eq!(parsed.terminal, Some(TerminalState::Version));
            assert!(parsed.event_command.is_none());
            assert_eq!(parsed.explicit_argument_count, 1);
        }
    }

    #[test]
    fn terminal_flags_cannot_be_combined_with_operational_arguments() {
        for args in [
            &["monitor", "--help", "--review"][..],
            &["monitor", "--version", "--test"][..],
        ] {
            let error = parse(args).expect_err("mixed terminal argv must fail");
            assert_eq!(error.code(), "selection_terminal_combination_invalid");
        }
    }

    #[test]
    fn bare_disabled_invocation_is_a_storage_free_closed_state() {
        let parsed = parse(&["monitor"]).expect("bare invocation");
        let state = classify_parsed_invocation(parsed, false);
        assert!(matches!(state, BoundSelectionProcess::Disabled { .. }));
    }

    #[test]
    fn operational_invocation_gates_selection_against_release_materials() {
        let parsed = parse(&["monitor", "--review"]).expect("review invocation");
        let state = classify_parsed_invocation(parsed, true);
        // 仓库自 Phase 0 (2026-08-07) 起携带激活材料; 生效时刻已过后 gate
        // 返回 Enabled, 未生效时返回 BR-193 具体令牌。断言必须覆盖两态,
        // 且绝不允许旧占位符 "selection_v2_activation_not_released"。
        match state {
            BoundSelectionProcess::Operational {
                selection: SelectionCapabilityState::Enabled,
                ..
            } => {}
            BoundSelectionProcess::Operational {
                selection: SelectionCapabilityState::Disabled { reason_code },
                ..
            } => {
                assert_ne!(
                    reason_code, "selection_v2_activation_not_released",
                    "BR-193 门已重接, 旧占位符必须消失"
                );
            }
            _ => panic!("expected Operational bound process with Enabled or BR-193 Disabled token"),
        }
    }

    #[test]
    fn closed_process_state_cell_accepts_exactly_one_installation() {
        let cell = OnceLock::new();
        cell.set(BoundSelectionProcess::Rejected {
            code: "TEST_CODE_first_rejection",
            detail: "first".to_owned(),
        })
        .unwrap_or_else(|_| panic!("first closed state"));

        let second = cell.set(BoundSelectionProcess::Rejected {
            code: "TEST_CODE_second_rejection",
            detail: "second".to_owned(),
        });
        assert!(second.is_err(), "second initializer replaced closed state");
        assert!(matches!(
            cell.get(),
            Some(BoundSelectionProcess::Rejected {
                code: "TEST_CODE_first_rejection",
                detail,
            })
            if detail == "first"
        ));
    }

    #[test]
    fn public_facade_reads_real_argv_once_and_rejects_every_second_attempt() {
        if let Err(error) = bootstrap_selection_process() {
            assert_eq!(error.code(), "selection_cli_invalid");
        }

        let second = bootstrap_selection_process().expect_err("second attempt must be fatal");
        assert_eq!(second.code(), "selection_bootstrap_repeated");
    }

    #[test]
    fn manual_trade_bypass_is_rejected_by_the_owner_parser() {
        let error = parse(&["monitor", "--buy"]).expect_err("manual bypass must fail");
        assert_eq!(error.code(), "legacy_manual_trade_bypass_rejected");
    }
}
