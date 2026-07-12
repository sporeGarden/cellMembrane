// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deployment constants — paths, environment variables, timeouts, and defaults.
//!
//! Extracted from the service module to keep the service registry itself focused
//! on service definitions and capabilities.

/// Bind to all interfaces (externally reachable).
pub const BIND_ALL: &str = "0.0.0.0";
/// Bind to loopback only (not externally reachable).
pub const BIND_LOOPBACK: &str = "127.0.0.1";

/// Default base path for primal binary installations.
/// Override with `MEMBRANE_INSTALL_BASE` env var or membrane.toml config.
pub const DEFAULT_INSTALL_BASE: &str = "/opt/membrane";

/// Default base path for primal UDS sockets.
pub const DEFAULT_SOCKET_BASE: &str = "/run/membrane";

/// Default configuration directory (system-wide config files).
pub const DEFAULT_CONFIG_DIR: &str = "/etc/membrane";

/// Default ecoPrimals workspace root on VPS deployments.
/// Override with `ECOPRIMALS_ROOT` env var.
pub const DEFAULT_ECOPRIMALS_ROOT: &str = "/opt/ecoPrimals";

/// Infra path fragment for the shared coordination layer (`wateringHole`).
pub const INFRA_WATERING_HOLE: &str = "infra/wateringHole";

/// Filename for the physical network topology map in the wateringHole.
pub const TOPOLOGY_MAP_FILENAME: &str = "TOPOLOGY_MAP.toml";

/// Infra path fragment for the binary depot (`plasmidBin`).
pub const INFRA_PLASMID_BIN: &str = "infra/plasmidBin";

/// Directory name for the binary depot.
pub const PLASMID_BIN_DIR: &str = "plasmidBin";

// ── Standard deployment environment variables ────────────────────────

/// Environment variable for the `plasmidBin` depot directory.
pub const ENV_PLASMIDBIN_DEPOT: &str = "PLASMIDBIN_DEPOT";
/// Legacy environment variable for gate-specific `plasmidBin` path.
pub const ENV_PLASMIDBIN_LEGACY: &str = "ECOPRIMALS_PLASMID_BIN";
/// Environment variable for the security provider socket path.
pub const ENV_SECURITY_PROVIDER: &str = "SONGBIRD_SECURITY_PROVIDER";
/// Environment variable for the membrane install base directory.
pub const ENV_INSTALL_BASE: &str = "MEMBRANE_INSTALL_BASE";
/// Environment variable for the membrane socket base directory.
pub const ENV_SOCKET_BASE: &str = "MEMBRANE_SOCKET_BASE";
/// Environment variable for the membrane configuration directory.
pub const ENV_CONFIG_DIR: &str = "MEMBRANE_CONFIG_DIR";
/// Environment variable for the Forgejo SSH host.
pub const ENV_FORGEJO_SSH_HOST: &str = "FORGEJO_SSH_HOST";
/// Environment variable for the ecoPrimals workspace root.
pub const ENV_ECOPRIMALS_ROOT: &str = "ECOPRIMALS_ROOT";
/// Environment variable for the gate identity.
pub const ENV_GATE_NAME: &str = "GATE_NAME";
/// Environment variable for the webhook secret (HMAC-SHA256).
pub const ENV_WEBHOOK_SECRET: &str = "WEBHOOK_SECRET";
/// Environment variable for the `NeuralBridge` API socket path.
pub const ENV_NEURAL_API_SOCKET: &str = "NEURAL_API_SOCKET";

/// Default socket filename for the biomeOS Neural API.
pub const NEURAL_API_SOCKET_NAME: &str = "neural-api-default.sock";

