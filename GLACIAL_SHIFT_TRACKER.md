# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-08-06 (Wave 156s)
**Overall status:** STADIAL-READY — Zero P1, S1-S4 GRADUATED, 7-node WG mesh, deterministic deployment CODIFIED, SIGN-01 depot signing landed, OS Atheism Phase 1+2 shipped, `gate.enroll` automated mesh enrollment + hub-side peer addition, subdomain standard adopted (`prefix.primals.eco`), sovereign depot auto-build pipeline (4-phase), depot provenance builder attribution + multi-target harvest + staleness alarm (Wave 151a), ALL 8 GLACIAL CRITERIA CLEAR
**Full wave-by-wave history:** `infra/fossilRecord/cellMembrane/GLACIAL_SHIFT_TRACKER_FULL_HISTORY_wave142b.md`

---

## Recent Waves

**Wave 156s (G66 transport abstraction — silicon-agnostic byte pipes):**
Transport layer confining all `#[cfg(unix)]` IPC connection logic. (1) New
`TransportStream` enum (`Unix`, `Tcp`) implementing `AsyncRead + AsyncWrite +
Debug` — the platform-aware byte pipe. All `#[cfg(unix)]` for socket
connections confined to this enum and `connect_transport()`. (2)
`connect_transport(endpoint)` maps `TransportEndpoint` → `TransportStream`:
UDS on Unix, TCP cross-platform, with `Unsupported` error for NamedPipe
(future) and MeshRelay (requires songBird). (3) `endpoint_from_env_or_default()`
resolves `TRANSPORT_ENDPOINT` env var → platform default. (4)
`MembraneService::default_endpoint()` returns platform-appropriate transport
for any registry entry. (5) `negotiate_protocol()` (G65) refactored: now
delegates to `negotiate_protocol_endpoint()` which connects via
`connect_transport()` — eliminating the only unconditional `UnixStream` import
outside the transport layer. (6) Zero unconditional Unix imports remain in
business logic. Zero clippy (pedantic), zero fmt drift, 1312 tests pass (+9).

**Wave 156p (G65 protocol negotiation — cellMembrane discovery evolution):**
G65 Phase 3 of Cephalization. (1) New `IpcProtocol` enum (`JsonRpc`, `Tarpc`) with
G65 wire format (`PROTOCOLS:`/`PROTOCOL:` negotiation), `negotiate()` selection,
and `wire_name()`/`from_wire()` roundtrip. Constants: `PROTOCOL_NEGOTIATION_PREFIX`,
`PROTOCOL_NEGOTIATION_RESPONSE`, `PROTOCOL_NEGOTIATION_TIMEOUT_MS`. (2) `MembraneService`
evolves: `has_tarpc: bool` → `protocols: &'static [IpcProtocol]`; backward-compatible
`has_tarpc()` derived method retained; new `supports_negotiation()` for G65-aware
callers. (3) Registry: all 15 primals declared `DUAL_PROTOCOL` (`[JsonRpc, Tarpc]`),
external services `JSONRPC_ONLY`. bearDog corrected from false to true (shipped G65
with 30 tarpc methods). rhizoCrypt `ServerContract` evolved from `Tarpc` to `SocketOnly`
(G65 shipped). (4) `negotiate_protocol()` async client in `gate/sockets.rs` — sends
G65 handshake, reads response with timeout, returns `NegotiationResult { selected,
negotiated }`. (5) Health sweep enhanced: `probe_primal_jsonrpc()` attempts G65
negotiation before JSON-RPC fallback, logging successful negotiations. Zero clippy
(pedantic), zero fmt drift, 1303 tests pass (+10 new).

