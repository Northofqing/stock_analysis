//! Ordinary builds only generate refusal. Accepted issuance has a separate owner.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const REFUSAL_SOURCE: &[u8] = b"pub(super) const fn acquire() -> Result<ReviewedLayoutPin, LayoutPinRefusal> {\n    Err(LayoutPinRefusal::BuildVerificationNotInstalled)\n}\n";

pub(crate) fn write_refusal(out_dir: &Path) -> io::Result<()> {
    let destination = out_dir.join("replay_layout_pin_v1_refusal.rs");
    let directive_path = destination
        .to_str()
        .filter(|path| !path.contains('\r') && !path.contains('\n'))
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let pending = out_dir.join("replay_layout_pin_v1_refusal.rs.pending");
    // An interrupted writer leaves a refusing obstruction, never accepted text.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    file.write_all(REFUSAL_SOURCE)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&pending, &destination)?;

    // Emit the usable include only after the complete refusal replaces old bytes.
    // No environment value, certificate or source text is read by this helper.
    println!("cargo:rustc-env=STOCK_REPLAY_PIN_INCLUDE={directive_path}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/replay_layout_pin_v1.rs");
    println!("cargo:rerun-if-env-changed=STOCK_REPLAY_PIN_INCLUDE");
    Ok(())
}