/// Namespace directory for biomeOS runtime sockets (under `XDG_RUNTIME_DIR` or /tmp).
pub const NEURAL_API_NAMESPACE: &str = "biomeos";
/// Environment variable for the peptidoglycan SSH host (legacy, prefer `ENV_VALIDATE_SSH_HOST`).
pub const ENV_PEPTI_SSH_HOST: &str = "PEPTI_SSH_HOST";
/// Environment variable for the gate.validate SSH target host.
pub const ENV_VALIDATE_SSH_HOST: &str = "MEMBRANE_VALIDATE_SSH_HOST";
/// Environment variable for the Forgejo API token.
pub const ENV_FORGEJO_TOKEN: &str = "FORGEJO_TOKEN";
/// Environment variable for the Forgejo API URL.
pub const ENV_FORGEJO_API: &str = "FORGEJO_API";
/// Environment variable for the membrane SSH host (golgiBody).
pub const ENV_SSH_HOST: &str = "MEMBRANE_SSH_HOST";
/// Environment variable for the VPS ecoPrimals root directory.
pub const ENV_VPS_ECOPRIMALS_ROOT: &str = "VPS_ECOPRIMALS_ROOT";
/// Environment variable for NUCLEUS bind address.
pub const ENV_NUCLEUS_BIND: &str = "NUCLEUS_BIND_ADDRESS";
/// Environment variable for the membrane SSH external host (golgiBody-ext).
pub const ENV_SSH_HOST_EXT: &str = "MEMBRANE_SSH_HOST_EXT";
/// Environment variable for the golgiBody external host (relay target).
pub const ENV_GOLGI_EXT_HOST: &str = "GOLGI_EXT_HOST";
/// Environment variable for the Cloudflare API token.
pub const ENV_CLOUDFLARE_TOKEN: &str = "CLOUDFLARE_API_TOKEN";
/// Environment variable for the Cloudflare zone ID.
pub const ENV_CLOUDFLARE_ZONE: &str = "CLOUDFLARE_ZONE_ID";
/// Environment variable for the relay Forgejo remote name.
pub const ENV_RELAY_FORGEJO_REMOTE: &str = "RELAY_FORGEJO_REMOTE";
/// Environment variable for the relay GitHub/origin remote name.
pub const ENV_RELAY_GITHUB_REMOTE: &str = "RELAY_GITHUB_REMOTE";
/// Environment variable for the `nestGate` content path.
pub const ENV_NESTGATE_CONTENT_PATH: &str = "NESTGATE_CONTENT_PATH";
/// Environment variable for the `nestGate` HTTP port.
pub const ENV_NESTGATE_PORT: &str = "NESTGATE_PORT";
/// Environment variable for the VPS membrane binary directory.
pub const ENV_VPS_BIN_DIR: &str = "VPS_MEMBRANE_BIN_DIR";
/// Environment variable for the songbird configuration path.
pub const ENV_SONGBIRD_CONFIG: &str = "SONGBIRD_CONFIG_PATH";
/// Default relay config directory (e.g. `/etc/songbird`).
/// Override with `SONGBIRD_CONFIG_PATH`.
pub const DEFAULT_RELAY_CONFIG_DIR: &str = "/etc/songbird";
/// Environment variable for SSH connection timeout (seconds).
pub const ENV_SSH_TIMEOUT: &str = "SSH_TIMEOUT";
/// Environment variable for the Forgejo data directory path.
pub const ENV_FORGEJO_DATA_DIR: &str = "FORGEJO_DATA_DIR";
/// Default Forgejo data directory.
pub const DEFAULT_FORGEJO_DATA_DIR: &str = "/opt/forgejo/data";
/// Default Forgejo install base.
pub const DEFAULT_FORGEJO_INSTALL_BASE: &str = "/opt/forgejo";
/// Default Caddy systemd service unit name.
pub const CADDY_SERVICE_UNIT: &str = "caddy-tls";
/// Default WAN interface name hint.
pub const DEFAULT_WAN_IFACE: &str = "enp1s0";
/// Default LAN interface name hint.
pub const DEFAULT_LAN_IFACE: &str = "eno1";
/// Default LAN subnet CIDR.
pub const DEFAULT_LAN_SUBNET: &str = "192.168.4.0/22";
/// Default `WireGuard` mesh subnet CIDR.
pub const DEFAULT_WG_MESH_SUBNET: &str = "10.13.37.0/24";
/// Systemd `RuntimeDirectoryMode` for primal services.
///
/// `0755` allows non-root processes to traverse `/run/membrane/` and
/// connect to primal UDS sockets. Combined with `DEFAULT_SERVICE_UMASK`.
pub const DEFAULT_RUNTIME_DIRECTORY_MODE: &str = "0755";