**Wave 156m (type-safe constants + #[from] error propagation + scheduler diagnostics):**
Safe constant types and idiomatic error propagation. (1) `DEFAULT_SSH_TIMEOUT_SECS`
changed from `u64` to `u32` at source — eliminates 2 `as u32` casts and 2 const
assertion blocks in `config.rs` and `gate/enroll.rs`. `DEFAULT_API_PAGE_SIZE` changed
from `u32` to `usize` — eliminates `as usize` cast. (2) 56 redundant
`map_err(ShadowError::Io|Toml|Serialize|Json)` calls replaced with `?` operator
across 16 files — leverages `#[from]` attributes already on `ShadowError` variants.
3 now-unused `ShadowError` imports removed. (3) `SchedulerDecision` dead fields
(`waiting`, `auto_promoted`) wired into harvest dispatch JSON output via
`ShadowOutcome::ok_with`, removing `#[allow(dead_code)]`. Zero clippy (pedantic),
zero fmt drift, 1293 tests pass.

**Wave 156l (visibility narrowing + Copy enums + signing extraction + base64 bump):**
Deep encapsulation and API hygiene pass. (1) Visibility narrowing: 5 `pub(crate)` fns
narrowed to private (`parse_topology_map`, `role_to_capability`, `resolve_ndk_strip`,
`generate_beardog_unit`, `resolve_primal_tarpc_socket_paths`) and 9 narrowed to
`pub(super)` (`validate_lineage`, `summarize_depot_freshness`, `resolve_ndk_linker`,
`spawn_primal_server`, `verify_local_depot`, `load_gateway_config`,
`resolve_canary_bin_dir`, `generate_songbird_unit`, `resolve_biomeos_socket_dir`).
(2) `Copy` derive added to fieldless enums `ImpulseType`, `Priority`, `FocusStatus` —
eliminates 3 unnecessary `.clone()` calls. (3) `plasmid/signing.rs` extracted:
`signing_crypto.rs` (164L) contains ed25519 verify + bearDog UDS signing client;
`signing.rs` reduced from 676L to 521L. (4) `base64` dependency bumped from 0.22.1 to
0.23.1. Zero clippy (pedantic), zero fmt drift, 1293 tests pass.

**Wave 156j-c (dispatch extraction + typed error constructors + error API consolidation):**
Smart refactoring and API evolution pass. (1) `dispatch/mod.rs` extracted from 713L
to 258L — harvest, webhook, and validate/rootpulse handlers moved to dedicated
`dispatch_harvest.rs`, `dispatch_webhook.rs`, and `dispatch_validate.rs` modules.
Router is now a clean prefix-match table. (2) `ShadowError` gained 7 typed
constructors (`http`, `build`, `config`, `git`, `ssh`, `parse`, `rpc`) taking
`impl Display`, eliminating verbose `ShadowError::Variant(format!(...))` and
`ShadowError::Variant("msg".into())` patterns. 80+ call sites migrated across
35+ files. (3) All remaining raw variant construction in production code converted
to constructors — only pattern matches and test assertions use raw variants.
(4) `#[must_use]` audit: `iter_binaries` was the only missing annotation; HTTP
client factories and `HttpResponse` methods already return `Result` (inherently
must-use). Zero clippy (pedantic), zero fmt drift, 1285 tests pass.

**Wave 156j-b (deep debt sweep: zero clippy + self-knowledge evolution + smart refactoring):**
Deep debt pass achieving ZERO clippy warnings for the first time. (1) Clippy:
`StalenessEntry`/`SchedulerDecision` dead fields annotated; `load_queue` rewritten
with `let...else`; `harvest_one` refactored by extracting `SourceResolution`/`resolve_source`
helper (116L→76L body, no too-many-lines warning). (2) Self-knowledge evolution:
added 4 new `ServiceCapability` variants (`DnsAuthority`, `ReverseProxy`, `Visualization`,
`ContentAddressedStorage`); knot-dns and caddy registry entries now declare capabilities;
`channels.rs::default_primal()` migrated from `for_binary("knot-dns")`/`for_binary("caddy")`
to `binary_for(DnsAuthority)`/`binary_for(ReverseProxy)`, eliminating 2 `.expect()` calls;
`sporeprint.rs` migrated from `require_binary("petaltongue")`/`require_binary("nestgate")`
to `binary_for(Visualization)`/`binary_for(ContentAddressedStorage)`;
`gateway/mod.rs` migrated from `require_binary("caddy")` to `require_capability(ReverseProxy)`.
(3) `constants.rs` shrunk from 823L to 692L by extracting runtime resolution functions
(`env_or`, `resolve_socket_base`, `resolve_systemd_unit_dir`, etc.) into new
`service/resolve.rs` module (138L). 1,285 tests, ZERO clippy, 0 fmt drift.

**Wave 156j (G64 Cephalization: dual-socket registry + tarpc-aware discovery + deep debt sweep):**
Cephalization era (G64). Added `has_tarpc: bool` field to `MembraneService` struct —
11 primals marked `has_tarpc: true` (all tarpc-wired/-serving), 2 primals + 4 symbiotic
marked `false`. Added `TARPC_SOCKET_SUFFIX` constant (`.tarpc.sock`). New
`ServicePaths::tarpc_socket_path()` and `MembraneService::resolved_tarpc_socket_path()`.
`ServerContract::Tarpc` now emits `--tarpc-socket {path}` alongside `--socket` in
`exec_args_with_base()`. Socket discovery: `resolve_primal_tarpc_socket_paths()` and
`is_tarpc_socket()` added to `gate/sockets.rs`. Health sweep guards: glob-based health
probe in `provision/bootstrap.rs` now skips `.tarpc.sock` via `case` filter; sandbox
`list_active()` also excludes tarpc sockets. Pre-existing gap fixed: `resolve_local_uds()`
in `resolve.rs` now includes `socket_aliases` from registry (was missing). Legacy
pre-Cephalization aliases `compute-tarpc` and `coralreef-tarpc` removed from toadstool
and coralreef — replaced by the `has_tarpc` mechanism. `CompositionSpec::all_socket_paths_resolved()`
returns `(binary, path, is_tarpc)` triples. Deep debt sweep: 13 pre-existing clippy
warnings fixed (`is_ok_and`, `const fn`, `if let` migration, `map_or`, `if_not_else`,
`doc_markdown`, `implicit_clone`). 1,285 tests (+4), 0 new clippy, 0 fmt drift.

**Wave 155v (J18: gate coupling — env_or migration + gate-name identity bridge):**
J18 portability fix: 3 production call sites that used `DEFAULT_INSTALL_BASE`
directly now use `env_or(ENV_INSTALL_BASE, DEFAULT_INSTALL_BASE)` — sandbox
dependency lookup (`sandbox.rs`), remote data file scan (`dispatch/mod.rs`),
and mesh relay binary probe (`tower/timer.rs`). Gate identity bridge: the
`mobility_phase` in `bootstrap_phases.rs` now writes a `gate-name` file
(system-scope `/etc/membrane/gate-name` or user-scope `~/.config/membrane/gate-name`)
so the shipped NM dispatcher hook (`99-mesh-reconnect`) can resolve identity
without the Rust binary running. This closes the documented `--mobile` gap.
Composition test hardened for socket permission changes on dev machines.
1,281 tests, 0 clippy, 0 fmt drift.

**Wave 155u (deep debt: TargetArch deprecation + XDG dedup + UNKNOWN_LABEL):**
Three-layer deep debt cleanup. (1) Deprecated legacy `TargetArch` enum — migrated
last 2 production callers (`harvest.rs`, `fetch.rs`) from `TargetArch::X86_64Gnu`
to `Platform::gpu().triple()`, added `#[deprecated]` attribute with migration note.
(2) Added `resolve_xdg_runtime_dir()` to `cellmembrane-types/service/constants.rs`
as single source of truth for `XDG_RUNTIME_DIR` resolution with `/run/user/{uid}`
fallback. Deduplicated 3 independent implementations (constants.rs, resolve.rs,
sockets.rs). Removed now-dead `resolve_uid()` and `DEFAULT_FALLBACK_UID` from
`gate/sockets.rs`. (3) Added `UNKNOWN_LABEL` constant and migrated 15 scattered
`"unknown"` display fallbacks across 10 files: bridge.rs, data.rs, commands.rs,
depot.rs, plasmid_dispatch.rs, deploy_dispatch.rs, provision_dispatch.rs,
verify.rs, key_portal.rs, auto_fetch.rs, tower/mod.rs, enroll_crypto.rs.
1,281 tests, 0 clippy, 0 fmt drift.

**Wave 155t (P2 fix: platform detection — detect_target_triple uses Platform::detect):**
Fixed membrane.exe embedding `x86_64-unknown-linux-musl` on Windows cross-compiled
builds. Root cause: `detect_target_triple()` delegated to `TargetArch::detect_host()`
which only has Linux variants and uses `cfg!(target_arch)` without OS dimension.
Fix: rewired to `Platform::detect().triple()` which uses `cfg!(target_os)` +
`cfg!(target_arch)` — correctly returns `x86_64-pc-windows-gnu` when cross-compiled
with `--target x86_64-pc-windows-gnu`. Also fixed `TargetOs::detect()` Android
detection: Android check was nested inside Linux branch but `target_os = "android"`
is distinct from `target_os = "linux"`, so it never matched. Moved Android check
first. Test `detect_target_triple_contains_musl` evolved to platform-agnostic
`detect_target_triple_matches_platform`. Unblocks J12 (blueGate sub-builder).
Sandbox composition test hardened for dev-machine biomeOS. 1,281 tests, 0 clippy.

**Wave 155s (registry API evolution — require_capability/require_binary):**
Collapsed 6 redundant `.expect()` calls on `MembraneService` lookups into two
new registry methods: `require_capability(cap)` returns `&'static MembraneService`
(panics on missing capability with structured message), `require_binary(name)`
does the same for named composition roles. `binary_for(cap)` is now a thin
wrapper around `require_capability`. Updated callers in `gateway/mod.rs` (3x),
`gate/systemd_units.rs` (1x), and `gate/sporeprint.rs` (2x). Hardened
`validate_via_composition` test: the original test assumed no running Neural API
on dev machine but eastGate runs biomeOS — evolved assertion to be environment-
agnostic. 1,281 tests, 0 clippy, 0 fmt drift.

**Wave 155r (J16 sources.toml + J13 freshness probe + CSPRNG safety):**
J16 sources.toml self-enrollment: extended `provision_sources_from_manifest()`
to include `Garden`-category repos alongside registry primals, so cellMembrane
(and other garden tooling) can be built via sovereign CI. The `build_args` from
repo `package` field is propagated, and `enrich_sources_from_manifest()` applies
`[build.*]` overrides as before. J13 depot freshness: `plasmid.staleness
--publish` now publishes stale primals to mesh via `depot.build_pending` so
consumer gates know a rebuild is pending. BTSP CSPRNG safety: evolved
`generate_ephemeral_pub()` from `expect()` to `Option` propagation — callers
already return `Option`, so the `?` operator propagates naturally. 1,281 tests,
0 clippy, 0 fmt drift.

**Wave 155q (coevolution contract — composition.test_swap wired):**
Wired `composition.test_swap` for broker primals (biomeOS). The sandbox
`validate_with_deps` function now auto-detects `BiomeosApi` contract primals
and delegates validation to the running biomeOS Neural API via
`composition.test_swap { binary_path }` instead of spawning a standalone
sandbox (which fails because biomeOS can't self-validate in isolation).
The running biomeOS spawns the candidate on a temp socket, probes via
`composition.self_test`, and returns `{ validated: true/false }`. On API
error, graceful fallback to standalone sandbox preserves non-broker
primal behavior. Resolves J19 (sandbox false positive blocking depot
deploy). 1,281 tests (+4: broker detection + composition path), 0 clippy.

**Wave 155p (sandbox P2 fix + socket-base init-scope migration):**
Sandbox biomeOS false positive fix: added `strip_sandbox_suffix()` to
`plasmid/mod.rs` so commit-suffixed binaries (e.g. `biomeos-abc12345`) resolve
to the correct `BiomeosApi` server contract instead of the default `server
--socket`. All 5 pipeline sandbox call sites (`sovereign.rs`, `pipeline.rs`,
`commands.rs`, `plasmid_dispatch.rs`, `bootstrap_phases.rs`) migrated from
`validate()` to `validate_with_deps()` so broker primals like biomeOS get their
bearDog dependency chain provisioned during sandbox validation. Socket-base
init-scope migration: 10 runtime call sites migrated from hardcoded
`env_or(ENV_SOCKET_BASE, DEFAULT_SOCKET_BASE)` to `resolve_socket_base()` which
is init-scope-aware (system/user/bare). Systemd unit generation paths
intentionally kept on `DEFAULT_SOCKET_BASE`. New constants: `ENV_BUILD_SHA`,
`ENV_HOME` usage in `resolve_systemd_unit_dir`. 1,277 tests (+4 new), 0 clippy,
0 fmt drift.

**Wave 155o (smart file splits + crypto dedup + constants sweep):**
Smart refactoring of the two remaining >700L files: extracted
`gate/bootstrap_phases.rs` from `bootstrap.rs` (738→291L orchestrator + 348L
phases), split `temporal/post_sync.rs` (718L) into three-file submodule
(`post_sync.rs` orchestrator + `post_sync_harvest.rs` + `post_sync_content.rs`).
Created shared `crypto.rs` module consolidating identical HKDF-SHA256 and
HMAC-SHA256 implementations from `btsp_client.rs`, `ribocipher.rs`, and
`enroll_crypto.rs` (~60L dedup, 4 new tests). Webhook signature verification
(`webhook/mod.rs`) also consolidated to use `crypto::hmac_sha256_hex`.
Registry fail-closed: `spawn_primal_server` now logs a warning on unregistered
binaries instead of silently defaulting. Constants sweep: added `ENV_UID`,
`ENV_EUID`, `DEFAULT_RELAY_GITHUB_REMOTE`, `ENV_ANDROID_NDK_HOME` to types
crate; all raw string env reads eliminated from production code.
1,273 tests (+4 new), 0 clippy, 0 fmt drift.

**Wave 155m (smart refactoring + registry-enforced self-knowledge):**
Smart refactoring: extracted `gate/sockets.rs` from `health.rs` (shared socket
resolution used by 4+ modules), consolidated duplicate mesh notify functions into
single parameterized `notify_mesh()`, externalized inline tests from `process.rs`
(-279L), `depot.rs` (-237L), `relay.rs` (-133L). Registry-enforced self-knowledge:
replaced silent `map_or` fallbacks with `expect()` on static registry lookups
(`channels.rs`, `gateway/mod.rs`, `sporeprint.rs`), `binary_for()` now panics on
missing capability instead of returning `"unknown"`. Unified socket resolution:
`tower/timer.rs` delegates to `gate/sockets.rs` for full registry-aware resolution
(api_socket aliases, XDG paths, socket_aliases). Renamed `BEARDOG_SONGBIRD_SOCKET`
to `MEMBRANE_MESH_RELAY_SOCKET` (capability-neutral). Split `dispatch/gate.rs`
(750→570L): firewall + WireGuard generators extracted to `gate_network.rs`.
Init-scope-aware socket discovery: `resolve_socket_base()` auto-adapts to
`MEMBRANE_INIT_SCOPE=user` (defaults to `$XDG_RUNTIME_DIR/biomeos`), fixing
impulse XDG namespace mismatch (`membrane` → `biomeos`) that would miss user-session
sockets on steamGate-style deploys. Consolidated 3 divergent `resolve_gate_name`
implementations (gate_configure, dispatch/mod, freshness) into single shared
`resolve_gate_name_async()` in `gate/local.rs`. Extracted `plasmid/lineage.rs`
(lineage validation) + `plasmid/commands.rs` (pipeline/trigger/status) from
`plasmid/mod.rs` (727→417L, -310L). Mesh notify now uses capability-resolved
socket via `resolve_mesh_relay_socket()` instead of hardcoded constant.
1,266 tests (+7 new), 0 clippy, 0 fmt drift.

**Wave 155n (MEMBRANE_* env var standardization + env secret unification):**
Standardized all cellMembrane env vars to `MEMBRANE_*` prefix convention with
legacy fallback chains for backward compatibility:
- `GATE_NAME` → `MEMBRANE_GATE_NAME` (+`GATE_NAME` legacy): fixed P3 mismatch
  where VPS `/etc/environment` had `MEMBRANE_GATE_NAME` but code read `GATE_NAME`.
  Introduced `resolve_gate_name_env()` shared helper; all 6 consumers updated
  (identity.rs, local.rs, temporal.rs, deploy_dispatch.rs + 2 test guards).
- `WEBHOOK_SECRET` → `MEMBRANE_WEBHOOK_SECRET` (+`WEBHOOK_SECRET` legacy): fixed
  live split where `webhook/listener.rs` read `MEMBRANE_WEBHOOK_SECRET` but
  `dispatch/mod.rs webhook.verify` read `WEBHOOK_SECRET`. Introduced
  `resolve_webhook_secret_env()`.
- `FAMILY_SEED`/`BEARDOG_FAMILY_SEED` → `MEMBRANE_FAMILY_SEED` (+2 legacy):
  centralized into `resolve_family_seed_env()`; all 3 consumers (enroll_crypto,
  btsp_client, ribocipher) updated. `FAMILY_ID` → `MEMBRANE_FAMILY_ID`,
  `BEARDOG_ENROLLMENT_SEED_GENERATION` → `MEMBRANE_ENROLLMENT_SEED_GENERATION`.
- Replaced hardcoded systemd unit names in `gateway/mod.rs` (songbird-relay,
  beardog-membrane) + `systemd_units.rs` (songbird-gateway) with registry
  `expect()` lookups, matching the pattern established in Wave 155m.
- All 14 systemd unit templates now emit `MEMBRANE_GATE_NAME` instead of `GATE_NAME`.
- Added `resolve_env_chain()` DRY helper for multi-key env resolution.
1,269 tests (+3 new), 0 clippy, 0 fmt drift.

**Wave 155k (sovereign HTTP client + deep debt sweep):**
Purged `reqwest` — sovereign HTTP/1.1 client built on `tokio-rustls` + `webpki-roots`
+ `ring`. Eliminates 63 transitive crates (191 → 128 deps). New `http_client.rs`:
`HttpClient` (reusable, `Arc<ClientConfig>`), `RequestBuilder` (fluent API),
`HttpResponse` (buffered, sync `.json()`/`.text()`/`.bytes()`), chunked transfer,
`InsecureVerifier` for shadow TLS. Migrated 27 call sites across 10 files.
Deep debt sweep: removed dead `manifest/wave.rs` module (150 lines), added
`DEFAULT_TLS_CERT_DIR` const, wired `arch.rs`/`channels.rs` to canonical constants,
evolved binary name hardcoding to registry lookups (`MembraneService::for_binary()`),
replaced port literals with `DEFAULT_HTTPS_PORT`/`DEFAULT_HTTP_PORT`, cleaned
stale reqwest refs in `deny.toml`. 1,259 tests, 0 clippy, 0 fmt drift.

**Wave 155i (P0 glibc depot + P1 WG DNS + deep debt evolution sweep):**
P0 closed: `targets_for_primal()` auto-appends `x86_64-unknown-linux-gnu`
for GPU primals. P1 closed: `WgConfig` `dns` field, `DNS =` in wg-quick.
Deep debt sweep #1: P0 sandbox fail-closed, registry-driven tower status,
5 dedup extractions, let-chains modernization. Net -135 lines.
Deep debt sweep #2: 45 new centralized constants (timeouts, retries, ports,
systemd policy, WG keepalive). SSH timeout `10` → `DEFAULT_SSH_TIMEOUT_SECS`,
JSON-RPC `3` → `DEFAULT_JSONRPC_TIMEOUT_SECS`, sovereignty probes `5` →
`DEFAULT_PROBE_TIMEOUT_SECS`, TCP probe `3` → `DEFAULT_TCP_PROBE_TIMEOUT_SECS`,
enrollment `30` → `DEFAULT_ENROLL_PHASE_TIMEOUT_SECS`, mesh socket wait →
`MESH_SOCKET_WAIT_RETRIES`/`INTERVAL_SECS`, cascade `300`/`60` →
`DEFAULT_CASCADE_TIMEOUT_SECS`/`JITTER_SECS`, `RestartSec=5` → constant,
`StartLimitBurst=10` → constant. Hardcoded `/24` → parsed from subnet CIDR.
Duplicate `default_wg_port()` → delegates to `wireguard::DEFAULT_WG_PORT`.
Channel ports `53`/`80`/`443`/`3478` → `DEFAULT_DNS_PORT`/`DEFAULT_HTTP_PORT`/
`DEFAULT_HTTPS_PORT`/`DEFAULT_TURN_PORT`. `@primals.eco` → `SURFACE_DOMAIN`.
Unnecessary `clone()` → `as_deref()` in enroll. SSH port `"22"` → const.
WG keepalive `25` → `DEFAULT_WG_PERSISTENT_KEEPALIVE_SECS`.
1,221 tests, 0 clippy, 0 fmt drift.

**Wave 155f (J6+J8: `gate.configure` / `gate.apply` + key enrollment portal + deep debt):**
J6 completion: `gate.configure` and `gate.apply` CLI commands implement
declarative service config generation from gate composition profile. Reads
`ecosystem_manifest.toml`, builds `ServiceSpec` per primal, renders to
detected init system (systemd/launchd/bare), and installs. `--env K=V`
overrides supported. Extracted to `gate_configure.rs` module. `nucleus.rs`
helpers promoted to `pub(crate)`. Deep debt: Tower port `7780` →
`DEFAULT_TOWER_PORT` + `ENV_TOWER_PORT` constants. Bootstrap arch triple →
`detect_target_triple()`.
J8 foundation: SSH certificate lifecycle via sovereign step-ca CA.
`gate/key_portal.rs` module with `request_ssh_certificate()`,
`renew_ssh_certificate()`, `install_host_certificate()`, `bootstrap_ca()`,
`inspect_certificates()`. New CLI: `gate.keys`, `gate.keys.renew`,
`gate.keys.renew --bootstrap`. `SshCertificate`/`SshCertType` types +
`CredentialModel::StepCa` variant. step-ca constants in `service/constants.rs`.
`gate.enroll` phase 8 (`ssh_cert`) wired. Deployment handoff for step-ca.
1,219 tests, 0 clippy, 0 fmt drift.

**Wave 155d (J1+J2+J6 jelly string codification + deep debt — Tower Atomic hardening):**
J1+J2 closed: `plasmid.push` promoted to first-class command (was `depot_sync
--push`). `plasmid.harvest --push` flag combines harvest→push in one invocation.
Default `depot_sync` refactored from monolithic embedded bash `for` loop (30-line
shell script sent over SSH) to Rust-orchestrated per-primal SSH commands
(`sync_single_remote()` with `RemoteSyncResult` enum, per-binary BLAKE3 diff +
atomic copy). `depot_sync_push_standalone()` enables harvest→push without
pre-built config. `harvest()` refactored: `finalize_depot()` and
`append_push_outcome()` extracted to satisfy 100-line clippy limit.
J6 foundation: `ServiceSpec` unified cross-platform config model in
`cellmembrane-types/process.rs` — `to_systemd_unit()`, `to_systemd_override()`,
`to_launchd_plist()` renderers. `from_membrane_service()` builder wires registry
+ `ServerContract`. `nucleus.rs` `generate_unit_content` delegates to
`ServiceSpec`. Tower timer `systemctl` calls guarded with `InitSystem`.
Deep debt: duplicate `SONGBIRD_FEDERATION_PORT` const → `DEFAULT_FEDERATION_PORT`.
Enroll `:7700` literal → constant. `crash_loop.rs` `query_unit_restart_info`
defense-in-depth guard. `gateway/mod.rs` domain fallback → `SURFACE_DOMAIN`.
Hardcoded arch triple in `tower/timer.rs` → `detect_target_triple()`.
1,194 tests, 0 clippy, 0 fmt drift.

**Wave 155b (cross-platform NUCLEUS + fleet convergence):**
G1 evolution: `nucleus.rs` evolved from systemd-only to `InitSystem::detect()`
dispatch — systemd (Linux), bare process spawn with PID file tracking (Windows,
macOS, containers). Added `stop_bare_process()`, `restart_bare_process()` for
non-systemd lifecycle. `nucleus_restart.rs` refactored: `converge_primal()`
extraction with `ConvergeOutcome` enum (was 105 lines, now 3 clean functions).
`crash_loop.rs` guards systemd-only scan with `InitSystem` check. Dead code
cleanup in `verify.rs` — removed orphaned `ChecksumFile`/`ChecksumEntry` structs.
Track B Fleet Convergence: composition profiles fixed upstream (compute/nest now
include Tower base primals), blueGate joining as distributed builder. cellMembrane
changes: `gate/verify.rs` migrated from private `ChecksumEntry` struct (could
not parse plain-string format) to shared `parse_checksums_toml()` — now handles
both `{ blake3 = "hash", size = N }` and legacy `"hash"` formats. `checksum`
module promoted from `mod` to `pub(crate) mod`. blueGate and westGate added to
`MESH_REGISTRY` (WG IPs pending allocation), `KNOWN_GATES`, and zone fallbacks
(blueGate → Backbone, westGate → House1). Build authority foreman pattern already
supported. Clippy `uninlined_format_args` fix in enroll.rs test.
1,182 tests, 0 clippy, 0 fmt drift.

**Wave 151b (BTSP ClientHello evolution + deep debt sweep):**
Sub-wave 151b: all primals that talk to bearDog must evolve to BTSP before
Nest Atomic. cellMembrane implements the 4-step `ClientHello` handshake in
new `btsp_client.rs` module (sync + async variants). HMAC-SHA256 challenge-
response using `FAMILY_SEED` via HKDF-derived BTSP key (distinct from mito
key). All 3 direct bearDog UDS clients evolved: `signing.rs` (depot signing),
`impulse/primal.rs` (impulse signing), `jsonrpc.rs` (health probes). Signal
prefix `[0xEC, 0x03]` (BTSP_JSON_LINE) sent before handshake. Graceful
fallback to plain JSON-RPC when `FAMILY_SEED` unavailable or handshake fails.
Tower status bearDog probe now uses BTSP path. `is_beardog_socket()` helper
detects crypto signer sockets by binary name. 11 new tests covering key
derivation, HMAC determinism, protocol serialization.
Post-BTSP deep debt sweep: `gateway/shadow.rs` hardcoded `"lab.primals.eco"` →
`LAB_DOMAIN` constant. `gate/enroll.rs` IP fallback `"10.13.37.1"` → new
`DEFAULT_HUB_MESH_IP` constant + `mesh_address()` chain. Preventive test
extraction: `post_sync.rs` (791→716L), `plasmid/mod.rs` (788→662L) — both
safely below 800L threshold. `getrandom` 0.2 → 0.4 (`getrandom::getrandom()`
→ `getrandom::fill()`, 2 call sites). Zero files >800L (largest 745L).
1,167 tests, 0 clippy, 0 fmt drift.

**Wave 151a (depot provenance + multi-target harvest + staleness alarm):**
P0 DEPOT DIVERGENCE: golgiBody depot 40 days stale, builder identity was OS
hostname (often `"unknown"`), no multi-arch support, no staleness detection.
`provenance.toml` builder attribution now uses `resolve_local_gate_identity()`
(reads `GATE_NAME` env / `.gate` file) — aligned with mesh notification identity.
Removed dead `hostname()` fallback. Multi-target harvest: `BuildEntry.targets`
from `[build.<primal>]` manifest section now wired into `targets_for_primal()` —
when manifest specifies `targets = ["x86_64-...", "aarch64-..."]`, harvest
builds all listed targets without CLI `--target` override. CLI `--target` still
takes priority. `ManifestBuildConfig` extended with `targets` field, populated
from `manifest.build`. `plasmid.status` drift alarm: parses `provenance.toml`
`generated` timestamp, computes age in days, warns when >7 days stale
(`DEPOT_STALE_THRESHOLD_DAYS = 7`). Status outcome `ok` now false when stale.
6 new tests (manifest targets, CLI override, staleness parsing).
1,156 tests, 0 clippy, 0 fmt drift.

**Wave 150x (crash-loop breaker + nestgate unit fix + test extraction):**
Systemd crash-loop divergence found on eastGate: `membrane-nucleus@nestgate` (17,920
restarts at 3s intervals — CLI evolved, `--socket` flag rejected) and `biomeos-beacon`
(11,161 restarts at 5s intervals — binary not built). Combined ~890 restarts/hour caused
AT&T gateway throttle. Crash-loop breaker implemented: `CrashLoopReport` types in
`cellmembrane-types/process.rs`, `gate/crash_loop.rs` module scans systemd `NRestarts`,
auto-disables services exceeding threshold (default 5), integrated into `gate.status`
health probes and cascade post-sync. `ServerContract::ServerNoSocket` variant added for
binaries whose CLI evolved past `--socket` flag. nestgate registry updated from
`SocketOnly` → `ServerNoSocket`. sporePrint unit template updated to pass socket via env
var instead of CLI flag. New CLI command: `membrane gate.crash-loop [--dry-run] [--threshold N]`.
`Restart=always` eliminated from all systemd units — replaced with `Restart=on-failure` +
`StartLimitIntervalSec=120` + `StartLimitBurst=10`.
Large test extraction: `manifest/mod.rs` (785→333L), `webhook/mod.rs` (703→345L),
`gateway.rs` (833→371L), `harvest.rs` (804→422L) — all tests in dedicated files.
Zero files >800L across entire workspace.
`MESH_REGISTRY` extended with `lan_ip` field — LAN peer discovery bootstrap for
songBird local-priority routing. eastGate corrected to `192.168.4.244`.
Deep debt: hardcoded sockets/IPs/paths → capability registry + constants
(`tower/timer.rs`, `enroll.rs`, `relay.rs`, `resolve.rs`, `tower/mod.rs`).
`as` casts → `try_from`/`f64::from`. `for_binary()` replaces manual `.find()`.
Unused `portable-atomic` dep removed. Bug-adjacent `"forgejo"` literal
inconsistency fixed in `resolve.rs` (now uses `sovereign_remote()`).
1,150 tests, 0 clippy, 0 fmt drift.

**Wave 150w (tower.shadow + checksums migration + deep debt sweep):**
`membrane tower.shadow` shipped (1,204 lines, 14 tests) — continuous WG vs Tower
transport shadow comparison across all mesh gate pairs. Shadow deploy ACTIVE on
3 gates (sporeGate, flockGate, golgiBody) with 60min continuous benchmarks.
`checksums.toml` format migration: custom `serde::Deserialize` for `ChecksumEntry`
accepts both struct entries `{ blake3 = "...", size = N }` and legacy plain-string
`"hash"` — resolves depot.integrity DEGRADED on flockGate. `ChecksumEntry` moved
from harvest.rs to checksum.rs (857→804L). `persist_checksums` now writes struct
format with size. Sovereign depot auto-build pipeline (Wave 150v): Forgejo
post-receive hook, commit drift detection, hard lineage enforcement for
`PostPrimordial` primals, build-pending mesh signal. Deep debt sweep: unified mesh
registry, shared staging (60L dedup), capability-based naming, let-chain flattening,
guard consolidation, `format!` to `.display().to_string()` idiom fixes.
1,136 tests, 0 clippy, 0 fmt drift.

**Wave 150t (docs sweep + wateringHole reorg alignment):**
Root docs updated: README stale refs (Wave 147e→150t), test count (1,089→1,101),
depot URL (`depot.primals.eco`), mesh count (6→7 with southGate), Related Resources
paths updated for wateringHole 150t directory reorg (compositions/, foundations/,
fossilRecord/). Phantom `experiments/` dir removed from tree listing. VPS_STATE.md
updated (mesh, depot, validation). RUNBOOKS and IRONGATE bumped to 150t.
wateringHole handoff written for overwatch. Zero false-positive TODOs in codebase.
`cargo clean` reclaimed 1.3G. 1,101 tests, 0 clippy, 0 fmt drift.

**Wave 150o (mesh registry IP correction + methodology audit):**
southGate IP corrected from .8 to .9 (northGate keeps .8). Both northGate and
southGate now in `KNOWN_MESH_GATES` with correct addresses (.8 and .9).
Dimensional review's "456 production unwrap" and "2 unsafe" are false positives:
grep excludes `#[test]` lines but not `#[cfg(test)]` module bodies; "unsafe"
matches `#![forbid(unsafe_code)]` attributes and `'unsafe-inline'` CSP strings.
Actual: 0 production unwrap, 0 unsafe code. 1,101 tests, 0 clippy.

**Wave 150n (mesh registry: southGate allocated, origin remote fix):**
Origin remote fixed: `ecoPrimals/cellMembrane` → `sporeGarden/cellMembrane`
on Forgejo (Wave 150l canonicalization).

**Wave 150k (unwrap audit — 551 test-only, 0 production):**
Dimensional review flagged 551 `.unwrap()` as P1. Full audit confirms ALL 551
are in test code. Zero production unwrap/expect(invariant)/panic/todo.

**Wave 150h (depot URL evolution + NUCLEUS composition milestone):**
Full NUCLEUS composition wired — both footPrint and esotericWebb consumer-side
connections COMPLETE (petalTongue WS bridge + nestGate CAS). Depot defaults
evolved from legacy `membrane.primals.eco/depot` to subdomain-standard
`depot.primals.eco`. `DEFAULT_WAN_DEPOT_URL` and `DEFAULT_DEPOT_HOSTNAME`
updated. All P1 inter-primal wiring items RESOLVED on both provider and
consumer sides. 1,100 tests, 0 clippy warnings.

**Wave 150d (subdomain standard — routing architecture overhaul):**
URL standard adopted: all compositions use `prefix.primals.eco` subdomains.
Path-based routing (`/webb/`) eliminated. `ESOTERICWEBB_PATH` removed,
replaced by `WEBB_DOMAIN = "webb.primals.eco"`. `SPOREPRINT_DOMAIN` added.
footPrint Caddy simplified: catch-all → footPrint:8090 (Express handles
everything), `/ws` → petalTongue:8080 (agent bridge). CSP headers added
for Esri/OSM tile domains. esotericWebb Caddy: simple vhost at
`webb.primals.eco` → flockGate:8090. Root domain `primals.eco` redirects
to `sporeprint.primals.eco`. Gateway routes updated for subdomain standard.
`cargo fmt` applied (62 files, Wave 149b). 1,100 tests, 0 clippy warnings.

**Wave 148a (esotericWebb deploy fix — port + unit + Caddy correction):**
esotericWebb LIVE on flockGate:8090 (AAR resolved all 3 deploy blockers).
Port confusion clarified: 8080 = nestGate/petalTongue, 8090 = esotericWebb.
`esotericwebb-server.service` ExecStart fixed from `server --socket` to
`serve --content content/ --listen 0.0.0.0:8090`, WorkingDirectory added,
Restart policy changed to `on-failure`. `DEFAULT_ESOTERICWEBB_PORT` constant
(8090) added. Caddy generation for `/webb/*` sub-route fixed from
petalTongue :8080 → esotericWebb :8090. 1,100 tests, 0 clippy warnings.

**Wave 147e (zone fix + esotericWebb Caddy + composition service units):**
`ZoneLabel::House1` variant: unblocks cascade for northGate (manifest `zone = "house1"`
was silently falling to `Unassigned`). northGate added to `KNOWN_MESH_GATES`,
`KNOWN_GATES`, and `mesh_address` registry (10.13.37.8). House1 requires WG overlay.
`GateRole::EsotericWebb` typed variant. Caddy block for `primals.eco/webb/*` via
sub-route on root domain. NUCLEUS service units: `footprint-server.service` (sporeGate)
and `esotericwebb-server.service` (flockGate). `SURFACE_DOMAIN` + `ESOTERICWEBB_PATH`
constants. Gateway `default_routes_for_roles` updated for esotericWebb.
1,100 tests, 0 clippy warnings.

**Wave 147c (footPrint Caddy blocks + typed composition roles):**
Caddy blocks for footPrint API endpoints: `CaddySubRoute` type + `handle` block
rendering. `footprint.primals.eco` routes `/api/*` → footPrint server (8090),
`/ws` → petalTongue WS (8080), catch-all → petalTongue static (8080).
`GateRole::FootPrint` and `GateRole::TideGlass` promoted from `Other(String)` to
typed variants — eliminates stringly-typed matching in gateway config and Caddy
generation. Gateway `default_routes_for_roles` updated. `DEFAULT_PETALTONGUE_PORT`
constant added. tideGlass Caddy upstream corrected to petalTongue port.
1,096 tests, 0 clippy warnings.

**Wave 147b (hub.peer — hub-side peer addition + WG refactor):**
New `hub.peer` phase in `gate.enroll`: reads local WG pubkey, resolves hub
gate from manifest, SSHs to hub to run `wg set wg0 peer <pubkey> allowed-ips`.
Eliminates the manual SSH step for hub-side enrollment. WG helpers extracted
from `enroll.rs` into `gate/wg.rs` (smart refactor: enroll 503L, wg 370L).
Const assertion for SSH timeout bounds.
Timestamp dedup: 12 inline `chrono::Utc::now()` sites → 4 centralized helpers
(`utc_now_iso8601`, `utc_today`, `utc_now_rfc3339`, `utc_now_compact`).
HTTP client dedup: 8 `reqwest::Client::builder()` sites → 2 centralized
helpers (`http_client`, `http_client_insecure`).
1,089 tests, 0 clippy warnings.

**Wave 147a (gate.enroll — automated mesh enrollment):**
New `gate.enroll` command: WG keygen, wg-quick config render from manifest,
mesh connectivity verify, Forgejo SSH verify, Forgejo-first git remote config.
Implements the enrollment standard from northGate AAR — `origin` = Forgejo (sovereign),
`github` = GitHub (mirror). 6-gate mesh live (northGate 10.13.37.8 enrolled).
8 new tests (manifest→WG config, self-exclusion, URL format, rendered output, dry-run).
1,081 tests, 0 clippy warnings.

**Wave 145a (deep debt — let-chains modernization):**
Nested `if let` patterns → Rust 2024 let-chains across 8 files (manifest, resolve, caddy,
health, dispatch/data, post_sync, canary, canary_remote). Eliminates unnecessary nesting.
Ecosystem: Phase 2 Transport 14/14 COMPLETE, CAC 6/6 COMPLETE.
1,073 tests, 0 clippy warnings.

**Wave 143b (deep debt — typed probes, CSPRNG, registry filter, dead code cleanup):**
CSPRNG: platform-split `fill_random` (urandom + BLAKE3 fallback) → unified `getrandom` crate.
Service filter: hardcoded regex → `MembraneService::build_service_filter()` (registry-derived).
`ProbeResult` struct replacing 9 `(bool, String)` tuples across gate health/verify/nucleus/mesh.
`Priority::Priority` → `Priority::Urgent` (serde alias preserves wire compat).
`format_bytes` f64 casts → integer-only math with half-up rounding.
`DepotUpdatedNotification` `pub`→`pub(crate)`, `from_json` returns `Self` (not `Option`).
Duplicate `build_err` helpers consolidated (3 files → direct `ShadowError::Build`).
Dead code `#[allow]` attributes replaced with `reason` annotations throughout.
1,073 tests, 0 clippy warnings.

**Wave 142b (deep debt sweep — visibility, allocation, error taxonomy, domain centralization, CAC tree-parity):**
Visibility: 20 modules `pub`→`pub(crate)`, dead code removed (5 dead fns, 1 dead struct).
Allocation: `detect_target_triple()`→`const fn &'static str` (~25 allocs eliminated),
`compute_blake3_file_async(impl AsRef<Path>)`, `verify_blake3_async(impl AsRef<Path>, &str)`.
Error taxonomy: 8 `ShadowError::Parse` reclassified. Domain constants: `GIT_DOMAIN`,
`DEPOT_DOMAIN`, `MESH_DOMAIN`, `LAB_DOMAIN`, `GITHUB_HOST`, `GITHUB_API` centralized.
CAC P1: `sync_diverge` checks tree parity before impulse/policy (Newton-Leibniz).
`try_pull_converge` checks trees_match after rebase conflict. Caddy blocks for
footPrint + tideGlass wired from manifest roles. 1,072 tests, 0 clippy.

**Wave 140a (deep debt — constants, types, dependency evolution, OS Atheism Phase 2):**
Constants & dedup: `ISO8601_UTC`/`ISO8601_TZ` (18 format strings),
`DEFAULT_HTTPS_PORT`/`DEFAULT_SHADOW_PORT`. `FromStr` for `MembraneComposition`,
`WebhookProvider`. JSON substring probes → `serde_json` structural checks (7 sites).
`nix` crate eliminated. Smart refactor: `plasmid/mod.rs` 875→514L, `harvest.rs` 841→763L.
OS Atheism Phase 2: `TransportEndpoint::NamedPipe`, `InitSystem::detect()`,
platform-aware CSPRNG/chmod. Cascade hang fix (`BranchCheckedOut`, reconcile timeout).
`harvest --local`, `depot_sync --push`, `sources.toml` auto-provision. 1,074 tests.

---

## Stadial Entry Criteria

All criteria satisfied — stadial-ready.

| # | Criterion | Status |
|---|-----------|--------|
| 1 | All 4 sovereignty shadows cut over (7-day gates) | S1 **OPERATIONAL**, S2 LIVE, S3 LIVE, S4 **GRADUATED** |
| 2 | Multi-gate LAN mesh (3+ gates) | **OPERATIONAL** — 7-node WG mesh |
| 3 | Nest expansion deployed on VPS | **LIVE** (Wave 38) |
| 4 | Remote covalent node (WAN) | **flockGate LIVE** (16 bonds) |
| 5 | DNS pointed to sovereign infrastructure | **knot-dns RUNNING** — NS cutover pending (registrar) |
| 6 | Cloudflare removed from production path | Tunnel orphaned — Caddy + LE sole TLS |

---

## Remaining Blocker — NS Cutover (Criterion #5)

knot-dns **running** on VPS with DNSSEC. Zone configured, UFW :53 open.
Remaining step: registrar NS delegation update (permanently external dependency).

---

## Sovereignty Shadow Status

| Track | Sovereign | Shadow | Status |
|-------|-----------|--------|--------|
| S1 TLS | Caddy + LE | Cloudflare (INACTIVE) | **OPERATIONAL** — sole TLS provider |
| S2 NAT | Songbird :3478 | cloudflared | **LIVE** |
| S3 Content | NestGate + petalTongue | GitHub Pages | **LIVE** (68ms TTFB) |
| S4 Auth | BearDog BTSP | OAuth2/PAM (disabled) | **GRADUATED** |

---

## Dark Forest Compliance

| Pillar | Requirement | Status |
|--------|-------------|--------|
| 1 | Zero metadata leakage (stripped binaries) | PASS |
| 2 | Zero port exposure (UDS default, composition-aware UFW) | PASS |
| 3 | Songbird sole network surface | PASS |
| 4 | BTSP crypto integrity (13/13 primals) | PASS |
| 5 | Enclave computing (dual-tower ionic pattern) | PASS |
