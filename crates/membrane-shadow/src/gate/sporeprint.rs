// SPDX-License-Identifier: AGPL-3.0-or-later

//! sporePrint NUCLEUS deployment — systemd units for the 4-primal content
//! serving composition (petalTongue + nestGate + songBird + bearDog).
//!
//! This is a minimal NUCLEUS tier optimized for sovereign website hosting
//! on VPS gates like golgi. The composition serves `primals.eco` via:
//! - petalTongue: content rendering (Zola pages, SceneGraph→SVG viz)
//! - nestGate: CAS storage (content-addressed, BLAKE3 integrity)
//! - songBird: mesh routing (live topology, capability.call)
//! - bearDog: TLS termination (ACME cert for the domain)

use super::systemd_units::{GatewayUnitParams, generate_songbird_unit};

/// sporePrint composition uses specific primals in fixed roles.
///
/// Unlike NUCLEUS (which is capability-discovered), sporePrint is a named
/// deployment pattern: content rendering + CAS + relay + TLS. These role
/// assignments are resolved from the service registry to avoid raw string
/// literals while maintaining the architectural coupling.
fn sporeprint_binaries() -> SporePrintBinaries {
    SporePrintBinaries {
        content: cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::ServiceCapability::Visualization,
        ),
        cas: cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::ServiceCapability::ContentAddressedStorage,
        ),
        crypto: cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::ServiceCapability::CryptoSigner,
        ),
    }
}

struct SporePrintBinaries {
    content: &'static str,
    cas: &'static str,
    crypto: &'static str,
}

/// Parameters for sporePrint-specific NUCLEUS deployment (4 primals).
pub(crate) struct SporePrintDeployParams<'a> {
    pub gate_name: &'a str,
    pub install_base: &'a str,
    pub ecoprimals_root: &'a str,
    pub acme_domain: &'a str,
    pub songbird_socket: &'a str,
}

impl<'a> SporePrintDeployParams<'a> {
    /// Create params with defaults, requiring only the gate name and ACME domain.
    #[must_use]
    pub const fn new(gate_name: &'a str, acme_domain: &'a str) -> Self {
        Self {
            gate_name,
            install_base: cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            ecoprimals_root: cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
            acme_domain,
            songbird_socket: cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
        }
    }
}

/// Generate the content-serving systemd unit for sporePrint.
///
/// The content server renders sporePrint content (Zola pages, `SceneGraph`
/// visualization) and listens on loopback, behind TLS termination.
#[must_use]
fn generate_petaltongue_unit(params: &SporePrintDeployParams<'_>) -> String {
    let bind = cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND;
    let content_root = format!(
        "{}/{}",
        params.ecoprimals_root,
        cellmembrane_types::service::SPOREPRINT_CONTENT_DIR,
    );

    let roles = sporeprint_binaries();

    format!(
        "[Unit]\n\
         Description={content} content server ({gate} — sporePrint)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/{content} server --bind {bind} --content-dir {content_root}\n\
         Environment=MEMBRANE_GATE_NAME={gate}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         WorkingDirectory={content_root}\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        content = roles.content,
    )
}

/// Generate the CAS storage systemd unit for sporePrint.
///
/// The storage primal provides content-addressed storage with BLAKE3
/// integrity. Binds to UDS socket for local IPC only.
#[must_use]
fn generate_nestgate_unit(params: &SporePrintDeployParams<'_>) -> String {
    let socket_base = cellmembrane_types::service::DEFAULT_SOCKET_BASE;
    let roles = sporeprint_binaries();

    format!(
        "[Unit]\n\
         Description={cas} CAS storage ({gate} — sporePrint)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         UMask={umask}\n\
         ExecStart={base}/{cas} server\n\
         Environment=MEMBRANE_GATE_NAME={gate}\n\
         Environment=NESTGATE_SOCKET={socket_base}/{cas}.sock\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         RuntimeDirectory={rtd}\n\
         RuntimeDirectoryMode={rtd_mode}\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        cas = roles.cas,
        umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK,
        rtd = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY,
        rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE,
    )
}

/// Generate the crypto-signer ACME unit for sporePrint domain serving.
///
/// Unlike the generic gateway (which proxies to the mesh relay socket),
/// the sporePrint ACME unit proxies to the content server on loopback.
#[must_use]
fn generate_beardog_acme_unit(params: &SporePrintDeployParams<'_>) -> String {
    let upstream = cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND;
    let roles = sporeprint_binaries();
    let content_unit = format!("{}-sporeprint.service", roles.content);

    format!(
        "[Unit]\n\
         Description={crypto} ACME gateway ({gate} — {domain})\n\
         After=network-online.target {content_unit}\n\
         Wants=network-online.target\n\
         Requires={content_unit}\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/{crypto} serve-https \
         --upstream {upstream} \
         --domain {domain} \
         --acme-email {email}\n\
         Environment=MEMBRANE_GATE_NAME={gate}\n\
         Restart=on-failure\n\
         RestartSec={restart_delay}\n\
         StartLimitIntervalSec={start_limit_interval}\n\
         StartLimitBurst={start_limit_burst}\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        crypto = roles.crypto,
        domain = params.acme_domain,
        email = cellmembrane_types::service::DEFAULT_ACME_EMAIL,
        restart_delay = cellmembrane_types::service::DEFAULT_RESTART_DELAY_SECS,
        start_limit_interval = cellmembrane_types::service::DEFAULT_START_LIMIT_INTERVAL_SECS,
        start_limit_burst = cellmembrane_types::service::DEFAULT_START_LIMIT_BURST,
    )
}

