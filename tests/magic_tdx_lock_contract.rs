#[path = "../build_support/magic_tdx_lock.rs"]
mod magic_tdx_lock;

const REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";

fn source(query: &str, resolved: &str) -> String {
    format!("git+https://github.com/Northofqing/magic-market-data-rs.git?{query}#{resolved}")
}

#[test]
fn exact_full_revision_query_is_accepted() {
    let locked_source = source(&format!("rev={REVISION}"), REVISION);

    assert_eq!(
        magic_tdx_lock::exact_locked_magic_tdx_revision(&locked_source),
        Ok(REVISION)
    );
}

#[test]
fn branch_tag_short_extra_and_mismatched_queries_are_rejected() {
    let other_revision = "85ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";
    let rejected = [
        source("branch=main", REVISION),
        source("tag=v0.2.0", REVISION),
        source("rev=75ee2a2b", REVISION),
        source(&format!("rev={REVISION}&branch=main"), REVISION),
        source(&format!("rev={other_revision}"), REVISION),
    ];

    for locked_source in rejected {
        assert!(
            magic_tdx_lock::exact_locked_magic_tdx_revision(&locked_source).is_err(),
            "non-exact locked source must fail: {locked_source}"
        );
    }
}
