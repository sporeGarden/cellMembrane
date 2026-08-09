// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deployment constants — paths, environment variables, timeouts, and defaults.
//!
//! Extracted from the service module to keep the service registry itself focused
//! on service definitions and capabilities.

/// Bind to all interfaces (externally reachable).
pub const BIND_ALL: &str = "0.0.0.0";
/// Bind to loopback only (not externally reachable).
pub const BIND_LOOPBACK: &str = "127.0.0.1";

/// Display fallback for missing diagnostic data (gate names, commit SHAs, etc.).
pub const UNKNOWN_LABEL: &str = "unknown";

/// Default base path for primal binary installations.
/// Override with `MEMBRANE_INSTALL_BASE` env var or membrane.toml config.
pub const DEFAULT_INSTALL_BASE: &str = "/opt/membrane";

/// Default base path for primal UDS sockets.
pub const DEFAULT_SOCKET_BASE: &str = "/run/membrane";

/// Default configuration directory (system-wide config files).
pub const DEFAULT_CONFIG_DIR: &str = "/etc/membrane";

/// Default TLS certificate directory for externally-provisioned certs.
pub const DEFAULT_TLS_CERT_DIR: &str = "/etc/membrane/tls";

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

/// Depot checksums manifest filename.
pub const CHECKSUMS_FILE: &str = "checksums.toml";

/// Depot signatures manifest filename.
pub const SIGNATURES_FILE: &str = "signatures.toml";

/// Depot provenance manifest filename.
pub const PROVENANCE_FILE: &str = "provenance.toml";

/// Per-arch GNU-style hash file (`b3sum --check` compatible).
pub const BLAKE3SUMS_FILE: &str = "BLAKE3SUMS";

/// Depot freshness manifest filename.
pub const FRESHNESS_FILE: &str = "freshness.toml";

/// Environment variable to override the depot trust policy.
pub const ENV_DEPOT_TRUST_POLICY: &str = "DEPOT_TRUST_POLICY";

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
/// Environment variable to override init system detection.
///
/// Values: `systemd`, `launchd`, `bare`. Enables user-space deploy on
/// platforms where system systemd is detected but root access is unavailable
/// (e.g. `SteamOS` Steam Deck: `MEMBRANE_INIT_SCOPE=bare`).
pub const ENV_INIT_SCOPE: &str = "MEMBRANE_INIT_SCOPE";
/// Environment variable for the Forgejo SSH host.
pub const ENV_FORGEJO_SSH_HOST: &str = "FORGEJO_SSH_HOST";
/// Environment variable for the ecoPrimals workspace root.
pub const ENV_ECOPRIMALS_ROOT: &str = "ECOPRIMALS_ROOT";
/// Environment variable for the gate identity.
///
/// Uses `MEMBRANE_GATE_NAME` (standard `MEMBRANE_*` prefix) as the primary
/// name. The legacy `GATE_NAME` is still checked as a fallback by
/// `resolve_gate_name_env()`.
pub const ENV_GATE_NAME: &str = "MEMBRANE_GATE_NAME";
/// Legacy environment variable for gate identity (pre-Wave 155m).
pub const ENV_GATE_NAME_LEGACY: &str = "GATE_NAME";
/// Environment variable for the webhook secret (HMAC-SHA256).
pub const ENV_WEBHOOK_SECRET: &str = "MEMBRANE_WEBHOOK_SECRET";
/// Legacy environment variable for webhook secret (pre-Wave 155m).
pub const ENV_WEBHOOK_SECRET_LEGACY: &str = "WEBHOOK_SECRET";
/// Environment variable for the `NeuralBridge` API socket path.
pub const ENV_NEURAL_API_SOCKET: &str = "NEURAL_API_SOCKET";

// ── Family / enrollment ──────────────────────────────────────────────

/// Environment variable for the family seed (BTSP/enrollment crypto).
pub const ENV_FAMILY_SEED: &str = "MEMBRANE_FAMILY_SEED";
/// Legacy environment variable for family seed.
pub const ENV_FAMILY_SEED_LEGACY: &str = "BEARDOG_FAMILY_SEED";
/// Additional legacy environment variable for family seed.
pub const ENV_FAMILY_SEED_LEGACY2: &str = "FAMILY_SEED";
/// Environment variable for the family ID (enrollment HKDF salt).
pub const ENV_FAMILY_ID: &str = "MEMBRANE_FAMILY_ID";
/// Legacy environment variable for family ID.
pub const ENV_FAMILY_ID_LEGACY: &str = "FAMILY_ID";
/// Environment variable for enrollment seed generation counter.
pub const ENV_ENROLLMENT_SEED_GEN: &str = "MEMBRANE_ENROLLMENT_SEED_GENERATION";
/// Legacy environment variable for enrollment seed generation.
pub const ENV_ENROLLMENT_SEED_GEN_LEGACY: &str = "BEARDOG_ENROLLMENT_SEED_GENERATION";

