pub(crate) const SOURCE_REPOSITORY: &str =
    "git+https://github.com/Northofqing/magic-market-data-rs.git";

pub(crate) fn exact_locked_magic_tdx_revision(source: &str) -> Result<&str, String> {
    let (repository_and_query, resolved_revision) = source
        .rsplit_once('#')
        .ok_or_else(|| format!("locked Git source has no resolved commit: {source}"))?;
    let (repository, query) = repository_and_query
        .split_once('?')
        .ok_or_else(|| format!("locked Git source has no exact revision query: {source}"))?;
    if repository != SOURCE_REPOSITORY {
        return Err(format!("locked Git repository is not admitted: {source}"));
    }

    let requested_revision = query
        .strip_prefix("rev=")
        .filter(|revision| is_lower_hex_commit(revision))
        .ok_or_else(|| format!("locked Git query is not one exact full revision: {source}"))?;
    if !is_lower_hex_commit(resolved_revision) {
        return Err(format!(
            "resolved commit is not 40 lowercase hexadecimal characters: {source}"
        ));
    }
    if requested_revision != resolved_revision {
        return Err(format!(
            "requested and resolved Git revisions differ: {source}"
        ));
    }
    Ok(resolved_revision)
}

fn is_lower_hex_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
