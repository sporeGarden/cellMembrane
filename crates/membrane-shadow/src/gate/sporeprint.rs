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

use super::systemd_units::{generate_songbird_unit, GatewayUnitParams};

/// Parameters for sporePrint-specific NUCLEUS deployment (4 primals).
pub struct SporePrintDeployParams<'a> {
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

/// Generate the petalTongue content-serving systemd unit.
///
/// petalTongue renders sporePrint content (Zola pages, SceneGraph→SVG viz)
/// and listens on loopback, behind bearDog TLS termination.
#[must_use]
fn generate_petaltongue_unit(params: &SporePrintDeployParams<'_>) -> String {
    let bind = cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND;
    let content_root = format!(
        "{}/{}",
        params.ecoprimals_root,
        cellmembrane_types::service::SPOREPRINT_CONTENT_DIR,
    );

    format!(
        "[Unit]\n\
         Description=petalTongue content server ({gate} — sporePrint)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/petaltongue server --bind {bind} --content-dir {content_root}\n\
         Environment=GATE_NAME={gate}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         WorkingDirectory={content_root}\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
    )
}

/// Generate the nestGate CAS storage systemd unit.
///
/// nestGate provides content-addressed storage with BLAKE3 integrity.
/// Binds to UDS socket for local IPC only (petalTongue accesses via socket).
#[must_use]
fn generate_nestgate_unit(params: &SporePrintDeployParams<'_>) -> String {
    let socket_base = cellmembrane_types::service::DEFAULT_SOCKET_BASE;
    let socket = format!("{socket_base}/nestgate.sock");

    format!(
        "[Unit]\n\
         Description=nestGate CAS storage ({gate} — sporePrint)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         UMask={umask}\n\
         ExecStart={base}/nestgate server --socket {socket}\n\
         Environment=GATE_NAME={gate}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         RuntimeDirectory=membrane\n\
         RuntimeDirectoryMode={rtd_mode}\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK,
        rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE,
    )
}

/// Generate bearDog with ACME for a specific domain (sporePrint serving).
///
/// Unlike the generic gateway bearDog (which proxies to songBird socket),
/// the sporePrint bearDog proxies to petalTongue on loopback:8080.
#[must_use]
fn generate_beardog_acme_unit(params: &SporePrintDeployParams<'_>) -> String {
    let upstream = cellmembrane_types::service::DEFAULT_PETALTONGUE_BIND;

    format!(
        "[Unit]\n\
         Description=bearDog ACME gateway ({gate} — {domain})\n\
         After=network-online.target petaltongue-sporeprint.service\n\
         Wants=network-online.target\n\
         Requires=petaltongue-sporeprint.service\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/beardog serve-https \
         --upstream {upstream} \
         --domain {domain} \
         --acme-email {email}\n\
         Environment=GATE_NAME={gate}\n\
         Restart=always\n\
         RestartSec=5\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        domain = params.acme_domain,
        email = cellmembrane_types::service::DEFAULT_ACME_EMAIL,
    )
}

/// Generate all 4 sporePrint NUCLEUS systemd units.
///
/// Returns a `SporePrintUnits` struct with all unit file contents.
/// Uses the existing `generate_songbird_unit` for songBird and custom
/// ACME-aware bearDog for domain-specific TLS termination.
#[must_use]
pub fn generate_sporeprint_units(params: &SporePrintDeployParams<'_>) -> SporePrintUnits {
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
pub struct SporePrintUnits {
    pub petaltongue: String,
    pub nestgate: String,
    pub songbird: String,
    pub beardog: String,
}

impl SporePrintUnits {
    /// Unit filenames for systemd installation.
    #[must_use]
    pub const fn filenames() -> [&'static str; 4] {
        [
            "petaltongue-sporeprint.service",
            "nestgate-sporeprint.service",
            "songbird-gateway.service",
            "beardog-sporeprint.service",
        ]
    }

    /// Iterate over `(filename, content)` pairs in boot order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        let names = Self::filenames();
        [
            (names[0], self.petaltongue.as_str()),
            (names[1], self.nestgate.as_str()),
            (names[2], self.songbird.as_str()),
            (names[3], self.beardog.as_str()),
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
        assert!(unit.contains("--socket"));
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