/// Systemd `UMask` for primal services.
///
/// `0002` causes socket files to be created as `srw-rw-r--` (0664) instead
/// of `srw-------` (0600), allowing non-root IPC clients (e.g. membrane CLI,
/// Neural API cross-primal routing) to connect.
pub const DEFAULT_SERVICE_UMASK: &str = "0002";

/// Default systemd unit directory.
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
/// Default secrets environment file path.
pub const DEFAULT_SECRETS_ENV: &str = "/etc/membrane/secrets.env";
/// Environment variable for the Forgejo work directory path.
pub const ENV_FORGEJO_WORK_DIR: &str = "FORGEJO_WORK_DIR";
/// Environment variable for the Forgejo admin username.
pub const ENV_FORGEJO_ADMIN_USER: &str = "FORGEJO_ADMIN_USER";
/// Environment variable for the membrane service filter (systemd unit prefix).
pub const ENV_SERVICE_FILTER: &str = "MEMBRANE_SERVICE_FILTER";
/// Environment variable for the WAN depot base URL (outer membrane HTTPS endpoint).
pub const ENV_WAN_DEPOT_URL: &str = "WAN_DEPOT_URL";
/// Environment variable for the SSH user on provisioned gates.
pub const ENV_PROVISION_SSH_USER: &str = "MEMBRANE_PROVISION_SSH_USER";
/// Default SSH user for gate provisioning (DigitalOcean/Hetzner default).
pub const DEFAULT_PROVISION_SSH_USER: &str = "root";
/// Environment variable to override the Caddy admin API endpoint.
pub const ENV_CADDY_ADMIN_ENDPOINT: &str = "CADDY_ADMIN_ENDPOINT";
/// Default Caddy admin API endpoint (Caddy convention: localhost-only control plane).
pub const DEFAULT_CADDY_ADMIN_ENDPOINT: &str = "localhost:2019";

/// Default WAN depot base URL served by Caddy on the sovereign membrane surface.
pub const DEFAULT_WAN_DEPOT_URL: &str = "https://membrane.primals.eco/depot";

/// Environment variable to override the sandbox socket directory.
pub const ENV_SANDBOX_SOCKET_DIR: &str = "MEMBRANE_SANDBOX_SOCKET_DIR";
/// Default sandbox socket directory (ephemeral UDS probes during validation).
pub const DEFAULT_SANDBOX_SOCKET_DIR: &str = "/run/membrane/sandbox";

/// Default cascade timer interval in minutes (golgi relay loop).
pub const DEFAULT_CASCADE_INTERVAL_MINUTES: u32 = 15;
/// Environment variable to override the sandbox binary directory.
pub const ENV_SANDBOX_BIN_DIR: &str = "MEMBRANE_SANDBOX_BIN_DIR";
/// Default sandbox binary directory (isolated copies for validation).
pub const DEFAULT_SANDBOX_BIN_DIR: &str = "/opt/membrane/sandbox";
/// Environment variable to override the canary socket directory.
pub const ENV_CANARY_SOCKET_DIR: &str = "MEMBRANE_CANARY_SOCKET_DIR";
/// Default canary socket directory (previous-good fallback instances).
pub const DEFAULT_CANARY_SOCKET_DIR: &str = "/run/membrane/canary";
/// Environment variable to override the canary binary directory.
pub const ENV_CANARY_BIN_DIR: &str = "MEMBRANE_CANARY_BIN_DIR";
/// Default canary binary directory (previous-good binaries retained for rollback).
pub const DEFAULT_CANARY_BIN_DIR: &str = "/opt/membrane/canary";
/// Environment variable to override canary maximum age in hours before staleness.
pub const ENV_CANARY_MAX_AGE_HOURS: &str = "MEMBRANE_CANARY_MAX_AGE_HOURS";

