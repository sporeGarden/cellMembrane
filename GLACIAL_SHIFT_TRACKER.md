# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-05-28
**Overall status:** PROGRESSING — NUCLEUS typed, biomeOS deploy P0, NS cutover remaining
**Wave 59 update:** NUCLEUS composition tier (13 primals, 17 services) typed into service registry.
6 new services: toadStool, barraCuda, coralReef, biomeOS, squirrel, petalTongue. All UDS-only.
`membrane.toml` evolved to `composition = "nucleus"`. 175 tests. Spring overlay readiness proven.
primalSpring Wave 59 corrections applied: S2 DNS = "DEPLOYED", S4 CI = GitHub Actions gap.
**Wave 57 update:** Deep debt sprint — `clippy::pedantic` + `nursery` enforced (zero warnings),
typed `ConfigError` via `thiserror`, `DeployPaths` configurable paths, `iter_binaries()` zero-copy,
`#[must_use]` + `const fn` across all pure functions, scyBorg triple license, `cargo-deny` ecoBin
ban list, coverage **77% → 96%** (160 tests). Zero TODOs, zero unsafe, zero C deps. Audit-ready.
**Wave 56 update:** primalSpring Wave 56 VPS deployment standard consumed. `TransportMode::UdsOnly`
typed into service registry. `deploy_membrane.sh` evolved with `--uds-only` flag for `nucleus_launcher`
integration and `spring-overlay` mode for cell graph deployment. Three-step VPS deploy flow documented.
`membrane.toml` → `transport = "uds_only"`. Zero clippy warnings, all tests pass.
**Wave 55 update:** primalSpring audit (NC-3) identified stale docs — ops docs now synced to
Nest Atomic reality. `membrane.toml` → `composition = "nest"`, signal channel enabled.
K-Derm boundary published. knot-dns running with DNSSEC — registrar NS cutover remaining.
**Wave 51 update:** Deep debt sprint across all 3 owned repos (cellMembrane, benchScale, agentReagents).
benchScale + agentReagents converged from `sort-after/` into canonical `infra/` locations.
K-Derm diderm topology wired into benchScale. postPrimordial compliance enforced.
**Wave 50 update:** GitHub Actions incident (May 26) proved external CI is unacceptable.
Self-hosted runner deployed on ironGate. plasmidbin validate 98/98 PASS locally.

---

## Stadial Entry Criteria

All six criteria must be satisfied before glacial shift.

| # | Criterion | Status | cellMembrane Role | Blocker Owner |
|---|-----------|--------|-------------------|---------------|
| 1 | All 4 sovereignty shadows cut over (7-day gates) | S2 LIVE, S3 LIVE, S1/S4 incomplete | Operate shadow infrastructure | Shared |
| 2 | Multi-gate LAN mesh operational (3+ gates in Plasmodium) | **4 gates running** (Wave 50), mesh seeded | Provide TURN rendezvous | Gate teams |
| 3 | Nest expansion deployed on VPS | **LIVE** (Wave 38, 2026-05-22) — 21/0/1 darkforest, 10/10 trio | Operate Nest Atomic | **RESOLVED** |
| 4 | Remote covalent node (flockGate) validated over WAN | flockGate not deployed | Provide WAN rendezvous | Shared |
| 5 | DNS pointed to sovereign infrastructure | **knot-dns RUNNING** with DNSSEC — NS cutover to primary pending (registrar) | Complete registrar NS record change | **cellMembrane** |
| 6 | Cloudflare/cloudflared removed from production path | S1 not cut over | Caddy → BearDog ACME cutover | Shared |

---

## Resolved Blockers

### ~~Blocker 1: Nest Expansion on VPS (Criterion #3)~~ — RESOLVED

**Deployed:** Wave 38 (2026-05-22) via `deploy_membrane.sh deploy --composition nest --validate`

| Service | Port | Version | Status |
|---------|------|---------|--------|
| nestgate-membrane | :9500 | v2.1.0 | RUNNING |
| rhizocrypt-membrane | :9602 | v0.14.0 | RUNNING |
| loamspine-membrane | :9700 | v0.9.16 | RUNNING |
| sweetgrass-membrane | :9850 | v0.7.34 | RUNNING |

**Validation:** darkforest 21/0/1, provenance trio 10/10, shadow orchestrator 6/6.

---

## Remaining Blockers — cellMembrane Action Items

### Blocker: Sovereign DNS — NS Cutover to Primary (Criterion #5)

**What:** Complete the registrar NS record change to make knot-dns primary for `primals.eco`.

