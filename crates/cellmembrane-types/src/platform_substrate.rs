// SPDX-License-Identifier: AGPL-3.0-or-later

//! G68 Platform Substrate Abstraction — semantic filesystem access.
//!
//! Replaces raw `PermissionsExt::set_mode()` calls with platform-agnostic
//! operations that express *intent* rather than *mechanism*.  On Unix the
//! intent maps to mode bits; on Windows it maps to inheriting parent ACLs
//! with an optional read-only flag.
//!
//! # Pattern
//!
//! ```ignore
//! // Before (silicon deism):
//! #[cfg(unix)]
//! {
//!     use std::os::unix::fs::PermissionsExt;
//!     std::fs::set_permissions(path, Permissions::from_mode(0o755));
//! }
//!
//! // After (G68):
//! cellmembrane_types::PlatformAccess::Executable.apply(path)?;
//! ```

use std::io;
use std::path::Path;

/// Semantic filesystem access level (G68 L2 abstraction).
///
/// Each variant expresses an *intent* — "this file should be executable",
/// "this file contains secrets" — which maps to the correct platform
/// mechanism at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformAccess {
    /// Owner read/write/execute, group+other read/execute (0o755 on Unix).
    ///
    /// Use for: directories, binary executables, shell scripts.
    Executable,

    /// Owner read/write only (0o600 on Unix).
    ///
    /// Use for: private keys, credentials, gate identity files.
    Restricted,

    /// Owner read/write, group read/write (0o660 on Unix).
    ///
    /// Use for: IPC sockets shared within a service group.
    GroupReadWrite,
}

impl PlatformAccess {
    /// Apply this access level to the given path.
    ///
    /// On Unix, sets POSIX mode bits.  On Windows, sets the read-only
    /// attribute for `Restricted` (best-effort ACL equivalent) and
    /// clears it otherwise.
    pub fn apply(self, path: &Path) -> io::Result<()> {
        self.apply_inner(path)
    }

    #[cfg(unix)]
    fn apply_inner(self, path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mode = self.unix_mode();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }

    #[cfg(not(unix))]
    fn apply_inner(self, path: &Path) -> io::Result<()> {
        let meta = std::fs::metadata(path)?;
        let mut perms = meta.permissions();
        perms.set_readonly(self == Self::Restricted);
        std::fs::set_permissions(path, perms)
    }

    #[cfg(unix)]
    const fn unix_mode(self) -> u32 {
        match self {
            Self::Executable => 0o755,
            Self::Restricted => 0o600,
            Self::GroupReadWrite => 0o660,
        }
    }
}

impl std::fmt::Display for PlatformAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Executable => write!(f, "executable (755)"),
            Self::Restricted => write!(f, "restricted (600)"),
            Self::GroupReadWrite => write!(f, "group-rw (660)"),
        }
    }
}

/// Create a filesystem link (G68 L1 abstraction).
///
/// On Unix, creates a symbolic link.  On Windows, creates a hard link
/// (which works without elevated privileges, unlike symlinks).
///
/// cellMembrane currently has zero L1 sites — this is provided for
/// ecosystem consistency with the sourDough reference pattern.
pub fn platform_link(original: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(not(unix))]
    {
        std::fs::hard_link(original, link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_format() {
        assert_eq!(PlatformAccess::Executable.to_string(), "executable (755)");
        assert_eq!(PlatformAccess::Restricted.to_string(), "restricted (600)");
        assert_eq!(PlatformAccess::GroupReadWrite.to_string(), "group-rw (660)");
    }

    #[test]
    fn equality() {
        assert_eq!(PlatformAccess::Executable, PlatformAccess::Executable);
        assert_ne!(PlatformAccess::Executable, PlatformAccess::Restricted);
    }

    #[test]
    fn apply_to_existing_file() {
        let dir = std::env::temp_dir().join("g68_test_apply");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test_perms");
        std::fs::write(&file, b"test").unwrap();

        PlatformAccess::Executable.apply(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }

        PlatformAccess::Restricted.apply(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        #[cfg(not(unix))]
        {
            assert!(std::fs::metadata(&file).unwrap().permissions().readonly());
        }

        PlatformAccess::GroupReadWrite.apply(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o660);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_to_directory() {
        let dir = std::env::temp_dir().join("g68_test_dir_perms");
        let _ = std::fs::create_dir_all(&dir);

        PlatformAccess::Executable.apply(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_nonexistent_returns_error() {
        let result = PlatformAccess::Executable.apply(Path::new("/nonexistent/g68/path"));
        assert!(result.is_err());
    }

    #[test]
    fn platform_link_to_existing_file() {
        let dir = std::env::temp_dir().join("g68_test_link");
        let _ = std::fs::create_dir_all(&dir);
        let original = dir.join("original");
        std::fs::write(&original, b"link test").unwrap();
        let link = dir.join("link_target");
        let _ = std::fs::remove_file(&link);

        platform_link(&original, &link).unwrap();
        assert!(link.exists());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "link test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_mode_values() {
        assert_eq!(PlatformAccess::Executable.unix_mode(), 0o755);
        assert_eq!(PlatformAccess::Restricted.unix_mode(), 0o600);
        assert_eq!(PlatformAccess::GroupReadWrite.unix_mode(), 0o660);
    }
}
