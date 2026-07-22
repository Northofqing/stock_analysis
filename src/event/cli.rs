//! Registered business rules: BR-043, BR-141.
//! Event subcommand CLI parser — v17.3 Task 4
//!
//! Parses `--replay`, `--history`, and related flags from the monitor's
//! argument list without disturbing the existing `--test`, `--review`,
//! `--push`, `--e2e`, `--v13-diag` flags.

use chrono::NaiveDate;
use thiserror::Error;

// ========================================================================
// CliError
// ========================================================================

/// Errors from parsing event CLI arguments.
#[derive(Error, Debug)]
pub enum CliError {
    #[error("malformed date '{0}': expected YYYY-MM-DD")]
    MalformedDate(String),

    #[error("negative limit is not allowed: {0}")]
    InvalidLimit(i64),

    #[error("'{0}' is not a valid non-negative integer")]
    InvalidInteger(String),

    #[error("--replay-force requires --replay")]
    ReplayForceWithoutReplay,

    #[error("--replay-rate-ms requires --replay")]
    ReplayRateWithoutReplay,

    #[error("unrecognized flag: {0}")]
    UnrecognizedFlag(String),

    #[error("unrecognized argument at position {0}: {1}")]
    UnrecognizedArg(usize, String),
}

// ========================================================================
// EventCommand
// ========================================================================

/// Event subcommands that can be parsed from the monitor's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventCommand {
    Replay {
        date: NaiveDate,
        force: bool,
        rate_ms: u32,
    },
    History {
        date: Option<NaiveDate>,
        code: Option<String>,
        kind: Option<String>,
        limit: Option<usize>,
        success_rate: bool,
        sink: Option<String>,
    },
    Help,
}

// ========================================================================
// parse_args
// ========================================================================

