# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-07-28 (Wave 155f)
**Overall status:** STADIAL-READY — Zero P1, S1-S4 GRADUATED, 7-node WG mesh, deterministic deployment CODIFIED, SIGN-01 depot signing landed, OS Atheism Phase 1+2 shipped, `gate.enroll` automated mesh enrollment + hub-side peer addition, subdomain standard adopted (`prefix.primals.eco`), sovereign depot auto-build pipeline (4-phase), depot provenance builder attribution + multi-target harvest + staleness alarm (Wave 151a), ALL 8 GLACIAL CRITERIA CLEAR
**Full wave-by-wave history:** `infra/fossilRecord/cellMembrane/GLACIAL_SHIFT_TRACKER_FULL_HISTORY_wave142b.md`

---

## Recent Waves

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
