//! The production data identity is selected at build time, never from runtime env.
use std::path::{Component, Path};

pub fn select_build_root<'a>(
    manifest: &'a str,
    configured: Option<&'a str>,
) -> Result<&'a str, &'static str> {
    let Some(root) = configured else {
        return Ok(manifest);
    };
    let path = Path::new(root);
    if !path.is_absolute()
        || root.chars().any(char::is_control)
        || root
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err("production root must be an absolute path with only normal components");
    }
    Ok(root)
}

/// Immutable production identity; runtime environment and CWD do not participate.
pub fn production_root() -> &'static Path {
    Path::new(
        option_env!("STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT").unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
}

/// Test namespaces always belong to the build checkout, even in a production build.
pub fn root_for_mode(test_mode: bool) -> &'static Path {
    if test_mode {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    } else {
        production_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_root_selection_preserves_default_and_rejects_invalid_override() {
        assert_eq!(
            select_build_root("/build/worktree", None),
            Ok("/build/worktree")
        );
        assert_eq!(
            select_build_root("/build/worktree", Some("/production/project")),
            Ok("/production/project")
        );
        for invalid in [
            "",
            "relative/path",
            "/",
            "/production/../project",
            "/production/./project",
            "/production//project",
            "/production/project/",
            "/production/\nproject",
            "/production/\0project",
        ] {
            assert!(
                select_build_root("/build/worktree", Some(invalid)).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn test_namespaces_stay_in_build_checkout() {
        assert_eq!(root_for_mode(true), Path::new(env!("CARGO_MANIFEST_DIR")));
        let expected = option_env!("STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT")
            .unwrap_or(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(root_for_mode(false), Path::new(expected));
        assert_ne!(
            root_for_mode(true).join("data/locks/test/monitor-delivery.lock"),
            root_for_mode(false).join("data/locks/production/monitor-delivery.lock"),
        );
    }
}