**Current state:** knot-dns is **running** on the VPS with DNSSEC (H2-17 complete). Zone file
configured, UFW :53 open. Remaining step is the registrar NS delegation update.

**Checklist:**
- [x] knot-dns installed and running on VPS
- [x] Zone file configured for `primals.eco`
- [x] DNSSEC enabled
- [x] UFW port :53 tcp/udp open
- [ ] Registrar NS record update (secondary → primary cutover)
- [ ] Validate resolution: `dig @$VPS_IP primals.eco`
- [ ] 7-day monitoring period before declaring S2 closed

**Dependencies:**
- ICANN registrar cooperation (permanently external)
- Current commercial DNS must remain available during transition

### Blocker: Deploy biomeOS v3.84 to VPS (P0 — Critical Path)

**What:** Deploy the full NUCLEUS composition (13 primals) including biomeOS v3.84.
This unblocks all spring emissions and column U progression.

**Current state:** biomeOS binary available in plasmidBin (`primals/x86_64-unknown-linux-musl/biomeos`).
`deploy_membrane.sh deploy --composition nucleus --uds-only` ready. Cell graphs ready in primalSpring.
`cellmembrane-types` NUCLEUS composition typed (17 services, 175 tests).

**Checklist:**
- [x] biomeOS binary harvested in plasmidBin ecoBin
- [x] `deploy_membrane.sh` supports `--composition nucleus --uds-only`
- [x] `spring-overlay` mode implemented in deploy script
- [x] Cell graphs available: `hotspring_cell.toml` (6 VPS-standard springs)
- [x] `cellmembrane-types` models NUCLEUS composition (13 primals, 6 new services)
- [x] `membrane.toml` updated to `composition = "nucleus"`
- [ ] Execute: `deploy_membrane.sh deploy root@$VPS_IP --composition nucleus --uds-only`
- [ ] Verify: all 13 primals healthy via UDS sockets
- [ ] Test: `deploy_membrane.sh spring-overlay root@$VPS_IP --cell hotspring`
- [ ] Verify: hotSpring column U pass

**Dependencies:**
- SSH access to VPS (available)
- DNS resolution working (currently intermittent — see CI note)

### Observation: CI Sovereignty Gap (S4)

**Noted by primalSpring Wave 59:** Git hosting is Forgejo-primary but CI/CD
is still GitHub Actions. This is a glacial gate **observation**, not a stadial
blocker. Options: Forgejo Actions, self-hosted runners on LAN gates.

---

## Supporting Work (Not Direct Blockers)

### Self-Hosted GitHub Actions Runners (Wave 50)

GitHub Actions incident on May 26 proved external CI dependency is unacceptable.
Self-hosted runners on LAN gates provide free minutes and local toolchains. However,
the same incident revealed a deeper issue: GitHub's job dispatch plane is also
degraded during outages — self-hosted runners can't receive jobs even when online.
True CI sovereignty requires Forgejo Actions to own the dispatch plane.

**Status:** ironGate runner **ONLINE** at org level (v2.334.0). Serves all ecoPrimals repos.
All 5 plasmidBin workflows evolved to sovereign-first `runs-on` strategy. `validate.yml`
uses raw git checkout — zero marketplace action dependency, survives codeload outages.

**Handoff:** `infra/wateringHole/handoffs/archive/CELLMEMBRANE_SELF_HOSTED_RUNNERS_WAVE50_MAY26_2026.md`

**Acceptance:**
- [x] ironGate runner online (org-level): `irongate-runner online self-hosted,Linux,X64,x86_64,irongate,lan`
- [x] `plasmidbin validate .` passes on ironGate: **98/98 PASS**
- [x] Static musl binary builds: x86_64 + aarch64 cross-compile verified
- [x] All 5 workflows sovereign-first (self-hosted default, `USE_GITHUB_HOSTED` override)
- [x] validate.yml raw git checkout (no `uses: actions/checkout@v4` dependency)
- [ ] Dispatch completes on self-hosted runner (blocked by GitHub incident — dispatch plane degraded)
- [ ] 2nd runner online (eastGate or southGate)
- [ ] Forgejo Actions evaluated as dispatch plane replacement

### cellMembrane Formalization + K-Derm Topology (Wave 50)

cellMembrane formalized from operational docs + bash into a typed Rust system.
5 spec documents define the architecture, composition model, fieldMouse contract,
multi-membrane deployment, and K-Derm cell envelope topology. `cellmembrane-types`
crate provides typed config parsing, firewall derivation, envelope topology, and
validation — **160 tests** across 8 domain test modules, **zero clippy warnings**
(pedantic + nursery), **95.8% line coverage** (llvm-cov).

