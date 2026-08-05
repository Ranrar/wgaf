//! Shared validation for the daemon's two on-disk configuration files.
//!
//! Both `config.toml` and `permissions.toml` are **required** and must be
//! trustworthy before the daemon will start. The rules are identical for
//! both, so they live here rather than being written twice with a chance of
//! drifting apart.
//!
//! **Why required rather than defaulted.** Treating a missing file as "use
//! the defaults" makes deletion silent and invisible. For `permissions.toml`
//! that is a security failure outright — the defaults are *allow everything*,
//! so removing the file removes every restriction the user configured, with
//! no error and no warning, and a file lost to a stray `rm` or a restored
//! home directory is indistinguishable from a deliberate decision. For
//! `config.toml` the stakes are lower but the reasoning is the same: the
//! daemon should run the configuration you can see, not a hidden fallback.
//! Wanting the defaults is still fine — an empty file says exactly that, and
//! says it visibly.
//!
//! **Why ownership and mode matter.** Whoever can write these files decides
//! what the daemon does: `permissions.toml` decides what it is allowed to do,
//! and `config.toml` decides which bus names it talks to — including
//! `extension_bus_name`, which a hostile rewrite could point at a service the
//! attacker controls. A file another account can edit is not configuration,
//! it is a suggestion. `ssh` refuses to use a private key on exactly these
//! grounds.

use std::path::Path;

/// Why a configuration file could not be trusted. Every variant is fatal at
/// startup — the daemon refuses to run rather than fall back to a default
/// nobody chose.
#[derive(Debug, thiserror::Error)]
pub enum SecureFileError {
    #[error(
        "no {kind} file found at `{path}`\n\n\
         Create it (`packaging/{template}` is a commented template you can copy):\n\n    \
         {remedy}"
    )]
    Missing {
        kind: &'static str,
        path: String,
        template: &'static str,
        remedy: String,
    },

    #[error("{kind} file `{path}` is not a regular file")]
    NotAFile { kind: &'static str, path: String },

    #[error(
        "{kind} file `{path}` is owned by uid {owner}, but this daemon runs as uid \
         {running_as}. Fix with: chown {running_as} {path}"
    )]
    WrongOwner {
        kind: &'static str,
        path: String,
        owner: u32,
        running_as: u32,
    },

    #[error(
        "{kind} file `{path}` is writable by group or others (mode {mode:04o}). \
         Fix with: chmod 600 {path}"
    )]
    TooPermissive {
        kind: &'static str,
        path: String,
        mode: u32,
    },

    #[error("{kind} file `{path}` could not be read: {reason}")]
    Unreadable {
        kind: &'static str,
        path: String,
        reason: String,
    },

    #[error("{kind} file `{path}` is not valid TOML: {reason}")]
    Malformed {
        kind: &'static str,
        path: String,
        reason: String,
    },

    /// Distinct from [`Self::Malformed`]: the file parses as valid TOML, but
    /// sets a value this daemon does not support (yet, or at all).
    #[error("{kind} file `{path}` sets an unsupported value: {reason}")]
    Unsupported {
        kind: &'static str,
        path: String,
        reason: String,
    },
}

/// The uid the daemon is running as. Wrapped so the ownership check reads
/// clearly and has one place to change if this ever needs to distinguish
/// real from effective uid.
fn running_uid() -> u32 {
    // SAFETY: `getuid` takes no arguments, dereferences nothing, and is
    // documented as always succeeding — it cannot fail or misbehave.
    unsafe { libc::getuid() }
}

/// Verifies `path` exists, is a regular file, belongs to this user (or root),
/// and is not group- or world-writable — then reads it.
///
/// Returns the file's contents so callers do a single traversal: checking and
/// then separately opening would leave a window in which the file could be
/// swapped between the two.
pub(crate) fn read_trusted(
    path: &Path,
    kind: &'static str,
    template: &'static str,
    remedy: String,
) -> Result<String, SecureFileError> {
    let metadata = std::fs::metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            SecureFileError::Missing {
                kind,
                path: path.display().to_string(),
                template,
                remedy,
            }
        } else {
            SecureFileError::Unreadable {
                kind,
                path: path.display().to_string(),
                reason: source.to_string(),
            }
        }
    })?;

    if !metadata.is_file() {
        return Err(SecureFileError::NotAFile {
            kind,
            path: path.display().to_string(),
        });
    }

    use std::os::unix::fs::MetadataExt;

    // The daemon runs as a systemd *user* service, so its configuration must
    // belong to that same user. Root is also accepted: an administrator-owned
    // file the user cannot edit is a stricter arrangement, not a weaker one.
    let owner = metadata.uid();
    let running_as = running_uid();
    if owner != running_as && owner != 0 {
        return Err(SecureFileError::WrongOwner {
            kind,
            path: path.display().to_string(),
            owner,
            running_as,
        });
    }

    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(SecureFileError::TooPermissive {
            kind,
            path: path.display().to_string(),
            mode,
        });
    }

    std::fs::read_to_string(path).map_err(|source| SecureFileError::Unreadable {
        kind,
        path: path.display().to_string(),
        reason: source.to_string(),
    })
}