/// Environment variable for the `plasmidBin` staging directory.
pub const ENV_PLASMIDBIN_STAGING: &str = "PLASMIDBIN_STAGING";

/// Environment variable for the `biomeOS` socket directory.
pub const ENV_BIOMEOS_SOCKET_DIR: &str = "BIOMEOS_SOCKET_DIR";

/// Environment variable for the `NetworkManager` dispatcher directory.
pub const ENV_NM_DISPATCHER_DIR: &str = "NM_DISPATCHER_DIR";
/// Default `NetworkManager` dispatcher directory.
pub const DEFAULT_NM_DISPATCHER_DIR: &str = "/etc/NetworkManager/dispatcher.d";

/// Environment variable for the family seed (ribocipher key derivation).
pub const ENV_FAMILY_SEED: &str = "FAMILY_SEED";

/// Default VPS host (golgiBody sovereign surface).
///
/// Last-resort fallback only. Production code should resolve via
/// `manifest::resolve_federation_peer()` which checks: manifest roles →
/// `MEMBRANE_VPS_PEER` env var → this constant.
pub const DEFAULT_VPS_HOST: &str = "157.230.3.183";

/// Default SSH alias for golgiBody (internal name used in ~/.ssh/config).
pub const DEFAULT_SSH_ALIAS: &str = "golgi";
/// Default SSH alias for golgiBody external relay endpoint.
pub const DEFAULT_SSH_ALIAS_EXT: &str = "golgi-ext";

/// Default `NestGate` service port.
pub const DEFAULT_NESTGATE_PORT: u16 = 9500;

/// Default Forgejo web UI / HTTP API port.
pub const DEFAULT_FORGEJO_HTTP_PORT: u16 = 3000;

/// Default WAN depot file-server port (Caddy upstream for `/depot/`).
pub const DEFAULT_DEPOT_HTTP_PORT: u16 = 8080;

/// Default songbird federation port.
pub const DEFAULT_FEDERATION_PORT: u16 = 7700;

/// Default TURN relay port.
pub const DEFAULT_TURN_PORT: u16 = 3478;

/// `RustDesk` hbbs (ID/rendezvous server) port.
pub const RUSTDESK_HBBS_PORT: u16 = 21115;
/// `RustDesk` hbbr (relay server) port.
pub const RUSTDESK_HBBR_PORT: u16 = 21117;

/// Default VPS mesh peer address (hub songbird federation endpoint).
///
/// Last-resort fallback only. Production code should resolve via
/// `manifest::resolve_federation_peer()` which checks: manifest roles →
/// `MEMBRANE_VPS_PEER` env var → this constant.
pub const DEFAULT_VPS_MESH_PEER: &str = "157.230.3.183:7700";

/// Default mesh hub node identifier for peer addressing.
pub const DEFAULT_MESH_HUB_ID: &str = "hub";

/// Environment variable override for the VPS mesh peer address (host only).
pub const ENV_VPS_MESH_PEER: &str = "MEMBRANE_VPS_PEER";

/// Environment variable override for the mesh hub node identifier.
pub const ENV_MESH_HUB_ID: &str = "MEMBRANE_MESH_HUB_ID";

/// Environment variable for additional mesh peers (comma-separated `host:port`).
///
/// Used alongside `MEMBRANE_VPS_PEER` to bootstrap multi-peer mesh topologies.
/// Example: `MEMBRANE_MESH_PEERS=192.168.1.100:7700,10.0.0.5:7700`
pub const ENV_MESH_PEERS: &str = "MEMBRANE_MESH_PEERS";

// ── Standard system environment variables ────────────────────────────