/// Default socket filename for the biomeOS Neural API.
pub const NEURAL_API_SOCKET_NAME: &str = "neural-api-default.sock";

/// Default socket filename for the webhook listener.
pub const WEBHOOK_SOCKET_NAME: &str = "webhook.sock";

/// File extension suffix for tarpc binary-protocol sockets (Cephalization G64).
///
/// Dual-socket primals expose JSON-RPC on `{name}.sock` and tarpc on
/// `{name}.tarpc.sock`. Health sweeps must filter by suffix to avoid
/// sending JSON-RPC probes to tarpc sockets.
pub const TARPC_SOCKET_SUFFIX: &str = ".tarpc.sock";

// ── step-ca SSH Certificate Authority ──────────────────────────────────

/// Default URL for the sovereign step-ca instance.
pub const DEFAULT_STEP_CA_URL: &str = "https://ca.primals.eco:9443";
/// Environment variable override for the step-ca URL.
pub const ENV_STEP_CA_URL: &str = "STEP_CA_URL";
/// Default SSH certificate lifetime (hours).
pub const DEFAULT_SSH_CERT_LIFETIME: &str = "8h";
/// Default SSH host ECDSA public key path.
pub const DEFAULT_SSH_HOST_KEY_PUB: &str = "/etc/ssh/ssh_host_ecdsa_key.pub";
/// Default SSH host ECDSA private key path.
pub const DEFAULT_SSH_HOST_KEY: &str = "/etc/ssh/ssh_host_ecdsa_key";
/// Default SSH host certificate path.
pub const DEFAULT_SSH_HOST_CERT: &str = "/etc/ssh/ssh_host_ecdsa_key-cert.pub";
/// Environment variable override for SSH certificate lifetime.
pub const ENV_STEP_CA_SSH_LIFETIME: &str = "STEP_CA_SSH_LIFETIME";
/// Environment variable for the step-ca provisioner name.
pub const ENV_STEP_CA_PROVISIONER: &str = "STEP_CA_PROVISIONER";
/// Default provisioner name.
pub const DEFAULT_STEP_CA_PROVISIONER: &str = "admin";
/// Environment variable for the step-ca root CA fingerprint (SHA256).
pub const ENV_STEP_CA_FINGERPRINT: &str = "STEP_CA_FINGERPRINT";
/// Default certificate storage directory under the membrane install base.
pub const STEP_CA_CERT_DIR: &str = "certs";

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
/// Default hub gateway mesh IP (golgiBody `.1` in the WG subnet).
pub const DEFAULT_HUB_MESH_IP: &str = "10.13.37.1";
/// Systemd `RuntimeDirectory` name for primal services.
///
/// systemd creates `/run/{name}` from this value. Used in all generated
/// unit files and `ServiceSpec` default construction.
pub const DEFAULT_RUNTIME_DIRECTORY: &str = "membrane";

/// Systemd `RuntimeDirectoryMode` for primal services.
///
/// `0755` allows non-root processes to traverse `/run/membrane/` and
/// connect to primal UDS sockets. Combined with `DEFAULT_SERVICE_UMASK`.
pub const DEFAULT_RUNTIME_DIRECTORY_MODE: &str = "0755";

/// Default file descriptor limit for primal services (`LimitNOFILE`).
///
/// Prevents FD exhaustion from auto-discovery loops, connection pooling, and
/// high-connection-count primals (biomeOS, songBird). The default 1024 is too
/// low for primals that maintain persistent socket pools.
pub const DEFAULT_LIMIT_NOFILE: u64 = 65536;

/// Systemd `UMask` for primal services.
///
/// `0002` causes socket files to be created as `srw-rw-r--` (0664) instead
/// of `srw-------` (0600), allowing non-root IPC clients (e.g. membrane CLI,
/// Neural API cross-primal routing) to connect.
pub const DEFAULT_SERVICE_UMASK: &str = "0002";

/// Default systemd unit directory (system scope).
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
/// User-scope systemd unit directory fallback (`$XDG_CONFIG_HOME/systemd/user`).
pub const SYSTEMD_USER_UNIT_DIR: &str = ".config/systemd/user";
/// Environment variable to override the systemd unit directory.
pub const ENV_SYSTEMD_UNIT_DIR: &str = "MEMBRANE_SYSTEMD_UNIT_DIR";
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
pub const DEFAULT_WAN_DEPOT_URL: &str = "https://depot.primals.eco";

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