/// Parse event subcommand flags from a flat argument list.
///
/// Returns `Ok(Some(EventCommand))` when an event flag is recognized,
/// `Ok(None)` when no event flag is present (caller should use existing
/// monitor behavior), and `Err(CliError)` on a malformed flag combination.
///
/// Supports whitespace-tolerant forms:
/// - `--replay=YYYY-MM-DD [--replay-force] [--replay-rate-ms=N]`
/// - `--history --date=YYYY-MM-DD [--code=CODE] [--kind=KIND] [--limit=N]`
/// - `--history --success-rate [--date=YYYY-MM-DD] [--kind=KIND] [--sink=SINK]`
pub fn parse_args(args: &[&str]) -> Result<Option<EventCommand>, CliError> {
    let mut args_iter = args.iter().enumerate();
    let mut has_replay = false;
    let mut has_history = false;
    let mut replay_date: Option<NaiveDate> = None;
    let mut replay_force = false;
    let mut replay_rate_ms: Option<u32> = None;
    let mut history_date: Option<NaiveDate> = None;
    let mut history_code: Option<String> = None;
    let mut history_kind: Option<String> = None;
    let mut history_limit: Option<usize> = None;
    let mut history_success_rate = false;
    let mut history_sink: Option<String> = None;
    let mut help_requested = false;

    while let Some((idx, arg)) = args_iter.next() {
        match *arg {
            "--help" | "-h" => {
                help_requested = true;
            }
            "--test"
            | "--review"
            | "--push"
            | "--push-dry-run"
            | "--e2e"
            | "--v13-diag"
            | "--backfill-st-type"
            | "--backfill-chain-name" => {
                // Known monitor flags may be combined with terminal event commands.
                // Ignore them here; if no event command is present we still return None.
            }
            s if s.starts_with("--backfill-outcome=") => {
                let date_str = &s["--backfill-outcome=".len()..];
                parse_date(date_str)?;
            }
            "--replay" => {
                has_replay = true;
            }
            "--replay-force" => {
                if !has_replay {
                    return Err(CliError::ReplayForceWithoutReplay);
                }
                replay_force = true;
            }
            "--replay-rate-ms" => {
                if !has_replay {
                    return Err(CliError::ReplayRateWithoutReplay);
                }
                // Grab the next positional arg as the value
                let (_, val) = args_iter.next().ok_or_else(|| {
                    CliError::InvalidInteger("missing value after --replay-rate-ms".into())
                })?;
                let ms: u32 = val
                    .parse()
                    .map_err(|_| CliError::InvalidInteger(val.to_string()))?;
                replay_rate_ms = Some(ms);
            }
            s if s.starts_with("--replay-rate-ms=") => {
                if !has_replay {
                    return Err(CliError::ReplayRateWithoutReplay);
                }
                let value = &s["--replay-rate-ms=".len()..];
                let ms: u32 = value
                    .parse()
                    .map_err(|_| CliError::InvalidInteger(value.to_string()))?;
                replay_rate_ms = Some(ms);
            }
            s if s.starts_with("--replay=") => {
                has_replay = true;
                let date_str = &s["--replay=".len()..];
                let date = parse_date(date_str)?;
                replay_date = Some(date);
            }
            "--history" => {
                has_history = true;
            }
            "--success-rate" => {
                has_history = true;
                history_success_rate = true;
            }
            s if s.starts_with("--date=") => {
                let date_str = &s["--date=".len()..];
                let date = parse_date(date_str)?;
                if has_replay {
                    replay_date = Some(date);
                } else {
                    history_date = Some(date);
                }
            }
            s if s.starts_with("--code=") => {
                if !has_history {
                    return Err(CliError::UnrecognizedFlag(s.to_string()));
                }
                history_code = Some(s["--code=".len()..].to_string());
            }
            s if s.starts_with("--kind=") => {
                if !has_history {
                    return Err(CliError::UnrecognizedFlag(s.to_string()));
                }
                history_kind = Some(s["--kind=".len()..].to_string());
            }
            s if s.starts_with("--limit=") => {
                if !has_history {
                    return Err(CliError::UnrecognizedFlag(s.to_string()));
                }
                let limit_str = &s["--limit=".len()..];
                let limit: i64 = limit_str
                    .parse()
                    .map_err(|_| CliError::InvalidInteger(limit_str.to_string()))?;
                if limit < 0 {
                    return Err(CliError::InvalidLimit(limit));
                }
                history_limit = Some(limit as usize);
            }
            s if s.starts_with("--sink=") => {
                if !has_history {
                    return Err(CliError::UnrecognizedFlag(s.to_string()));
                }
                history_sink = Some(s["--sink=".len()..].to_string());
            }
            s if s.starts_with("--") => {
                return Err(CliError::UnrecognizedFlag(s.to_string()));
            }
            _ => {
                // Allow the program name (e.g. "monitor") as the first positional arg.
                // Only reject truly unexpected positional arguments.
                if idx == 0
                    && (arg == &"monitor"
                        || arg.ends_with("/monitor")
                        || arg.ends_with("\\monitor"))
                {
                    // skip program name
                } else {
                    return Err(CliError::UnrecognizedArg(idx, arg.to_string()));
                }
            }
        }
    }

    if help_requested {
        return Ok(Some(EventCommand::Help));
    }

    if has_replay {
        let date = replay_date.ok_or_else(|| {
            CliError::MalformedDate("missing date for --replay, use --replay=YYYY-MM-DD".into())
        })?;
        return Ok(Some(EventCommand::Replay {
            date,
            force: replay_force,
            rate_ms: replay_rate_ms.unwrap_or(0),
        }));
    }

    if has_history {
        return Ok(Some(EventCommand::History {
            date: history_date,
            code: history_code,
            kind: history_kind,
            limit: history_limit,
            success_rate: history_success_rate,
            sink: history_sink,
        }));
    }

    Ok(None)
}

// ========================================================================
// Internal helpers
// ========================================================================

