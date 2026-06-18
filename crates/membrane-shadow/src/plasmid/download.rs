// SPDX-License-Identifier: AGPL-3.0-or-later

//! Download transport for plasmid fetch — SSH, HTTP, and atomic write.

use std::path::Path;

use super::fetch::FetchSource;

pub(super) async fn download_asset(
    source: FetchSource,
    config: &crate::ShadowConfig,
    tag: &str,
    asset: &str,
    arch: &str,
    primal: &str,
    dest: &Path,
) -> bool {
    match source {
        FetchSource::GitHub => {
            let org = std::env::var(cellmembrane_types::service::ENV_GITHUB_ORG)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_GITHUB_ORG.into());
            let url =
                format!("https://github.com/{org}/plasmidBin/releases/download/{tag}/{asset}");
            download_via_http(&url, dest).await
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let org = std::env::var(cellmembrane_types::service::ENV_FORGEJO_ORG)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_ORG.into());
            let url = format!("{base}/{org}/plasmidBin/releases/download/{tag}/{asset}");
            download_via_http(&url, dest).await
        }
        FetchSource::Vps => {
            let vps_bin_dir = std::env::var(cellmembrane_types::service::ENV_VPS_BIN_DIR)
                .unwrap_or_else(|_| {
                    format!(
                        "{}/plasmidBin/primals",
                        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
                    )
                });
            let remote_path = format!("{vps_bin_dir}/{arch}/{primal}");
            download_via_ssh(&config.ssh_host, &remote_path, dest).await
        }
        FetchSource::Wan => {
            let base_url = std::env::var(cellmembrane_types::service::ENV_WAN_DEPOT_URL)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_WAN_DEPOT_URL.to_string());
            let url = format!("{base_url}/{arch}/{primal}");
            download_via_http(&url, dest).await
        }
    }
}

async fn download_via_ssh(host: &str, remote_path: &str, dest: &Path) -> bool {
    let output = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=30",
            "-o",
            "BatchMode=yes",
            host,
            "cat",
            remote_path,
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => atomic_write(dest, &o.stdout).await,
        _ => false,
    }
}

#[cfg(feature = "http")]
async fn download_via_http(url: &str, dest: &Path) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            cellmembrane_types::service::DEFAULT_FETCH_TIMEOUT_SECS,
        ))
        .build()
    else {
        return false;
    };

    let response = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return false,
    };

    let Ok(bytes) = response.bytes().await else {
        return false;
    };

    atomic_write(dest, &bytes).await
}

#[cfg(not(feature = "http"))]
async fn download_via_http(_url: &str, _dest: &Path) -> bool {
    false
}

/// Remove leftover `.tmp` files from interrupted downloads.
pub(super) async fn cleanup_partial_downloads(bin_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(bin_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

/// Atomic write: data -> temp file -> rename to final path.
/// Prevents partial/corrupt binaries from appearing at `dest` if the process
/// is interrupted mid-write. Cleans up the temp file on failure.
pub(super) async fn atomic_write(dest: &Path, data: &[u8]) -> bool {
    let tmp = dest.with_extension("tmp");
    if tokio::fs::write(&tmp, data).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return false;
    }
    if tokio::fs::rename(&tmp, dest).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return false;
    }
    true
}