// ── System identity ─────────────────────────────────────────────────

/// Environment variable for the real user ID (system-provided).
pub const ENV_UID: &str = "UID";
/// Environment variable for the effective user ID (system-provided).
pub const ENV_EUID: &str = "EUID";

// ── Cross-compilation ───────────────────────────────────────────────

/// Environment variable for the Android NDK home directory.
pub const ENV_ANDROID_NDK_HOME: &str = "ANDROID_NDK_HOME";

/// Environment variable for the build commit SHA embedded at compile time.
pub const ENV_BUILD_SHA: &str = "MEMBRANE_BUILD_SHA";

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

/// Default Tower Atomic drawbridge port.
pub const DEFAULT_TOWER_PORT: u16 = 7780;

/// Environment variable for Tower port override.
pub const ENV_TOWER_PORT: &str = "MEMBRANE_TOWER_PORT";

/// Default TURN relay port.
pub const DEFAULT_TURN_PORT: u16 = 3478;

/// Default builder service port (Tower Atomic dispatch).
pub const DEFAULT_BUILDER_PORT: u16 = 9800;

/// Default skunkBat metadata port.
pub const DEFAULT_SKUNKBAT_PORT: u16 = 9140;

/// Default rhizoCrypt primary port.
pub const DEFAULT_RHIZOCRYPT_PORT: u16 = 9601;

/// Default rhizoCrypt secondary port.
pub const DEFAULT_RHIZOCRYPT_SECONDARY_PORT: u16 = 9602;

/// Default loamSpine port.
pub const DEFAULT_LOAMSPINE_PORT: u16 = 9700;

/// Default sweetGrass port.
pub const DEFAULT_SWEETGRASS_PORT: u16 = 9850;

/// `RustDesk` hbbs (ID/rendezvous server) port.
pub const RUSTDESK_HBBS_PORT: u16 = 21115;

/// `RustDesk` hbbs NAT-type-test (UDP) port.
pub const RUSTDESK_HBBS_NAT_PORT: u16 = 21116;

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
pub const DEFAULT_DEPOT_HOSTNAME: &str = "depot.primals.eco";

/// Sovereign git remote name — authority-first push target.
///
/// This is the canonical remote that the temporal sync system converges to
/// before pushing to mirror remotes. Override for non-standard deployments.
pub const ENV_SOVEREIGN_REMOTE: &str = "MEMBRANE_SOVEREIGN_REMOTE";
/// Default sovereign remote name.
pub const DEFAULT_SOVEREIGN_REMOTE: &str = "forgejo";
/// Default GitHub mirror remote name for relay operations.
pub const DEFAULT_RELAY_GITHUB_REMOTE: &str = "origin";

/// When set to `1`/`true`/`yes`, cascade auto-triggers harvest+sandbox+refresh
/// when depot staleness is detected (production gates only).
pub const ENV_AUTO_REBUILD: &str = "MEMBRANE_AUTO_REBUILD";

/// When set to `1`/`true`/`yes`, this gate is a build authority — commit drift
/// detection auto-harvests drifted primals after cascade sync. Set on sporeGate.
pub const ENV_BUILD_AUTHORITY: &str = "MEMBRANE_BUILD_AUTHORITY";

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
///
/// **Deprecated:** Use `MembraneService::build_service_filter()` instead — it
/// derives the filter from the service registry and tracks new services
/// automatically.
#[deprecated(note = "use MembraneService::build_service_filter() instead")]
pub const DEFAULT_SERVICE_FILTER: &str =
    "membrane|forgejo|caddy|songbird|beardog|knot|hbb|fail2ban";

/// Infrastructure services included in the service filter that are NOT in the
/// membrane service registry (external daemons managed alongside the membrane).
pub const INFRA_SERVICE_FILTER_EXTRAS: &[&str] = &["forgejo", "fail2ban"];

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

/// Standard HTTPS port.
pub const DEFAULT_HTTPS_PORT: u16 = 443;
/// Shadow gateway validation port.
pub const DEFAULT_SHADOW_PORT: u16 = 8443;
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
/// Environment variable for the mesh relay socket path.
///
/// Capability-neutral name — resolves to whatever binary provides `MeshRelay`.
pub const ENV_SONGBIRD_SOCKET: &str = "MEMBRANE_MESH_RELAY_SOCKET";
/// Environment variable for songBird proxy route table (comma-separated `host/path=capability`).
pub const ENV_SONGBIRD_PROXY_ROUTES: &str = "SONGBIRD_PROXY_ROUTES";

