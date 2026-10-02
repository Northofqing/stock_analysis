//! Explicit candidate diagnostics; this binary cannot select a production provider.

use clap::{error::ErrorKind, ArgGroup, CommandFactory, Parser};
use sha2::{Digest, Sha256};
use std::fs;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
use std::fs::File;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use stock_analysis::grpc_client::client::candidate_probe::{
    compiled_candidate_b7_inputs, read_candidate_b7_receipt, CandidateProbePlan,
    MAX_CANDIDATE_PLAN_BYTES, MAX_CANDIDATE_RECEIPT_BYTES,
};

const PROFILE: &str = "windows-b7-20261002.17-diagnostic-v1";
const PRODUCTION_ROOT: &str = "/Users/zhangzhen/.local/share/stock-analysis-runtime";
const COMPLETED: &str = "CompletedRecordedObservation";

#[derive(Parser)]
#[command(about = "Generate, explicitly execute, or verify one fixed candidate diagnostic plan")]
#[command(group(ArgGroup::new("mode").args(["offline_plan", "execute_plan", "verify_receipt"]).required(true).multiple(false)))]
struct Args {
    #[arg(long, value_parser = parse_profile)]
    profile: String,
    #[arg(long, requires = "output", conflicts_with_all = ["bundle", "receipt_sha256"])]
    offline_plan: bool,
    #[arg(long, value_name = "PLAN", requires_all = ["bundle", "output"], conflicts_with = "receipt_sha256")]
    execute_plan: Option<PathBuf>,
    #[arg(long, value_name = "FILE", requires = "receipt_sha256", conflicts_with_all = ["bundle", "output"])]
    verify_receipt: Option<PathBuf>,
    #[arg(long, value_name = "EXISTING_ROOT", requires = "execute_plan")]
    bundle: Option<PathBuf>,
    #[arg(long, value_name = "NEW_FILE", conflicts_with = "verify_receipt")]
    output: Option<PathBuf>,
    #[arg(long, value_parser = parse_sha256, requires = "verify_receipt")]
    receipt_sha256: Option<String>,
}

fn parse_profile(value: &str) -> Result<String, &'static str> {
    if value == PROFILE {
        Ok(value.to_owned())
    } else {
        Err("the compiled diagnostic profile is required")
    }
}

fn parse_sha256(value: &str) -> Result<String, &'static str> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value.to_owned())
    } else {
        Err("a lowercase SHA-256 is required")
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LocalError {
    InputRejected,
    OutputRejected,
    PublicInputsUnavailable,
    PlanRejected,
    ReceiptRejected,
    ExecutionRejected,
    EncodingRejected,
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    )))]
    UnsupportedPlatform,
}

impl LocalError {
    fn name(&self) -> &'static str {
        match self {
            Self::InputRejected => "InputRejected",
            Self::OutputRejected => "OutputRejected",
            Self::PublicInputsUnavailable => "PublicInputsUnavailable",
            Self::PlanRejected => "PlanRejected",
            Self::ReceiptRejected => "ReceiptRejected",
            Self::ExecutionRejected => "ExecutionRejected",
            Self::EncodingRejected => "EncodingRejected",
            #[cfg(not(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "linux",
                target_os = "android"
            )))]
            Self::UnsupportedPlatform => "UnsupportedPlatform",
        }
    }
}

struct Summary {
    compiled_inputs: String,
    outcome: &'static str,
    rpc_count: usize,
    artifact_sha256: String,
    exit_code: u8,
}

