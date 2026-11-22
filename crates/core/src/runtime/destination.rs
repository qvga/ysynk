//! Destination checks for password export and restore.
use crate::{config, models::Config};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(super) fn password_export_target(c: &Config, data: &Path, target: &Path) -> Result<PathBuf> {
    fn validate(c: &Config, data: &Path, target: &Path) -> Result<PathBuf> {
        config::absolute(target.to_str().context("invalid path")?)?;
        let parent = fs::canonicalize(target.parent().context("missing parent")?)?;
        ensure!(parent.is_dir(), "parent is not a directory");
        let target = parent.join(target.file_name().context("missing file name")?);
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("target already exists or is inaccessible"),
        }
        let mut protected = vec![config::resolved(data)?];
        for repo in c
            .repositories
            .iter()
            .filter_map(|r| config::local_repository(&r.location))
        {
            protected.push(config::resolved(repo)?);
        }
        for source in c.profiles.iter().flat_map(|p| &p.sources) {
            protected.push(config::resolved(Path::new(source))?);
        }
        ensure!(
            !protected.iter().any(|path| config::overlaps(path, &target)),
            "protected export location"
        );
        Ok(target)
    }
    validate(c, data, target).map_err(|_| anyhow::anyhow!("Choose a new password file in an existing private folder outside app data, repositories, and backup sources; existing files and symlinks are never overwritten"))
}
pub(super) fn sync_export_parent(parent: &File) -> Result<()> {
    #[cfg(test)]
    config::write_fault("export_directory_sync")?;
    parent.sync_all()?;
    Ok(())
}
#[cfg(target_os = "linux")]
pub(super) fn trusted_parent(target: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };
    ensure!(
        target.is_absolute(),
        "an absolute destination path is required"
    );
    let mut parent = File::open("/")?;
    for component in target
        .parent()
        .context("missing parent")?
        .components()
        .skip(1)
    {
        ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "invalid parent component"
        );
        let name = CString::new(component.as_os_str().as_bytes())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "parent must exist without symlinks");
        parent = unsafe { File::from_raw_fd(fd) };
    }
    let metadata = parent.metadata()?;
    let trusted_owner = metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() };
    // A sticky shared directory (such as /tmp) protects the entry we create.
    // Same-user processes and root are within the trust boundary.
    ensure!(
        trusted_owner && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
        "destination parent must be owned by this user or root and not writable by others unless sticky"
    );
    Ok(parent)
}
#[cfg(target_os = "linux")]
pub(super) fn pinned_export_target(parent: &File, target: &Path) -> Result<PathBuf> {
    use std::os::fd::AsRawFd;
    Ok(
        PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
            .join(target.file_name().context("missing file name")?),
    )
}
#[cfg(not(target_os = "linux"))]
pub(super) fn trusted_parent(_: &Path) -> Result<File> {
    bail!("safe descriptor-based password export currently requires Linux")
}
#[cfg(not(target_os = "linux"))]
pub(super) fn pinned_export_target(_: &File, _: &Path) -> Result<PathBuf> {
    bail!("safe descriptor-based password export currently requires Linux")
}
#[cfg(target_os = "linux")]
pub(super) fn private_restore_directory(target: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let name = target
        .file_name()
        .context("restore target must be a new directory")?;
    let name = CString::new(name.as_bytes())?;
    let parent = trusted_parent(target)?;
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    ensure!(
        created == 0,
        "restore output must not already exist, and parent must be writable"
    );
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(
        fd >= 0,
        "could not open the newly created restore directory"
    );
    // Restic must see a real directory as the final component, not a /proc symlink.
    drop(unsafe { File::from_raw_fd(fd) });
    Ok(parent)
}
#[cfg(not(target_os = "linux"))]
pub(super) fn private_restore_directory(_: &Path) -> Result<File> {
    bail!("safe descriptor-based restore currently requires Linux")
}
