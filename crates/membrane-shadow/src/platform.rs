// SPDX-License-Identifier: AGPL-3.0-or-later

//! G68 async helpers for [`cellmembrane_types::PlatformAccess`].
//!
//! The sync `PlatformAccess::apply()` lives in `cellmembrane-types` (no async
//! runtime). This module adds the tokio-backed async variant used by
//! `membrane-shadow` build and fetch pipelines.

use std::io;
use std::path::Path;

use cellmembrane_types::PlatformAccess;

/// Async version of [`PlatformAccess::apply`] using tokio.
pub(crate) async fn apply_access_async(access: PlatformAccess, path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = match access {
            PlatformAccess::Executable => 0o755,
            PlatformAccess::Restricted => 0o600,
            PlatformAccess::GroupReadWrite => 0o660,
        };
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await
    }
    #[cfg(not(unix))]
    {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || access.apply(&path))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    }
}
