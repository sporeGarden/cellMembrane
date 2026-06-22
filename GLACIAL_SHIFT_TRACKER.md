# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-06-22 (Wave 123+)
**Overall status:** STADIAL-READY — Zero P1, S1-S4 GRADUATED, 5-node WG mesh, deterministic deployment CODIFIED
**Wave 123+ update (Deep Debt — Typed RPC Errors + Visibility + Smart Refactors):**
`ShadowError::Rpc` typed variant replaces all `Result<_, String>` in JSON-RPC transport
(7 async fns, 5 caller sites). Visibility tightened: 15 functions `pub` → `pub(crate)` across
cli, topology, freshness, resolve modules. `manifest.rs` smart refactored (780L → 706L mod.rs
+ 142L wave.rs). `topology.endpoint <role>` single-arg dispatch shortcut. `ironGate` added to
`BOOTSTRAP_GATES` (5-node bootstrap complete). Webhook test coverage (+11), dispatch capability
tests (+6), WaveState lifecycle tests (+5). 791 tests, zero clippy/doc warnings.
**Wave 123 update (Deep Evolution — Wire Format + Sovereignty + Quorum):**
`ServiceCapability::wire_name()` fixes mesh relay routing bug (Debug format → serde snake_case).
`parse_verify_response()` pure function for sovereignty ledger with 7-branch test coverage.
TCP transport graduated — `call_tcp()` riboCipher-framed over WireGuard mesh. Quorum Phase 1:
`gate.quorum` installs systemd cascade timer. Role-to-capability mapping consolidated.
Error variant semantic fix (`Ssh` → `Parse` for UDS failures). Fragile string detection →
structured JSON field check. 769 tests, zero clippy/doc warnings.
**Wave 121 update (Transport Evolution + Dual-Target Depot):** `TargetArch` enum typed
target triples. Dual-target depot (musl + gnu for GPU primals). PAT deprecated. Transport
endpoint resolver: `(gate, capability)` → `Uds|Tcp|MeshRelay`. `call_via_relay()` routes
through songBird. `topology.endpoint` CLI. `MeshRelay` variant graduated to operational.
Webhook `classify_push` bug fix (harvest vs cascade precedence). 5-node mesh (ironGate .7).
751 tests, zero warnings.
**Wave 120 update (Deployment Isomorphism — Identity-Based Resolution):** `topology.service`
identity-based service discovery. Manifest-authoritative `wg_ip` + `roles` on `GateProfile`.
`wireguard.generate` and `caddy.generate` produce configs from manifest. `topology.roles` +
`topology.mesh` manifest-aware. `gate.validate` generic composition trust barrier validation
(evolved from `pepti.validate`). `wireguard.*` dispatch routing fixed. pepti decommissioned.
Dependency evolution: `toml` 0.8→1.x, `nix` 0.29→0.31. Manifest-first federation
peer resolution. `to_nftables_script` chain helpers. Deep debt: `.leak()` memory debt
eliminated (owned `String` gate identity), `HEALTH_REQUEST` const centralized, corrupt
TOML parse now warns instead of silently resetting, `CanaryStalenessReport` disambiguation,
`FirewallProtocol` derives `Ord`, mesh response parsing deduped. Build/harvest pipeline
unified: `clone_source`, `stage_to_depot`, `get_head` deduped into single implementations.
Service port constants added (`DEFAULT_FORGEJO_HTTP_PORT`, `DEFAULT_DEPOT_HTTP_PORT`).
BLAKE3 hash failure uses sentinel instead of empty string. 731 tests, zero clippy.
**Wave 119+ update (Native Detection + Error Normalization):** Shell-outs evolved to native
Rust: `ss` → `/proc/net/{tcp,udp}`, `ip link/addr` → sysfs + `/proc/net/route`, `systemctl
is-active` → cgroup detection. `ShadowError::Parse` normalized to `Config`/`Ssh`/`Io` across
29 files (22 genuine `Parse` remain). `.expect()` → `let-else + unreachable!()` in ribocipher.
`PLASMID_BIN_DIR` constant eliminates 8 hardcoded literals. 711 tests, zero clippy.
**Wave 116–118 update (Deep Debt + Topology Convergence):** Webhook cascade wiring,
rootpulse sovereignty pipeline, SSH/git_ops consolidation, manifest-driven cascade repos,
identity unification, hardcoded path constants. 680 tests. Fixed P0 topology dispatch
collision. 620 tests → 680 → 711 → 729.
**Wave 116 update (Gate Enrollment Pipeline):** Fresh binary rebuilt from `11a7c68` with
ARP probe fix (uses detected LAN interface, not loopback). `InterfaceRole` Display impl
for clean preflight output. Gate enrollment pipeline validated: `gate.preflight`,
`firewall.generate --plasma-membrane`, `gate.bootstrap --dry-run` all operational.
eastGate enrollment BLOCKED on SSH key authorization (operator action). 498 tests.
**Wave 115 update (Sovereign Mesh & Gate Hardening):** `gate.bootstrap` per-phase
timeouts (120s) + `identity.git` phase (detects missing git config + SSH keys).
`depot.integrity` command (generate/verify BLAKE3 checksums). Smart refactor:
`bootstrap.rs` 861L → 555L via `gate/nucleus.rs` + `gate/mesh.rs`. All sync phases
evolved to `spawn_blocking`. Zero `as` casts, zero `.expect()` in production.
`option_if_let_else` promoted to warn. SSH user and Caddy endpoint env-driven.
55 new tests (416 → 471). All deps pure Rust (ring tracked in deny.toml).
**Wave 113 update (Deep Debt + Zero-Copy):** Arc manifest in cascade, Cow relay
defaults, safe casts (TryFrom), idiomatic error handling, SPDX headers, cargo-deny CI.
**Wave 111 update (riboCipher + Deep Debt):** riboCipher Transport Signal Standard
complete (mito-tier HKDF-SHA256 + HMAC tag generation/verification). All outbound UDS
connections prepend clear signal `[0xEC, 0x01]`. `dispatch/infra.rs` smart refactored
(762L → 264L remote VPS API + 518L gate.rs local self-management). Error propagation
modernized. Neural API constants shared via types crate. Freshness auto-publish
race-fix (wave-ID guard prevents stale overwrites). 391 tests, zero clippy.
**Wave 111 update (Gate Expansion):** `gate.bootstrap` sandbox integration. CASCADE-STALE-RECOVERY
(auto-stash + ff-only + pop). PARTIAL-FETCH-RESUME (atomic `.tmp` → rename, retry with backoff).
Pure Rust ELF validation. Hardcoded ports → named constants. 391 tests, zero clippy.
**Wave 110+ update:** Primal composition grade + sandbox/canary pipeline achieved.
`ServiceCapability` enum — capability-based service discovery replaces all hardcoded primal
names. `temporal/resolve.rs` extracted (authority-first push + agentic divergence resolution).
`plasmid/toolchain.rs` extracted (ELF validation + NDK). **Sandbox NUCLEUS**: ephemeral
isolated validation before binary promotion (spin-up → UDS JSON-RPC probe → teardown).
**Canary pool**: previous-good binaries retired to `/opt/membrane/canary/`, health-watched,
available as failover targets. Atomic blue/green promotion with canary retirement wired
into cascade-restart. `service/registry.rs` extracted (pure data, smart refactor).
All deployment paths env-configurable (`MEMBRANE_CONFIG_DIR`, `MEMBRANE_SOCKET_BASE`,
`VPS_MEMBRANE_BIN_DIR`, `MEMBRANE_SOVEREIGN_REMOTE`, `NM_DISPATCHER_DIR`).
DRY socket resolution (bootstrap reuses health's). Stream 5 `agentic_resolve` DONE. Zero
production unwrap/expect, zero TODO/FIXME/HACK, zero #[allow], zero unsafe (forbid), zero
mocks in prod, all deps pure Rust. 365 tests, zero clippy.
**Wave 110 update:** Deep debt evolution — native UDS probes, gate/ modular split,
dual-checksum verification, cascade-restart, agentic resolver, agnostic config.
Stream 2 (Build Pipeline) 6/6 DONE. northGate + westGate profiles registered.
**Wave 109 update:** guideStone convergence — `plasmid.build` (Rust build pipeline),
`gate.profile`, `deployment.toml` emission, JSON-RPC health probes, BUILD-ELF-01,
HARVEST-NAME-01, GATE-PROFILE-01. Three-tier context architecture shipped.
**Wave 107 update:** Post-stadial tooling evolution complete. `gate.status` (local health
probe), `gate.bootstrap --dry-run`, source divergence fix, checksum coherence detection,
WAN checksums (zero-git verification), atomic publish (harvest auto-commits checksums.toml).
Zero development debt: all files <800L. Remaining items are purely operational.
**Wave 106 update:** Cross-topology validation. gate.bootstrap SHIPPED + VALIDATED on
strandGate + ironGate. Cascade auto-fetch. NUCLEUS supervision (biomeOS v4.17). 3-gate
mesh collective (eastGate ↔ golgiBody ↔ ironGate). Deterministic deployment standard
codified (6 invariants). TCP-only fallback shipped. Zero P1.
**Wave 74 update:** ironGate **JOINED MESH** as 3rd plasmodium gate. BearDog + Songbird running locally
with SONGBIRD_PEERS pointing to eastGate + strandGate. `discovery.peers` shows both peers,
`mesh.health_check` all_healthy. `capability.call` cross-gate validated (Songbird fix `d6a6f714`
landed — TCP→HTTP POST for JSON-RPC). Deep debt sprint: all `#[allow]` eliminated from production,
`HardeningConfig` evolved to `HardeningStep` enum, `plasmid.rs::fetch()` decomposed. Zero clippy.
**Wave 73 update:** westGate onboarding prep (manifest entry, GATE_SETUP_STANDARD updated). ironGate
mesh join documented (SONGBIRD_PEERS config, capability symlinks, startup sequence, verification).
strandGate deploy graph upgraded to `strand_heavy_compute.toml` (10 primals). Live mesh validated
by eastGate (discovery.peers + mesh.health_check 2-gate PASS). `capability.call` cross-gate blocked
on Songbird remote dispatch fix (raw TCP → HTTP POST). ironGate = 3rd plasmodium gate after fix lands.
**Wave 71 update:** Legacy `cascade()` removed (all callers migrated to `cascade_with_opts`). New commands:
`relay.status`, `gate.health`, `content.verify`. S3 content cutover documented — VPS READY, awaiting
DNS flip only. 210 tests. Zero clippy (pedantic+nursery).
**Wave 69+ update:** Deep debt evolution — all `#[allow(clippy::too_many_lines)]` eliminated from codebase.
`plasmid.rs::fetch()` decomposed into staged pipeline. `temporal/mod.rs` extracted 4 helpers (sync_converge,
sync_diverge, resolve_tree_parity, count_divergent_remotes). `cascade.rs` evolved: `CascadeMode` enum
replaces 3 bools, `CascadeOpts` struct, extracted process_repo/clone_repo/check_repo/sync_repo.
`freshness.rs` dead_code wired (binary_blake3 + installed_at now in report). `FetchSource` Display impl.
NUCLEUS tests relocated from coverage.rs (903L→743L) to composition.rs (canonical home). All files <800L.
Zero clippy (pedantic+nursery). 209 tests.
**Wave 69 update:** Sovereignty graduation sprint — membrane binary deployed to VPS (6.1M musl static,
`/usr/local/bin/membrane`). Full K-Derm relay validated in Rust (relay.run, relay.mediate, relay.ship all
operational, bash scripts archived). S4 auth formal 7-day gate ACTIVATED (`BEARDOG_AUTH_MODE=enforced`,
monitoring timer at 15min intervals). Disk cleanup 69%→60% (old kernels, journals, apt lists, locales
removed). Workspace resolution evolved to recognize sparse VPS deployments (`infra/` marker). Relay ship
bug fixed (git remote get-url stdout leaking into variable). All primal binaries in `/opt/membrane/`
recovered and validated. S1 TLS infrastructure verified ready for NS cutover (registrar action pending).
Family seeds confirmed deployed (`/etc/membrane/family/`). Provenance sidecar written for membrane binary.
**Wave 68 update:** Graduated composition evolution — Neural Bridge wired into dispatch (try-primal-first
for gate.*, service.*, repo.*, mirror.*, token.* commands). gate.pull/check evolved to use Rust membrane
binary on VPS. PushResult struct replaces silent push failures. #[must_use] sweep (12 functions, 7 modules).
resolve_workspace_root() promoted to crate-level. forgejo_work_dir config chain. 209 tests. Zero clippy.
**Wave 67+ update:** Cascade evolution sprint — dispatch.rs split into 5 domain submodules (all <340L).
Tree-parity divergence auto-resolution (SyncAction::TreeParity). `--publish-freshness` wired into cascade.
`post_sync_diverge()` + graduated merge strategies (merge-ff, merge-rebase, impulse-only). Impulse ack
safety evolved (separate ack files prevent rebase loss). Binary freshness tracking (`--check-installed`).
All hardcoded paths evolved to capability-based discovery (ServicePaths, CredentialPaths). rsync eliminated
(SSH+cat). 207 tests. Zero clippy warnings. S1 TLS graduated OPERATIONAL (13+ days clean).
**Wave 66 update:** Deep debt evolution sprint — eliminated 3 external tool dependencies
(socat→native UnixStream, curl→reqwest, b3sum→blake3 crate). Removed deprecated signal.rs.
K-Derm relay chain fully in Rust (relay.rs: mediate + ship_extracellular + run). Real BLAKE3
checksums.toml verification. FromStr trait for FetchSource. Hardcoded paths parameterized
(relay SSH script, temporal clone URL uses manifest). 204 tests. Bash scripts archived
(forgejo_sync.sh, forgejo_pull_mirror.sh → superseded by membrane CLI).
**Wave 60 update:** golgiBody Phase A complete — VPS Forgejo live at `git.primals.eco`, 34 repos seeded,
eastGate WaterFall shadow validated (33/36 pull clean from sovereign Forgejo). VPS knot-dns zone updated
with `lab` and `git` A records. Caddyfile lab routes fixed (dead proxy → 503 + static file_server).
`cascade-pull.sh` fixed for non-default remote branch resolution. 2 repos need ironGate action
(rustChip seeding, toadStool branch rename). Cloudflare tunnel orphaned but kept for JupyterHub until BTSP relay.
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
| 1 | All 4 sovereignty shadows cut over (7-day gates) | S1 **OPERATIONAL** (13d clean), S2 LIVE, S3 LIVE, S4 formal gate pending | Operate shadow infrastructure | Shared |
| 2 | Multi-gate LAN mesh operational (3+ gates in Plasmodium) | **OPERATIONAL** — 3-gate plasmodium (eastGate + strandGate + ironGate), discovery.peers + mesh.health_check PASS | Provide TURN rendezvous | Gate teams |
| 3 | Nest expansion deployed on VPS | **LIVE** (Wave 38, 2026-05-22) — 21/0/1 darkforest, 10/10 trio | Operate Nest Atomic | **RESOLVED** |
| 4 | Remote covalent node (flockGate) validated over WAN | flockGate not deployed | Provide WAN rendezvous | Shared |
| 5 | DNS pointed to sovereign infrastructure | **knot-dns RUNNING** with DNSSEC — zone has lab/git/membrane A records, NS cutover pending (registrar) | Complete registrar NS record change | **cellMembrane** |
| 6 | Cloudflare/cloudflared removed from production path | S1 not cut over, tunnel orphaned (git route superseded by VPS Caddy) | Caddy → BearDog ACME cutover | Shared |

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

### ~~Blocker: Deploy biomeOS to VPS (P0)~~ — RESOLVED

**Deployed:** Wave 59 (2026-05-28). Full NUCLEUS composition (13 primals) running on VPS.
biomeOS v0.1.0 active, all primals healthy via UDS sockets. See VPS_STATE.md for live status.

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

**cellMembrane readiness:** TURN relay operational. **3-gate plasmodium mesh OPERATIONAL** (Wave 74):
eastGate + strandGate + ironGate. `discovery.peers` + `mesh.health_check` validated.
Forgejo inner membrane mirror healthy (25 native pull mirrors + 6 timer-synced, all current).

---

## Sovereignty Shadow Cutover Progress

| Track | Sovereign | Shadow | 7-Day Gate | Status |
|-------|-----------|--------|------------|--------|
| S1 TLS | Caddy + LE | Cloudflare (INACTIVE) | p95 ≤ 1.5× commercial | **OPERATIONAL** — 13+ days clean, Caddy sole TLS provider |
| S2 NAT | Songbird :3478 | cloudflared | 100% reachable | **LIVE — tracking 7-day window** |
| S3 Content | NestGate + petalTongue | GitHub Pages | TTFB parity | **VPS READY** — awaiting DNS flip (Caddyfile configured, 67ms TTFB) |
| S4 Auth | BearDog BTSP | OAuth2/PAM (disabled) | p95 < 50ms | **ENFORCED** — 7-day formal gate active |

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