/// XDG base directory for user data (fallback: `~/.local/share`).
pub const ENV_XDG_DATA_HOME: &str = "XDG_DATA_HOME";
/// XDG runtime directory (e.g. `/run/user/1000`).
pub const ENV_XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
/// XDG config directory (fallback: `~/.config`).
pub const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
/// User home directory.
pub const ENV_HOME: &str = "HOME";
/// System hostname.
pub const ENV_HOSTNAME: &str = "HOSTNAME";
/// Alternate hostname variable (some systems use HOST instead of HOSTNAME).
pub const ENV_HOST: &str = "HOST";
/// Cloudflare API token (alternate alias used by `wrangler`/Cloudflare tooling).
pub const ENV_CF_API_TOKEN: &str = "CF_API_TOKEN";
/// Cloudflare zone ID (alternate alias used by `wrangler`/Cloudflare tooling).
pub const ENV_CF_ZONE_ID: &str = "CF_ZONE_ID";

/// Forgejo SSH git server address (host:port).
pub const ENV_FORGEJO_GIT_ADDR: &str = "FORGEJO_GIT_ADDR";
/// Default Forgejo SSH address for git operations.
pub const DEFAULT_FORGEJO_GIT_ADDR: &str = "git.primals.eco:2222";

/// GitHub organization name (for release artifact URLs).
pub const ENV_GITHUB_ORG: &str = "MEMBRANE_GITHUB_ORG";
/// Default GitHub organization.
pub const DEFAULT_GITHUB_ORG: &str = "ecoPrimals";

/// Forgejo organization name (for repo paths).
pub const ENV_FORGEJO_ORG: &str = "MEMBRANE_FORGEJO_ORG";
/// Default Forgejo organization.
pub const DEFAULT_FORGEJO_ORG: &str = "sporeGarden";

/// WAN depot hostname (used in Caddy config and depot URLs).
pub const ENV_DEPOT_HOSTNAME: &str = "MEMBRANE_DEPOT_HOSTNAME";
/// Default depot hostname served by Caddy.
pub const DEFAULT_DEPOT_HOSTNAME: &str = "membrane.primals.eco";

/// Sovereign git remote name — authority-first push target.
///
/// This is the canonical remote that the temporal sync system converges to
/// before pushing to mirror remotes. Override for non-standard deployments.
pub const ENV_SOVEREIGN_REMOTE: &str = "MEMBRANE_SOVEREIGN_REMOTE";
/// Default sovereign remote name.
pub const DEFAULT_SOVEREIGN_REMOTE: &str = "forgejo";

/// When set to `1`/`true`/`yes`, cascade auto-triggers harvest+sandbox+refresh
/// when depot staleness is detected (production gates only).
pub const ENV_AUTO_REBUILD: &str = "MEMBRANE_AUTO_REBUILD";

/// Single-writer freshness publisher designation. Set to `1`/`true`/`yes`
/// on exactly one gate per mesh to avoid multi-writer race conditions.
pub const ENV_FRESHNESS_PUBLISHER: &str = "FRESHNESS_PUBLISHER";

/// `DigitalOcean` API token for cloud provisioning (fieldMouse droplets).
/// Fallback: `DO_TOKEN` (doctl-compatible).
pub const ENV_DIGITALOCEAN_TOKEN: &str = "DIGITALOCEAN_TOKEN";
/// `doctl`-compatible token fallback.
pub const ENV_DO_TOKEN_COMPAT: &str = "DO_TOKEN";

/// `DigitalOcean` REST API base URL.
pub const DEFAULT_DIGITALOCEAN_API: &str = "https://api.digitalocean.com/v2";
/// Cloudflare REST API (v4) base URL.
pub const DEFAULT_CLOUDFLARE_API: &str = "https://api.cloudflare.com/client/v4";
/// Default Forgejo admin username (for initial provisioning).
pub const DEFAULT_FORGEJO_ADMIN_USER: &str = "admin";
/// Default push remotes for K-Derm relay chain operations.
pub const DEFAULT_PUSH_REMOTES: &[&str] = &["forgejo", "origin"];
/// Default systemd service filter for membrane-related units (ERE `grep -E` syntax).
pub const DEFAULT_SERVICE_FILTER: &str =
    "membrane|forgejo|caddy|songbird|beardog|knot|hbb|fail2ban";