Gap analysis against `darkforest_membrane.sh` (MEM-01..17) and `s_membrane_composition.rs`
(Pillar 4 telemetry) closed 5 gaps: journald persistence, credential file inventory,
binary integrity, RustDesk key paths, telemetry/shadow config.

K-Derm topology models inner/outer membrane sync as monoderm/diderm cell envelopes
with absolute layer naming (cytoplasm → plasma membrane → periplasm → outer membrane
→ extracellular), bonding per layer (organo-metallo-salt model), and channel protein
specificity. Parallels K-NOME methodology.

Quality evolution: static service registry (zero allocation, no `Box::leak`),
typed `ShadowMode` enum (replaced stringly-typed), capability-derived boundary
policies (layers declare bonds, policies assemble from capabilities), all clippy
warnings resolved, `default_true` deduplicated, tests smart-refactored by domain.

**Handoff:** `infra/wateringHole/handoffs/archive/CELLMEMBRANE_FORMALIZATION_WAVE50_MAY26_2026.md`

**Deliverables:**
- [x] `specs/CELLMEMBRANE_ARCHITECTURE.md` — 3-channel model, crypto layers, firewall policy, K-Derm section
- [x] `specs/MEMBRANE_COMPOSITION_MODEL.md` — relay → rustdesk → tower → nest ladder
- [x] `specs/FIELDMOUSE_CONTRACT.md` — third-party deployment contract
- [x] `specs/MULTI_MEMBRANE_DEPLOYMENT.md` — provider abstraction, multi-region
- [x] `specs/K_DERM_TOPOLOGY.md` — monoderm/diderm, periplasm, bonding, channel proteins, vesicle transport
- [x] `crates/cellmembrane-types/` — Rust types, serde, validation (160 tests, 8 modules, 95.8% coverage)
- [x] `membrane.toml` — reference config for live VPS deployment
- [x] Gap closure: 5 Dark Forest audit gaps closed in types
- [x] Debt resolution: static registry, typed ShadowMode, capability-based derivation, clippy-clean

### benchScale + agentReagents Ownership Convergence (Wave 51)

Mature Rust repos (benchScale ~22k LOC, agentReagents ~7.9k LOC) moved from
`sort-after/` to canonical `infra/` locations. Slim bash scaffold predecessors
archived to `infra/*-slim-archive/`. Both repos aligned on dependencies
(`thiserror` 2.0, `serde_yaml` 0.10, `clap` 4.5), postPrimordial compliance
enforced (binary resolution from `plasmidBin`, not local builds).

K-Derm diderm topology wired as benchScale YAML (`topologies/nucleus/kderm_diderm_membrane.yaml`)
with 5 nodes, boundary crossing validation, and a parsing test.

**Deliverables:**
- [x] benchScale + agentReagents converged to `infra/` (308 + 113 tests pass)
- [x] Dependency alignment across both repos
- [x] postPrimordial compliance: binary deploy paths, `PLASMID_BIN` env, `fetch.sh` fallback
- [x] K-Derm diderm topology YAML + parsing test
- [x] Archive cleanup: `archive/`, `scripts/legacy/`, `templates/archive/` removed from working tree (preserved in git history)

### Deep Debt Sprint — All 3 Repos (Wave 51)

Systematic deep debt resolution targeting modern idiomatic Rust across all
owned repos. Audit identified and resolved:

**cellMembrane:** `FirewallRule.comment` String → `&'static str` (zero allocation),
supplementary ports in service registry (hbbs 21115, caddy 80), output-only types
drop `Deserialize`, `push_port_rules()` helper eliminates repetition.

**agentReagents:** 10 `println!` → structured `tracing::info!`, `PciAttachMode`
enum replaces stringly-typed attach mode, unused imports cleaned, 26 `missing_docs`
warnings resolved on verification types.

**benchScale:** `constants::deploy` + `constants::libvirt_defaults` modules with
env var discovery (`BENCHSCALE_DEPLOY_DIR`, `BENCHSCALE_LIBVIRT_NETWORK`), all
hardcoded `/opt/biomeos/bin` and `"default"` network references centralized,
9 `println!` → tracing, unsafe FFI `dhcp_leases.rs` evolved to safe `Option<&CStr>`
API, call sites no longer touch raw pointers.

**Handoff:** `infra/wateringHole/handoffs/archive/CELLMEMBRANE_DEEP_DEBT_WAVE51_MAY26_2026.md`