async fn run(args: Args, production: &Path) -> Result<Summary, LocalError> {
    // No environment or runtime bundle participates in the compiled expected identity.
    if args.profile != PROFILE {
        return Err(LocalError::PlanRejected);
    }
    let compiled_inputs = serde_json::to_string(
        &compiled_candidate_b7_inputs().map_err(|_| LocalError::PublicInputsUnavailable)?,
    )
    .map_err(|_| LocalError::EncodingRejected)?;
    let (outcome, rpc_count, artifact_sha256, exit_code) = if args.offline_plan {
        let plan = CandidateProbePlan::windows_b7().map_err(|_| LocalError::PlanRejected)?;
        let bytes = plan
            .canonical_bytes()
            .map_err(|_| LocalError::EncodingRejected)?;
        let mut output = PrivateOutput::create(
            args.output.as_deref().ok_or(LocalError::OutputRejected)?,
            production,
        )?;
        output.write_durable(&bytes, MAX_CANDIDATE_PLAN_BYTES)?;
        ("OfflinePlanCreated", 0, sha256(&bytes), 0)
    } else if let Some(path) = args.execute_plan {
        let bytes = read_bounded_regular(&path, MAX_CANDIDATE_PLAN_BYTES, production)?;
        let plan =
            CandidateProbePlan::read_checked(&bytes).map_err(|_| LocalError::PlanRejected)?;
        let bundle = canonical_nonproduction(
            args.bundle.as_deref().ok_or(LocalError::InputRejected)?,
            production,
        )?;
        if !bundle.is_dir() {
            return Err(LocalError::InputRejected);
        }
        // Validate and reserve the new artifact before any credential load or dial.
        let mut output = PrivateOutput::create(
            args.output.as_deref().ok_or(LocalError::OutputRejected)?,
            production,
        )?;
        let receipt = plan
            .execute_from_bundle(&bundle)
            .await
            .map_err(|_| LocalError::ExecutionRejected)?;
        let bytes = receipt
            .canonical_bytes()
            .map_err(|_| LocalError::EncodingRejected)?;
        output.write_durable(&bytes, MAX_CANDIDATE_RECEIPT_BYTES)?;
        let outcome = receipt.outcome_name();
        (
            outcome,
            receipt.rpc_count(),
            sha256(&bytes),
            u8::from(outcome != COMPLETED),
        )
    } else {
        let bytes = read_bounded_regular(
            args.verify_receipt
                .as_deref()
                .ok_or(LocalError::InputRejected)?,
            MAX_CANDIDATE_RECEIPT_BYTES,
            production,
        )?;
        let evidence = read_candidate_b7_receipt(
            &bytes,
            args.receipt_sha256
                .as_deref()
                .ok_or(LocalError::ReceiptRejected)?,
        )
        .map_err(|_| LocalError::ReceiptRejected)?;
        let outcome = evidence.outcome_name();
        (
            outcome,
            evidence.rpc_count(),
            sha256(&bytes),
            u8::from(outcome != COMPLETED),
        )
    };
    Ok(Summary {
        compiled_inputs,
        outcome,
        rpc_count,
        artifact_sha256,
        exit_code,
    })
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, LocalError> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|_| LocalError::InputRejected)?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn reject_production(path: &Path, production: &Path) -> Result<(), LocalError> {
    let path = absolute_lexical(path)?;
    let production = absolute_lexical(production)?;
    if path.starts_with(&production)
        || fs::canonicalize(&production).is_ok_and(|root| path.starts_with(root))
    {
        return Err(LocalError::InputRejected);
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    ))]
    if unix_files::has_production_ancestor(&path, &production) {
        return Err(LocalError::InputRejected);
    }
    Ok(())
}