// ── LAN service discovery ────────────────────────────────────────────

/// LAN DNS domain suffix served by edge router dnsmasq.
///
/// Gates resolve each other as `<gate-lower>.primals.local` — e.g.
/// `sporegate.primals.local`, `eastgate.primals.local`. This replaces
/// hardcoded LAN IPs and enables hot-plug compute (gate IP can change
/// via DHCP without breaking resolution).
pub const LAN_DNS_DOMAIN: &str = "primals.local";

/// Build the LAN DNS hostname for a gate (lowercase + domain suffix).
///
/// Returns e.g. `sporegate.primals.local` for gate name `"sporeGate"`.
#[must_use]
pub fn lan_dns_name(gate_name: &str) -> String {
    format!("{}.{LAN_DNS_DOMAIN}", gate_name.to_lowercase())
}

// ── Gateway constants (Tower HTTP gateway) ───────────────────────────

/// Default bearDog TLS gateway bind address (production).
pub const DEFAULT_GATEWAY_BIND: &str = "0.0.0.0:443";
/// Default bearDog TLS gateway bind for shadow validation period.
pub const DEFAULT_GATEWAY_SHADOW_BIND: &str = "0.0.0.0:8443";
/// Default ACME HTTP-01 challenge port.
pub const DEFAULT_ACME_CHALLENGE_PORT: u16 = 80;
/// Default upstream timeout for reverse proxy routes (seconds).
pub const DEFAULT_GATEWAY_TIMEOUT_SECS: u32 = 30;
/// Default max upstream connections for gateway.
pub const DEFAULT_GATEWAY_MAX_CONNECTIONS: u32 = 100;
/// Default songBird socket path (for gateway → mesh routing).
pub const DEFAULT_SONGBIRD_SOCKET: &str = "/run/membrane/songbird.sock";
/// Default bearDog data directory (cert storage, state).
pub const DEFAULT_BEARDOG_DATA_DIR: &str = "/var/lib/beardog";
/// Default ACME directory URL (Let's Encrypt production).
pub const DEFAULT_ACME_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// Environment variable for the gateway bind address.
pub const ENV_GATEWAY_BIND: &str = "BEARDOG_GATEWAY_BIND";
/// Environment variable for gateway domains (comma-separated).
pub const ENV_GATEWAY_DOMAINS: &str = "BEARDOG_GATEWAY_DOMAINS";
/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY: &str = "BEARDOG_ACME_DIRECTORY";
/// Environment variable for the songBird socket path.
pub const ENV_SONGBIRD_SOCKET: &str = "BEARDOG_SONGBIRD_SOCKET";
/// Environment variable for songBird proxy route table (comma-separated `host/path=capability`).
pub const ENV_SONGBIRD_PROXY_ROUTES: &str = "SONGBIRD_PROXY_ROUTES";

// ── Timeout constants ────────────────────────────────────────────────