### Wave 56: VPS Deployment Standard Absorption

primalSpring shipped the VPS deployment standard (Waves 55b–56): `nucleus_launcher --uds-only`,
cell graph `vps_standard` tagging, env var centralization, 12 primordial script archival.

**cellMembrane response:**
- [x] `TransportMode` enum (`UdsOnly`, `TcpDefault`, `TcpOptIn`) added to service registry
- [x] All NUCLEUS primals marked `vps_transport: TransportMode::UdsOnly`
- [x] `HealthCheckMethod::SocketExists` added for UDS socket file existence checks
- [x] `deploy_membrane.sh` → `--uds-only` flag for nucleus composition
- [x] `deploy_membrane.sh` → `spring-overlay` mode for cell graph deployment
- [x] `membrane.toml` → `transport = "uds_only"`
- [x] `CompositionSpec::uds_socket_paths()` and `tcp_ports_uds_mode()` helpers
- [x] Stale Channel 1/3 `[future]` comments fixed to `[ACTIVE]` in deploy script
- [x] VPS deployment standard documented in RUNBOOKS, VPS_STATE
- [ ] `nucleus_launcher` binary available in plasmidBin releases (upstream dependency)
- [ ] Test `biomeos deploy` with live cell graph against VPS NUCLEUS (operational)

### NC-3.2: K-Derm Boundary Publication (Wave 55)

`membrane.toml` updated from `composition = "tower"` to `composition = "nest"`, signal
channel enabled (knot-dns :53 with DNSSEC). K-Derm diderm boundary now published.
primalSpring `s_kderm_boundary` live validation can activate against this config.

**Deliverables:**
- [x] `membrane.toml` → `composition = "nest"`, `topology = "diderm"`
- [x] Signal channel enabled: `knot-dns` :53, `dnssec = true`
- [x] Integration tests updated: `parse_reference_membrane_toml` expects `Nest`, signal `enabled = true`
- [x] 93/93 tests pass, 0 clippy warnings

### NC-3.4: Forgejo Releases (Criteria #6 enabler)

Sovereign binary distribution channel alongside GitHub Releases. plasmidBin `auto-harvest.yml`
updated with Forgejo support (Wave 55). `provenance.toml` Layer 2 records forge identity.

**Status:** plasmidBin shipped Forgejo hooks — coordination with cellMembrane Forgejo instance for first sovereign release pending.

### NC-3.5: sporePrint Living Content

Sovereign content hosting via NestGate `content.put`. Blocked on BearDog `auth.issue_session`
scope expansion for `content.*`. When unblocked: `publish_sporeprint.sh` → NestGate → sovereign content.

**Status:** BLOCKED on BearDog scope expansion.

### Multi-Gate LAN Mesh (Criteria #2, #4)

Gate teams deploying NUCLEUS compositions on LAN. cellMembrane provides TURN rendezvous via Songbird :3478. At least one remote covalent node (flockGate) must validate over WAN for criterion #4.

**cellMembrane readiness:** TURN relay operational. 4 gates running as of Wave 50.
Forgejo inner membrane mirror healthy (25 native pull mirrors + 6 timer-synced, all current).

---

## Sovereignty Shadow Cutover Progress

| Track | Sovereign | Shadow | 7-Day Gate | Status |
|-------|-----------|--------|------------|--------|
| S1 TLS | BearDog :8443 | Cloudflare | p95 ≤ 1.5× commercial | Shadow live, **NOT cut over** |
| S2 NAT | Songbird :3478 | cloudflared | 100% reachable | **LIVE — tracking 7-day window** |
| S3 Content | NestGate + petalTongue | GitHub Pages | TTFB parity | **LIVE — 68ms vs 89ms** |
| S4 Auth | BearDog BTSP | OAuth2/PAM | p95 < 50ms | Ready, **incomplete** |

**Cutover sequence:** S2 → S3 → S4 → S1 (S1 last because it requires Cloudflare removal)

---

## Dark Forest Compliance

All deployments must satisfy five pillars before stadial entry.

| Pillar | Requirement | Current Status |
|--------|-------------|----------------|
| 1. Zero metadata leakage | Stripped binaries, no hostnames embedded | PASS |
| 2. Zero port exposure | UDS default, TCP opt-in, composition-aware UFW | PASS |
| 3. Songbird sole network surface | All external traffic through Songbird | PASS |
| 4. BTSP crypto integrity | 13/13 primals, ChaCha20-Poly1305 | PASS |
| 5. Enclave computing | Dual-tower ionic pattern | PASS (Tower + Nest Atomic) |