fn parse_date(s: &str) -> Result<NaiveDate, CliError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| CliError::MalformedDate(s.to_string()))
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_replay_date() {
        let cmd = parse_args(&["monitor", "--replay=2026-07-16"]).unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::Replay { date, force: false, rate_ms: 0 })
            if date == NaiveDate::from_ymd_opt(2026, 7, 16).unwrap()
        ));
    }

    #[test]
    fn cli_parses_replay_with_force() {
        let cmd = parse_args(&["monitor", "--replay=2026-07-16", "--replay-force"]).unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::Replay { force: true, .. })
        ));
    }

    #[test]
    fn cli_parses_replay_with_rate_ms() {
        let cmd =
            parse_args(&["monitor", "--replay=2026-07-16", "--replay-rate-ms", "100"]).unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::Replay { rate_ms: 100, .. })
        ));
    }

    #[test]
    fn cli_parses_documented_replay_rate_equals_form() {
        let cmd = parse_args(&["monitor", "--replay=2026-07-16", "--replay-rate-ms=100"]).unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::Replay { rate_ms: 100, .. })
        ));
    }

    #[test]
    fn cli_keeps_history_command_when_monitor_flags_are_present() {
        for args in [
            vec!["monitor", "--test", "--history", "--success-rate"],
            vec!["monitor", "--history", "--success-rate", "--test"],
        ] {
            let cmd = parse_args(&args).unwrap();
            assert!(matches!(
                cmd,
                Some(EventCommand::History {
                    success_rate: true,
                    ..
                })
            ));
        }
    }

    #[test]
    fn cli_parses_history_with_filters() {
        // Protocol-format exception: CLI history filters intentionally accept
        // the documented native six-digit stock-code syntax.
        let cmd = parse_args(&[
            "monitor",
            "--history",
            "--date=2026-07-16",
            "--code=600519",
            "--kind=Announcement",
            "--limit=50",
        ])
        .unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::History {
                date,
                code,
                kind,
                limit,
                success_rate: false,
                sink: None,
            })
            if date == Some(NaiveDate::from_ymd_opt(2026, 7, 16).unwrap())
                && code == Some("600519".to_string())
                && kind == Some("Announcement".to_string())
                && limit == Some(50)
        ));
    }

    #[test]
    fn cli_parses_history_success_rate() {
        let cmd = parse_args(&[
            "monitor",
            "--history",
            "--success-rate",
            "--date=2026-07-16",
            "--kind=Announcement",
            "--sink=dry_run",
        ])
        .unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::History {
                success_rate: true,
                sink: Some(s),
                ..
            })
            if s == "dry_run"
        ));
    }

    #[test]
    fn cli_returns_none_when_no_event_flags() {
        let cmd = parse_args(&["monitor", "--test"]).unwrap();
        assert!(cmd.is_none());

        let cmd = parse_args(&["monitor", "--review"]).unwrap();
        assert!(cmd.is_none());

        let cmd = parse_args(&["monitor", "--push"]).unwrap();
        assert!(cmd.is_none());

        let cmd = parse_args(&["monitor", "--e2e"]).unwrap();
        assert!(cmd.is_none());

        let cmd = parse_args(&["monitor", "--test", "--v13-diag"]).unwrap();
        assert!(cmd.is_none());

        for flag in [
            "--push-dry-run",
            "--backfill-outcome=2026-07-21",
            "--backfill-st-type",
            "--backfill-chain-name",
        ] {
            let cmd = parse_args(&["monitor", flag]).unwrap();
            assert!(cmd.is_none(), "known one-shot flag rejected: {flag}");
        }
    }

    #[test]
    fn cli_rejects_empty_or_malformed_backfill_outcome_dates() {
        for flag in ["--backfill-outcome=", "--backfill-outcome=not-a-date"] {
            let error = parse_args(&["monitor", flag]).expect_err("invalid date must fail");
            assert!(error.to_string().contains("malformed date"), "{error}");
        }
    }

    #[test]
    fn cli_rejects_replay_force_without_replay() {
        let err = parse_args(&["monitor", "--replay-force"]).unwrap_err();
        assert!(err.to_string().contains("replay-force"));
    }

    #[test]
    fn cli_rejects_replay_rate_without_replay() {
        let err = parse_args(&["monitor", "--replay-rate-ms", "50"]).unwrap_err();
        assert!(err.to_string().contains("replay-rate-ms"));
    }

    #[test]
    fn cli_rejects_malformed_date() {
        let err = parse_args(&["monitor", "--replay=not-a-date"]).unwrap_err();
        assert!(err.to_string().contains("malformed date"));
    }

    #[test]
    fn cli_rejects_negative_limit() {
        let err = parse_args(&["monitor", "--history", "--limit=-5"]).unwrap_err();
        assert!(err.to_string().contains("limit"));
    }

    #[test]
    fn cli_preserves_zero_as_explicit_unbounded_history_limit() {
        let cmd = parse_args(&["monitor", "--history", "--limit=0"]).unwrap();
        assert!(matches!(
            cmd,
            Some(EventCommand::History { limit: Some(0), .. })
        ));
    }

    #[test]
    fn cli_returns_help_for_help_flag() {
        let cmd = parse_args(&["monitor", "--help"]).unwrap();
        assert!(matches!(cmd, Some(EventCommand::Help)));
    }

    #[test]
    fn cli_rejects_unrecognized_flags() {
        let err = parse_args(&["monitor", "--unknown-flag"]).unwrap_err();
        assert!(err.to_string().contains("unrecognized"));
    }
}

#[cfg(test)]
#[path = "../gate_d_event_cli_regression.rs"]
mod gate_d_regression;