// ── Timeout constants ────────────────────────────────────────────────

/// Default SSH connection timeout (seconds).
pub const DEFAULT_SSH_TIMEOUT_SECS: u32 = 10;
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
/// JSON-RPC call timeout over UDS/TCP (seconds).
pub const DEFAULT_JSONRPC_TIMEOUT_SECS: u64 = 3;
/// Sovereignty / HTTP probe timeout (seconds).
pub const DEFAULT_PROBE_TIMEOUT_SECS: u64 = 5;
/// TCP reachability probe timeout (seconds).
pub const DEFAULT_TCP_PROBE_TIMEOUT_SECS: u64 = 3;
/// Enrollment phase timeout (seconds).
pub const DEFAULT_ENROLL_PHASE_TIMEOUT_SECS: u64 = 30;
/// IPC write timeout over UDS (seconds).
pub const DEFAULT_IPC_WRITE_TIMEOUT_SECS: u64 = 2;
/// IPC read timeout over UDS (seconds).
pub const DEFAULT_IPC_READ_TIMEOUT_SECS: u64 = 5;
/// Sandbox health probe retries.
pub const DEFAULT_SANDBOX_PROBE_RETRIES: u32 = 5;
/// Sandbox health probe interval (milliseconds).
pub const DEFAULT_SANDBOX_PROBE_INTERVAL_MS: u64 = 2000;
/// Mesh socket wait retries during relay startup.
pub const MESH_SOCKET_WAIT_RETRIES: u32 = 5;
/// Mesh socket wait interval (seconds).
pub const MESH_SOCKET_WAIT_INTERVAL_SECS: u64 = 2;
/// Forgejo API pagination page size.
pub const DEFAULT_API_PAGE_SIZE: usize = 50;

// ── Well-known ports ─────────────────────────────────────────────────

/// DNS standard port.
pub const DEFAULT_DNS_PORT: u16 = 53;
/// Knot-DNS listen directives (IPv4 + IPv6).
pub const DEFAULT_KNOT_LISTEN: [&str; 2] = ["0.0.0.0@53", "::@53"];
/// HTTP standard port (ACME challenge, surface).
pub const DEFAULT_HTTP_PORT: u16 = 80;

// ── Systemd policy constants ─────────────────────────────────────────

/// Cascade oneshot timeout (seconds) for `TimeoutStartSec`.
pub const DEFAULT_CASCADE_TIMEOUT_SECS: u32 = 300;
/// Cascade timer jitter (seconds) for `RandomizedDelaySec`.
pub const DEFAULT_CASCADE_JITTER_SECS: u32 = 60;
/// Service restart delay (seconds) for `RestartSec`.
pub const DEFAULT_RESTART_DELAY_SECS: u32 = 5;
/// Start-limit burst window (seconds) for `StartLimitIntervalSec`.
pub const DEFAULT_START_LIMIT_INTERVAL_SECS: u32 = 120;
/// Start-limit burst count for `StartLimitBurst`.
pub const DEFAULT_START_LIMIT_BURST: u32 = 10;

// ── WireGuard ────────────────────────────────────────────────────────

/// Default `WireGuard` persistent keepalive (seconds).
pub const DEFAULT_WG_PERSISTENT_KEEPALIVE_SECS: u16 = 25;

/// Default push mirror sync interval (Forgejo -> GitHub).
pub const DEFAULT_PUSH_MIRROR_INTERVAL: &str = "8h0m0s";

// ── knot-dns paths ──────────────────────────────────────────────────

/// Default knot-dns configuration file path.
pub const DEFAULT_KNOT_CONF_PATH: &str = "/etc/knot/knot.conf";
/// Default knot-dns zone file directory.
pub const DEFAULT_KNOT_ZONE_DIR: &str = "/var/lib/knot/zones";

// ── Process lifecycle ───────────────────────────────────────────────

/// Settle delay after service restart (milliseconds).
pub const DEFAULT_RESTART_SETTLE_MS: u64 = 500;
/// Inter-primal delay during batch operations (milliseconds).
pub const DEFAULT_INTER_PRIMAL_DELAY_MS: u64 = 500;
/// Dependency wait retries during sandbox provisioning.
pub const DEFAULT_SANDBOX_DEP_RETRIES: u32 = 10;
/// Dependency wait max backoff (milliseconds).
pub const DEFAULT_SANDBOX_DEP_BACKOFF_MS: u64 = 8000;