fn canonical_nonproduction(path: &Path, production: &Path) -> Result<PathBuf, LocalError> {
    reject_production(path, production)?;
    let canonical = fs::canonicalize(path).map_err(|_| LocalError::InputRejected)?;
    reject_production(&canonical, production)?;
    Ok(canonical)
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
mod unix_files {
    use super::*;
    use std::ffi::{c_char, c_int, CString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const NOFOLLOW: c_int = 0x0000_0100;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const NONBLOCK: c_int = 0x0000_0004;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const DIRECTORY: c_int = 0x0010_0000;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const CREATE_EXCLUSIVE: c_int = 0x0000_0200 | 0x0000_0800 | 0x0100_0000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NOFOLLOW: c_int = 0x0002_0000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NONBLOCK: c_int = 0x0000_0800;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const DIRECTORY: c_int = 0x0001_0000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const CREATE_EXCLUSIVE: c_int = 0x0000_0040 | 0x0000_0080 | 0x0008_0000;

    unsafe extern "C" {
        fn getuid() -> u32;
        fn geteuid() -> u32;
        fn openat(directory: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    }

    fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
        left.dev() == right.dev() && left.ino() == right.ino()
    }

    pub(super) fn has_production_ancestor(path: &Path, production: &Path) -> bool {
        let Ok(root) = fs::metadata(production) else {
            return false;
        };
        // Inode ancestry also catches case aliases on a case-insensitive filesystem.
        path.ancestors().any(|ancestor| {
            fs::metadata(ancestor)
                .is_ok_and(|metadata| metadata.is_dir() && same_file(&root, &metadata))
        })
    }

    pub(super) fn read_bounded_regular(
        path: &Path,
        cap: usize,
        production: &Path,
    ) -> Result<Vec<u8>, LocalError> {
        reject_production(path, production)?;
        let named = fs::symlink_metadata(path).map_err(|_| LocalError::InputRejected)?;
        if !named.is_file() || named.nlink() != 1 || named.len() > cap as u64 {
            return Err(LocalError::InputRejected);
        }
        canonical_nonproduction(path, production)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | NONBLOCK)
            .open(path)
            .map_err(|_| LocalError::InputRejected)?;
        let opened = file.metadata().map_err(|_| LocalError::InputRejected)?;
        if !opened.is_file()
            || opened.nlink() != 1
            || opened.len() > cap as u64
            || !same_file(&named, &opened)
        {
            return Err(LocalError::InputRejected);
        }
        let mut bytes = Vec::new();
        (&file)
            .take(cap as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| LocalError::InputRejected)?;
        let after = file.metadata().map_err(|_| LocalError::InputRejected)?;
        let current = fs::symlink_metadata(path).map_err(|_| LocalError::InputRejected)?;
        if bytes.len() > cap
            || after.len() != bytes.len() as u64
            || !current.is_file()
            || current.nlink() != 1
            || !same_file(&opened, &after)
            || !same_file(&opened, &current)
            || opened.len() != after.len()
            || opened.mtime() != after.mtime()
            || opened.mtime_nsec() != after.mtime_nsec()
            || opened.ctime() != after.ctime()
            || opened.ctime_nsec() != after.ctime_nsec()
        {
            return Err(LocalError::InputRejected);
        }
        Ok(bytes)
    }

    pub(super) struct PrivateOutput {
        file: File,
        directory: File,
        path: PathBuf,
        parent: PathBuf,
        production: PathBuf,
    }

    fn validate_directory(
        directory: &File,
        parent: &Path,
        production: &Path,
    ) -> Result<(), LocalError> {
        let canonical =
            canonical_nonproduction(parent, production).map_err(|_| LocalError::OutputRejected)?;
        let opened = directory
            .metadata()
            .map_err(|_| LocalError::OutputRejected)?;
        let named = fs::symlink_metadata(parent).map_err(|_| LocalError::OutputRejected)?;
        // A retained descriptor prevents a later path replacement from redirecting
        // openat. These current path checks reject drift; they are not a global
        // atomic guarantee against arbitrary filesystem renames.
        let (uid, effective_uid) = unsafe { (getuid(), geteuid()) };
        if canonical != parent
            || !opened.is_dir()
            || !named.is_dir()
            || !same_file(&opened, &named)
            || opened.nlink() == 0
            || named.nlink() == 0
            || opened.uid() != uid
            || effective_uid != uid
            || opened.mode() & 0o077 != 0
        {
            return Err(LocalError::OutputRejected);
        }
        Ok(())
    }

    impl PrivateOutput {
        pub(super) fn create(path: &Path, production: &Path) -> Result<Self, LocalError> {
            Self::create_checked(path, production).map_err(|_| LocalError::OutputRejected)
        }

        fn create_checked(path: &Path, production: &Path) -> Result<Self, LocalError> {
            reject_production(path, production)?;
            let leaf = path.file_name().ok_or(LocalError::OutputRejected)?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty());
            let parent = canonical_nonproduction(parent.unwrap_or(Path::new(".")), production)?;
            let named = fs::symlink_metadata(&parent).map_err(|_| LocalError::OutputRejected)?;
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(DIRECTORY | NOFOLLOW | NONBLOCK)
                .open(&parent)
                .map_err(|_| LocalError::OutputRejected)?;
            let opened = directory
                .metadata()
                .map_err(|_| LocalError::OutputRejected)?;
            // SAFETY: uid getters have no preconditions and retain no pointers.
            let (uid, effective_uid) = unsafe { (getuid(), geteuid()) };
            if !opened.is_dir()
                || !same_file(&named, &opened)
                || opened.uid() != uid
                || effective_uid != uid
                || opened.mode() & 0o077 != 0
            {
                return Err(LocalError::OutputRejected);
            }
            validate_directory(&directory, &parent, production)?;
            let path = parent.join(leaf);
            let leaf = CString::new(leaf.as_bytes()).map_err(|_| LocalError::OutputRejected)?;
            // SAFETY: the retained directory descriptor is valid, the CString lives
            // through this call, and the mode argument accompanies O_CREAT.
            let descriptor = unsafe {
                openat(
                    directory.as_raw_fd(),
                    leaf.as_ptr(),
                    1 | CREATE_EXCLUSIVE | NOFOLLOW,
                    0o600u32,
                )
            };
            if descriptor < 0 {
                return Err(LocalError::OutputRejected);
            }
            // SAFETY: a successful openat returns a newly owned descriptor.
            let file = unsafe { File::from_raw_fd(descriptor) };
            let metadata = file.metadata().map_err(|_| LocalError::OutputRejected)?;
            if !metadata.is_file()
                || metadata.nlink() != 1
                || metadata.uid() != uid
                || metadata.mode() & 0o077 != 0
            {
                return Err(LocalError::OutputRejected);
            }
            let output = Self {
                file,
                directory,
                path,
                parent,
                production: production.to_owned(),
            };
            validate_directory(&output.directory, &output.parent, &output.production)?;
            Ok(output)
        }

        pub(super) fn write_durable(&mut self, bytes: &[u8], cap: usize) -> Result<(), LocalError> {
            if bytes.len() > cap {
                return Err(LocalError::OutputRejected);
            }
            validate_directory(&self.directory, &self.parent, &self.production)?;
            self.file
                .write_all(bytes)
                .and_then(|()| self.file.sync_all())
                .and_then(|()| self.directory.sync_all())
                .map_err(|_| LocalError::OutputRejected)?;
            let opened = self
                .file
                .metadata()
                .map_err(|_| LocalError::OutputRejected)?;
            let named = fs::symlink_metadata(&self.path).map_err(|_| LocalError::OutputRejected)?;
            if !named.is_file()
                || named.nlink() != 1
                || !same_file(&opened, &named)
                || opened.nlink() != 1
                || opened.len() != bytes.len() as u64
                || opened.mode() & 0o077 != 0
            {
                return Err(LocalError::OutputRejected);
            }
            validate_directory(&self.directory, &self.parent, &self.production)?;
            Ok(())
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
use unix_files::{read_bounded_regular, PrivateOutput};

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn read_bounded_regular(_: &Path, _: usize, _: &Path) -> Result<Vec<u8>, LocalError> {
    Err(LocalError::UnsupportedPlatform)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
struct PrivateOutput;

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
impl PrivateOutput {
    fn create(_: &Path, _: &Path) -> Result<Self, LocalError> {
        Err(LocalError::UnsupportedPlatform)
    }
    fn write_durable(&mut self, _: &[u8], _: usize) -> Result<(), LocalError> {
        Err(LocalError::UnsupportedPlatform)
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) if error.kind() == ErrorKind::DisplayHelp => {
            let _ = Args::command().print_help();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            // Clap's detailed errors may echo operator input, including credentials.
            eprintln!("candidate_probe_failed=InvalidArguments");
            return ExitCode::from(2);
        }
    };
    match run(args, Path::new(PRODUCTION_ROOT)).await {
        Ok(summary) => {
            println!("compiled_candidate_inputs={}", summary.compiled_inputs);
            println!(
                "candidate_probe outcome={} rpc_count={} artifact_sha256={}",
                summary.outcome, summary.rpc_count, summary.artifact_sha256
            );
            ExitCode::from(summary.exit_code)
        }
        Err(error) => {
            eprintln!("candidate_probe_failed={}", error.name());
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[path = "grpc_candidate_bundle_probe/tests.rs"]
mod tests;
