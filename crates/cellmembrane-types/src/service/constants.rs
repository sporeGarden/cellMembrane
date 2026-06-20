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
/// Environment variable for the songbird federation port.
pub const ENV_FEDERATION_PORT: &str = "SONGBIRD_FEDERATION_PORT";
/// Environment variable for the songbird production bind address.
pub const ENV_PRODUCTION_BIND: &str = "SONGBIRD_PRODUCTION_BIND_ADDRESS";
/// Environment variable for the webhook secret (HMAC-SHA256).
pub const ENV_WEBHOOK_SECRET: &str = "WEBHOOK_SECRET";
/// Environment variable for the `NeuralBridge` API socket path.
pub const ENV_NEURAL_API_SOCKET: &str = "NEURAL_API_SOCKET";

/// Default socket filename for the biomeOS Neural API.
pub const NEURAL_API_SOCKET_NAME: &str = "neural-api-default.sock";

/// Namespace directory for biomeOS runtime sockets (under `XDG_RUNTIME_DIR` or /tmp).
pub const NEURAL_API_NAMESPACE: &str = "biomeos";
/// Environment variable for the peptidoglycan SSH host.
pub const ENV_PEPTI_SSH_HOST: &str = "PEPTI_SSH_HOST";
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
pub const DEFAULT_VPS_HOST: &str = "157.230.3.183";

/// Default SSH alias for golgiBody (internal name used in ~/.ssh/config).
pub const DEFAULT_SSH_ALIAS: &str = "golgi";
/// Default SSH alias for golgiBody external relay endpoint.
pub const DEFAULT_SSH_ALIAS_EXT: &str = "golgi-ext";
/// Default SSH alias for peptidoglycan trust barrier.
pub const DEFAULT_PEPTI_SSH_ALIAS: &str = "pepti";

/// Default `NestGate` service port.
pub const DEFAULT_NESTGATE_PORT: u16 = 9500;

/// Default songbird federation port.
pub const DEFAULT_FEDERATION_PORT: u16 = 7700;

/// Default TURN relay port.
pub const DEFAULT_TURN_PORT: u16 = 3478;

/// `RustDesk` hbbs (ID/rendezvous server) port.
pub const RUSTDESK_HBBS_PORT: u16 = 21115;
/// `RustDesk` hbbr (relay server) port.
pub const RUSTDESK_HBBR_PORT: u16 = 21117;

/// Default VPS mesh peer address (hub songbird federation endpoint).
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
/// GitHub REST API base URL.
pub const DEFAULT_GITHUB_API: &str = "https://api.github.com";
/// Default Forgejo admin username (for initial provisioning).
pub const DEFAULT_FORGEJO_ADMIN_USER: &str = "admin";
/// Default push remotes for K-Derm relay chain operations.
pub const DEFAULT_PUSH_REMOTES: &[&str] = &["forgejo", "origin"];
/// Default systemd service filter for membrane-related units (ERE `grep -E` syntax).
pub const DEFAULT_SERVICE_FILTER: &str = "membrane|forgejo|caddy|knot|hbb|fail2ban";

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
/// Cloud provision polling timeout (seconds).
pub const DEFAULT_PROVISION_POLL_TIMEOUT_SECS: u64 = 300;
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
