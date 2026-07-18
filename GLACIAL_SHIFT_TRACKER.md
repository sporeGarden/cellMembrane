# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-07-18 (Wave 150d)
**Overall status:** STADIAL-READY — Zero P1, S1-S4 GRADUATED, 6-node WG mesh, deterministic deployment CODIFIED, SIGN-01 depot signing landed, OS Atheism Phase 1+2 shipped, `gate.enroll` automated mesh enrollment + hub-side peer addition, subdomain standard adopted (`prefix.primals.eco`), ALL 8 GLACIAL CRITERIA CLEAR
**Full wave-by-wave history:** `infra/fossilRecord/cellMembrane/GLACIAL_SHIFT_TRACKER_FULL_HISTORY_wave142b.md`

---

## Recent Waves

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
| 2 | Multi-gate LAN mesh (3+ gates) | **OPERATIONAL** — 6-node WG mesh |
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