/// Repo name for the sporePrint Zola site.
pub const SPOREPRINT_REPO: &str = "sporePrint";

/// Environment variable controlling post-cascade Zola rebuild.
/// Set to `1`/`true`/`yes` to enable automatic `zola build` after
/// sporePrint is pulled during cascade.
pub const ENV_ZOLA_AUTO_BUILD: &str = "MEMBRANE_ZOLA_AUTO_BUILD";

// ── sporePrint NUCLEUS deployment ────────────────────────────────────

/// Default petalTongue content-serving bind address (loopback only, behind bearDog).
pub const DEFAULT_PETALTONGUE_BIND: &str = "127.0.0.1:8080";
/// Default petalTongue content-serving port.
pub const DEFAULT_PETALTONGUE_PORT: u16 = 8080;

/// Environment variable to override petalTongue bind address.
pub const ENV_PETALTONGUE_BIND: &str = "PETALTONGUE_BIND";

/// Default sporePrint content root relative to `ECOPRIMALS_ROOT`.
pub const SPOREPRINT_CONTENT_DIR: &str = "sporePrint";

/// Environment variable for the ACME domain (bearDog TLS).
pub const ENV_ACME_DOMAIN: &str = "BEARDOG_ACME_DOMAIN";

/// Default ACME email for certificate issuance.
pub const DEFAULT_ACME_EMAIL: &str = "acme@primals.eco";

/// Whether a primal is in the postPrimordial set (requires signed depot lineage).
///
/// Checks the service registry `requires_signed_lineage` flag, falling back
/// to a check for "cellmembrane" (which is not in the registry since it IS
/// the membrane, but still requires signed lineage).
#[must_use]
pub fn is_post_primordial(primal: &str) -> bool {
    if primal == "cellmembrane" {
        return true;
    }
    super::MembraneService::for_binary(primal).is_some_and(|s| s.requires_signed_lineage)
}

// ── Composition domains ──────────────────────────────────────────────

/// Root domain for the sovereign membrane surface (intra-membrane).
pub const SURFACE_DOMAIN: &str = "primals.eco";
/// Default domain for footPrint composition.
pub const FOOTPRINT_DOMAIN: &str = "footprint.primals.eco";
/// Default domain for tideGlass composition.
pub const TIDEGLASS_DOMAIN: &str = "tideglass.primals.eco";
/// Default domain for esotericWebb composition.
pub const WEBB_DOMAIN: &str = "webb.primals.eco";
/// Default domain for sporePrint documentation site.
pub const SPOREPRINT_DOMAIN: &str = "sporeprint.primals.eco";
/// Default esotericWebb server port.
pub const DEFAULT_ESOTERICWEBB_PORT: u16 = 8090;

/// Default footPrint server bind (loopback, behind drawbridge).
pub const DEFAULT_FOOTPRINT_BIND: &str = "127.0.0.1:8090";
/// Default footPrint content port for health checks.
pub const DEFAULT_FOOTPRINT_PORT: u16 = 8090;

// ── Domain names ─────────────────────────────────────────────────────

/// Default git hosting domain.
pub const GIT_DOMAIN: &str = "git.primals.eco";
/// Default depot hosting domain.
pub const DEPOT_DOMAIN: &str = "depot.primals.eco";
/// Default mesh relay domain.
pub const MESH_DOMAIN: &str = "mesh.primals.eco";
/// Default lab/compute domain.
pub const LAB_DOMAIN: &str = "lab.primals.eco";

/// GitHub base URL for clone operations.
pub const GITHUB_HOST: &str = "github.com";
/// GitHub API base URL.
pub const GITHUB_API: &str = "https://api.github.com";

// ── Timestamp Formats ────────────────────────────────────────────────

/// ISO 8601 UTC timestamp format (strftime) — `2026-07-15T14:30:00Z`.
///
/// Canonical format spec for interop. `membrane-shadow` uses the `time` crate's
/// built-in formatters; this constant is retained for primals still using `chrono`.
pub const ISO8601_UTC: &str = "%Y-%m-%dT%H:%M:%SZ";

/// ISO 8601 timestamp with timezone offset (strftime) — `2026-07-15T14:30:00-04:00`.
///
/// Canonical format spec for interop. `membrane-shadow` uses the `time` crate's
/// built-in formatters; this constant is retained for primals still using `chrono`.
pub const ISO8601_TZ: &str = "%Y-%m-%dT%H:%M:%S%:z";

#[cfg(test)]
#[path = "constants_tests.rs"]
mod tests;