/// Default SSH connection timeout (seconds).
pub const DEFAULT_SSH_TIMEOUT_SECS: u64 = 10;
/// HTTP download timeout for binary fetch operations (seconds).
pub const DEFAULT_FETCH_TIMEOUT_SECS: u64 = 300;
/// Bootstrap phase timeout (seconds).
pub const DEFAULT_BOOTSTRAP_PHASE_TIMEOUT_SECS: u64 = 120;
/// Git operation timeout (seconds).
pub const DEFAULT_GIT_OP_TIMEOUT_SECS: u64 = 60;
/// Forgejo API write timeout (seconds).
pub const DEFAULT_API_WRITE_TIMEOUT_SECS: u64 = 30;
/// Forgejo API read timeout (seconds).
pub const DEFAULT_API_READ_TIMEOUT_SECS: u64 = 15;
/// Cloudflare API timeout (seconds).
pub const DEFAULT_CLOUDFLARE_TIMEOUT_SECS: u64 = 15;
/// Binary staleness threshold (seconds) — 7 days.
pub const DEFAULT_STALENESS_THRESHOLD_SECS: u64 = 7 * 86_400;
/// Canary maximum age (hours).
pub const DEFAULT_CANARY_MAX_AGE_HOURS: i64 = 168;
/// Sandbox health check timeout (seconds).
pub const DEFAULT_SANDBOX_HEALTH_TIMEOUT_SECS: u64 = 15;
/// Forgejo API pagination page size.
pub const DEFAULT_API_PAGE_SIZE: u32 = 50;

/// Default push mirror sync interval (Forgejo -> GitHub).
pub const DEFAULT_PUSH_MIRROR_INTERVAL: &str = "8h0m0s";

/// Repo name for the sporePrint Zola site.
pub const SPOREPRINT_REPO: &str = "sporePrint";

/// Environment variable controlling post-cascade Zola rebuild.
/// Set to `1`/`true`/`yes` to enable automatic `zola build` after
/// sporePrint is pulled during cascade.
pub const ENV_ZOLA_AUTO_BUILD: &str = "MEMBRANE_ZOLA_AUTO_BUILD";

// ── sporePrint NUCLEUS deployment ────────────────────────────────────

/// Default petalTongue content-serving bind address (loopback only, behind bearDog).
pub const DEFAULT_PETALTONGUE_BIND: &str = "127.0.0.1:8080";

/// Environment variable to override petalTongue bind address.
pub const ENV_PETALTONGUE_BIND: &str = "PETALTONGUE_BIND";

/// Default sporePrint content root relative to `ECOPRIMALS_ROOT`.
pub const SPOREPRINT_CONTENT_DIR: &str = "sporePrint";

/// Environment variable for the ACME domain (bearDog TLS).
pub const ENV_ACME_DOMAIN: &str = "BEARDOG_ACME_DOMAIN";

/// Default ACME email for certificate issuance.
pub const DEFAULT_ACME_EMAIL: &str = "acme@primals.eco";

/// The 4 binaries in a sporePrint NUCLEUS composition.
pub const SPOREPRINT_NUCLEUS_BINARIES: &[&str] =
    &["petaltongue", "nestgate", "songbird", "beardog"];

// ── Helpers ──────────────────────────────────────────────────────────

/// Read an environment variable, falling back to a compile-time default.
///
/// Reduces the `std::env::var(X).unwrap_or_else(|_| DEFAULT.into())` pattern
/// that appears 50+ times across the codebase.
#[must_use]
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_or_returns_default_when_unset() {
        let val = env_or("_CELLMEMBRANE_TEST_UNSET_VAR_xyz", "fallback");
        assert_eq!(val, "fallback");
    }

    #[test]
    fn default_socket_base_is_absolute() {
        assert!(DEFAULT_SOCKET_BASE.starts_with('/'));
    }

    #[test]
    fn default_push_remotes_nonempty() {
        assert!(!DEFAULT_PUSH_REMOTES.is_empty());
        assert!(DEFAULT_PUSH_REMOTES.contains(&"forgejo"));
    }

    #[test]
    fn lan_dns_name_lowercases_gate() {
        assert_eq!(lan_dns_name("sporeGate"), "sporegate.primals.local");
        assert_eq!(lan_dns_name("eastGate"), "eastgate.primals.local");
        assert_eq!(lan_dns_name("golgi"), "golgi.primals.local");
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn timeout_constants_reasonable() {
        assert!(DEFAULT_SSH_TIMEOUT_SECS >= 5);
        assert!(DEFAULT_GIT_OP_TIMEOUT_SECS >= 30);
        assert!(DEFAULT_FETCH_TIMEOUT_SECS >= 60);
    }
}
