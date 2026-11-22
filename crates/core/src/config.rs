use crate::models::*;
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::Path,
};

pub(crate) const LIMIT: usize = 8 * 1024 * 1024;
#[cfg(test)]
thread_local! {
    pub(crate) static WRITE_FAILURE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn write_fault(stage: &str) -> Result<()> {
    WRITE_FAILURE.with(|fault| {
        if fault.get() == Some(stage) {
            fault.set(None);
            anyhow::bail!("injected write failure");
        }
        Ok(())
    })
}
pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= LIMIT as u64,
        "select a regular file within the size limit"
    );
    let mut bytes = vec![];
    file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "file exceeds size limit");
    Ok(bytes)
}
pub(crate) fn write(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().context("missing parent directory")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    #[cfg(test)]
    write_fault("publish")?;
    if replace {
        file.persist(path).map_err(|e| e.error)?;
    } else {
        file.persist_noclobber(path).map_err(|e| e.error)?;
    }
    #[cfg(test)]
    write_fault("directory_sync")?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub(crate) fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub(crate) fn text(s: &str) -> bool {
    s.len() <= 8192 && !s.chars().any(|c| c.is_control())
}
pub(crate) fn snapshot(s: &str) -> Result<()> {
    ensure!(
        (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid snapshot ID"
    );
    Ok(())
}
pub(crate) fn absolute(s: &str) -> Result<()> {
    ensure!(
        text(s) && Path::new(s).is_absolute(),
        "an absolute path is required"
    );
    ensure!(
        !Path::new(s)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "parent traversal is not allowed"
    );
    Ok(())
}
fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        ensure!(valid_id(id) && seen.insert(id), "invalid or duplicate ID");
    }
    Ok(())
}
pub(crate) fn validate(c: &Config) -> Result<()> {
    ensure!(
        serde_json::to_vec(c)?.len() <= 1024 * 1024,
        "configuration too large"
    );
    absolute(&c.restic_path)?;
    ensure!(
        c.repositories.len() <= 100 && c.profiles.len() <= 100,
        "too many configuration entries"
    );
    unique(c.repositories.iter().map(|v| v.id.as_str()))?;
    unique(c.profiles.iter().map(|v| v.id.as_str()))?;
    for r in &c.repositories {
        ensure!(
            !r.location.is_empty() && text(&r.location) && text(&r.label),
            "invalid repository"
        );
        // URL userinfo passwords, query credentials and control characters never enter configuration.
        ensure!(
            !r.location.contains('?') && !r.location.contains('#'),
            "repository query credentials are not supported"
        );
        if let Some((_, tail)) = r.location.split_once("://") {
            let authority = tail.split('/').next().unwrap_or("");
            if let Some((userinfo, _)) = authority.rsplit_once('@') {
                ensure!(
                    !userinfo.contains([':', '%']),
                    "embedded URL credentials are not allowed"
                );
            }
        }
        if let Some(tail) = r.location.strip_prefix("sftp:")
            && !tail.starts_with("//")
            && let Some((userinfo, _)) = tail.split('/').next().unwrap_or("").split_once('@')
        {
            // SCP syntax has no password field: the first colon starts its
            // path, and restic also supports email-like usernames. Preserve
            // the legacy first-@ restriction rather than treating path text
            // after the host as URL credentials. Do not rewrite bound locators.
            ensure!(
                !userinfo.contains([':', '%']),
                "embedded SFTP credentials are not allowed"
            );
        }
        if let Some(path) = local_repository(&r.location) {
            absolute(path.to_str().context("invalid local repository path")?)?;
        } else {
            ensure!(
                r.location.contains(':'),
                "local repository paths must be absolute"
            );
        }
    }
    for p in &c.profiles {
        ensure!(
            text(&p.label)
                && !p.sources.is_empty()
                && p.sources.len() <= 256
                && p.excludes.len() <= 256,
            "invalid profile sources or exclusions"
        );
        for s in &p.sources {
            absolute(s)?;
        }
        ensure!(p.excludes.iter().all(|s| text(s)), "invalid exclusion");
        ensure!(
            !p.repository_ids.is_empty(),
            "profile requires a repository"
        );
        unique(p.repository_ids.iter().map(String::as_str))?;
        ensure!(
            p.repository_ids
                .iter()
                .all(|id| c.repositories.iter().any(|r| &r.id == id)),
            "unknown profile repository"
        );
        ensure!(
            p.retention.keep_last >= 1,
            "retention must keep at least the last snapshot"
        );
        ensure!(p.schedule_minutes <= 525600, "schedule interval too large");
    }
    Ok(())
}
pub(crate) fn local_repository(location: &str) -> Option<&Path> {
    if let Some(path) = location.strip_prefix("local:") {
        Some(Path::new(path))
    } else if Path::new(location).is_absolute() || !location.contains(':') {
        Some(Path::new(location))
    } else {
        None
    }
}
pub(crate) fn resolved(path: &Path) -> Result<std::path::PathBuf> {
    // A new repository or restore directory can have missing components.
    // Resolve its existing ancestor first. Errors other than a missing path
    // must reach the overlap check instead of being treated as missing.
    for ancestor in path.ancestors() {
        match fs::canonicalize(ancestor) {
            Ok(base) if ancestor == path => return Ok(base),
            Ok(base) => return Ok(base.join(path.strip_prefix(ancestor)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("path has no existing ancestor")
}
pub(crate) fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
pub(crate) fn validate_overlap(c: &Config, data: &Path) -> Result<()> {
    let data = resolved(data)?;
    let repositories = c
        .repositories
        .iter()
        .filter_map(|r| local_repository(&r.location))
        .map(resolved)
        .collect::<Result<Vec<_>>>()?;
    for (index, repo) in repositories.iter().enumerate() {
        ensure!(
            !overlaps(repo, &data),
            "local repository and app data directories must not overlap"
        );
        ensure!(
            !repositories[..index]
                .iter()
                .any(|other| overlaps(repo, other)),
            "local repositories must not overlap"
        );
    }
    for source in c.profiles.iter().flat_map(|p| &p.sources) {
        let source = resolved(Path::new(source))?;
        ensure!(
            !overlaps(&source, &data) && !repositories.iter().any(|r| overlaps(&source, r)),
            "backup sources must not overlap repository or app data directories; select separate source directories"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_missing_components_but_report_invalid_ancestors() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let base = dir.path().canonicalize()?;
        let missing = dir.path().join("new/repository");
        assert_eq!(resolved(&missing)?, base.join("new/repository"));
        let regular = dir.path().join("document");
        fs::write(&regular, [])?;
        assert!(resolved(&regular)?.is_file());
        assert!(resolved(&regular.join("repository")).is_err());
        Ok(())
    }
}
