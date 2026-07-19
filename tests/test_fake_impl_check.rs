//! AGENTS.md §2.8 fake-implementation scanner fixture tests.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

fn fixture(name: &str, source: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir()
        .join("stock_analysis_fake_impl_tests")
        .join(format!(
            "{}-{}-{name}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.rs"), source).unwrap();
    let required_test = root.join("required_test.rs");
    std::fs::write(&required_test, "// fixture\n").unwrap();
    (src, required_test)
}

fn run(src: &PathBuf, required_test: &PathBuf) -> std::process::Output {
    Command::new("bash")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tools/compliance/lib/check_fake_impl.sh"),
        )
        .env("FAKE_IMPL_SRC_DIR", src)
        .env("FAKE_IMPL_REQUIRED_TEST", required_test)
        .output()
        .expect("应能运行假实现检查")
}

#[test]
fn logging_only_save_is_rejected_but_real_delegation_passes() {
    let (fake_src, fake_test) = fixture(
        "fake",
        r#"
pub fn save_record(value: &str) -> Result<(), String> {
    let _ = value;
    log::info!("saved");
    Ok(())
}
"#,
    );
    let fake = run(&fake_src, &fake_test);
    assert!(!fake.status.success(), "日志后 Ok(()) 必须被拒绝");
    assert!(
        String::from_utf8_lossy(&fake.stderr).contains("logging-only/literal result"),
        "stderr={}",
        String::from_utf8_lossy(&fake.stderr)
    );

    let (real_src, real_test) = fixture(
        "real",
        r#"
pub fn save_record(db: &Database, value: &str) -> Result<(), String> {
    db.insert(value)?;
    Ok(())
}
"#,
    );
    let real = run(&real_src, &real_test);
    assert!(
        real.status.success(),
        "委托真实目标操作应通过: stderr={}",
        String::from_utf8_lossy(&real.stderr)
    );
}

#[test]
fn unimplemented_notify_is_rejected() {
    let (src, required_test) = fixture(
        "todo",
        r#"
pub async fn notify_owner() -> Result<(), String> {
    unimplemented!("transport not implemented")
}
"#,
    );
    let output = run(&src, &required_test);
    assert!(!output.status.success(), "unimplemented notify 必须被拒绝");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("todo/unimplemented/stub"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
