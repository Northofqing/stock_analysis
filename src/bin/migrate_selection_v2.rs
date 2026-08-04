use std::process::ExitCode;

// Business-Rule: BR-180 fixed-root, fail-closed selection-v2 migration command.
fn main() -> ExitCode {
    match stock_analysis::database::run_selection_v2_migration_command(std::env::args_os().skip(1))
    {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("selection-v2 migration refused: {message}");
            ExitCode::from(2)
        }
    }
}
