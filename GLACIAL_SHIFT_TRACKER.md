# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-07-16 (Wave 145a)
**Overall status:** STADIAL-READY — Zero P1, S1-S4 GRADUATED, 5-node WG mesh, deterministic deployment CODIFIED, SIGN-01 depot signing landed, OS Atheism Phase 1+2 shipped, ALL 8 GLACIAL CRITERIA CLEAR
**Full wave-by-wave history:** `infra/fossilRecord/cellMembrane/GLACIAL_SHIFT_TRACKER_FULL_HISTORY_wave142b.md`

---

## Recent Waves

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
| 2 | Multi-gate LAN mesh (3+ gates) | **OPERATIONAL** — 5-node WG mesh |
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