/// Generate all 4 sporePrint NUCLEUS systemd units.
///
/// Returns a `SporePrintUnits` struct with all unit file contents.
/// Uses the existing `generate_songbird_unit` for songBird and custom
/// ACME-aware bearDog for domain-specific TLS termination.
#[must_use]
pub(crate) fn generate_sporeprint_units(params: &SporePrintDeployParams<'_>) -> SporePrintUnits {
    let gw_params = GatewayUnitParams {
        gate_name: params.gate_name,
        install_base: params.install_base,
        songbird_socket: params.songbird_socket,
        gateway_bind: cellmembrane_types::service::DEFAULT_GATEWAY_BIND,
        proxy_routes: "",
    };

    SporePrintUnits {
        petaltongue: generate_petaltongue_unit(params),
        nestgate: generate_nestgate_unit(params),
        songbird: generate_songbird_unit(&gw_params),
        beardog: generate_beardog_acme_unit(params),
    }
}

/// All 4 systemd unit file contents for a sporePrint NUCLEUS composition.
pub(crate) struct SporePrintUnits {
    pub petaltongue: String,
    pub nestgate: String,
    pub songbird: String,
    pub beardog: String,
}

impl SporePrintUnits {
    /// Unit filenames for systemd installation, derived from the service registry.
    #[must_use]
    pub fn filenames() -> [String; 4] {
        let roles = sporeprint_binaries();
        [
            format!("{}-sporeprint.service", roles.content),
            format!("{}-sporeprint.service", roles.cas),
            format!(
                "{}-gateway.service",
                cellmembrane_types::MembraneService::binary_for(
                    cellmembrane_types::ServiceCapability::MeshRelay
                )
            ),
            format!("{}-sporeprint.service", roles.crypto),
        ]
    }

    /// Iterate over `(filename, content)` pairs in boot order.
    pub fn iter(&self) -> impl Iterator<Item = (String, &str)> {
        let names = Self::filenames();
        [
            (names[0].clone(), self.petaltongue.as_str()),
            (names[1].clone(), self.nestgate.as_str()),
            (names[2].clone(), self.songbird.as_str()),
            (names[3].clone(), self.beardog.as_str()),
        ]
        .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn petaltongue_unit_has_systemd_sections() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let unit = generate_petaltongue_unit(&params);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("golgiBody"));
        assert!(unit.contains("petaltongue server"));
        assert!(unit.contains("--content-dir"));
        assert!(unit.contains("sporePrint"));
    }

    #[test]
    fn petaltongue_unit_binds_loopback() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let unit = generate_petaltongue_unit(&params);
        assert!(
            unit.contains(cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND),
            "petalTongue should bind loopback:8080"
        );
    }

    #[test]
    fn nestgate_unit_has_systemd_sections() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let unit = generate_nestgate_unit(&params);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("nestgate server"));
        assert!(
            !unit.contains("--socket"),
            "nestgate CLI no longer accepts --socket flag"
        );
        assert!(
            unit.contains("NESTGATE_SOCKET="),
            "socket path should be passed via env var"
        );
        assert!(unit.contains("nestgate.sock"));
    }

    #[test]
    fn beardog_acme_unit_includes_domain() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let unit = generate_beardog_acme_unit(&params);
        assert!(unit.contains("primals.eco"), "should include ACME domain");
        assert!(unit.contains("--domain"), "should have --domain flag");
        assert!(unit.contains("--acme-email"), "should have --acme-email");
        assert!(unit.contains("CAP_NET_BIND_SERVICE"));
    }

    #[test]
    fn beardog_acme_unit_upstreams_to_petaltongue() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let unit = generate_beardog_acme_unit(&params);
        assert!(
            unit.contains(cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND),
            "bearDog should upstream to petalTongue bind address"
        );
        assert!(
            unit.contains("Requires=petaltongue-sporeprint.service"),
            "bearDog should depend on petalTongue"
        );
    }

    #[test]
    fn sporeprint_units_generates_all_four() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let units = generate_sporeprint_units(&params);
        assert!(units.petaltongue.contains("petaltongue"));
        assert!(units.nestgate.contains("nestgate"));
        assert!(units.songbird.contains("songbird"));
        assert!(units.beardog.contains("beardog"));
    }

    #[test]
    fn sporeprint_units_filenames_correct() {
        let names = SporePrintUnits::filenames();
        assert_eq!(names.len(), 4);
        assert!(names[0].contains("petaltongue"));
        assert!(names[1].contains("nestgate"));
        assert!(names[2].contains("songbird"));
        assert!(names[3].contains("beardog"));
    }

    #[test]
    fn sporeprint_units_iter_pairs_match() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        let units = generate_sporeprint_units(&params);
        let pairs: Vec<_> = units.iter().collect();
        assert_eq!(pairs.len(), 4);
        assert!(pairs[0].0.contains("petaltongue"));
        assert!(pairs[0].1.contains("petaltongue"));
        assert!(pairs[3].0.contains("beardog"));
        assert!(pairs[3].1.contains("beardog"));
    }

    #[test]
    fn sporeprint_deploy_params_defaults() {
        let params = SporePrintDeployParams::new("golgiBody", "primals.eco");
        assert_eq!(params.gate_name, "golgiBody");
        assert_eq!(params.acme_domain, "primals.eco");
        assert_eq!(
            params.install_base,
            cellmembrane_types::service::DEFAULT_INSTALL_BASE
        );
        assert_eq!(
            params.ecoprimals_root,
            cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
        );
    }
}
