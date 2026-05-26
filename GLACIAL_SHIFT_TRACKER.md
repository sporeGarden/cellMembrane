# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-05-26
**Overall status:** BLOCKED — 2 direct blockers owned by cellMembrane
**Wave 51 update:** Deep debt sprint across all 3 owned repos (cellMembrane, benchScale, agentReagents).
benchScale + agentReagents converged from `sort-after/` into canonical `infra/` locations.
K-Derm diderm topology wired into benchScale. postPrimordial compliance enforced.
**Wave 50 update:** GitHub Actions incident (May 26) proved external CI is unacceptable.
Self-hosted runner deployed on ironGate. plasmidbin validate 98/98 PASS locally.
CI runs still failing on GitHub-hosted runners (can't download action archives).
Our self-hosted runner has Rust toolchain installed locally — zero download dependency.

---

## Stadial Entry Criteria

All six criteria must be satisfied before glacial shift.

| # | Criterion | Status | cellMembrane Role | Blocker Owner |
|---|-----------|--------|-------------------|---------------|
| 1 | All 4 sovereignty shadows cut over (7-day gates) | S2 LIVE, S3 LIVE, S1/S4 incomplete | Operate shadow infrastructure | Shared |
| 2 | Multi-gate LAN mesh operational (3+ gates in Plasmodium) | **4 gates running** (Wave 50), mesh seeded | Provide TURN rendezvous | Gate teams |
| 3 | **Nest expansion deployed on VPS** | **NOT DEPLOYED** | **Direct blocker — we deploy** | **cellMembrane** |
| 4 | Remote covalent node (flockGate) validated over WAN | flockGate not deployed | Provide WAN rendezvous | Shared |
| 5 | **DNS pointed to sovereign infrastructure** | **Channel 1 NOT deployed** | **Direct blocker — we deploy knot-dns** | **cellMembrane** |
| 6 | Cloudflare/cloudflared removed from production path | S1 not cut over | Caddy → BearDog ACME cutover | Shared |

---

## Direct Blockers — cellMembrane Action Items

### Blocker 1: Nest Expansion on VPS (Criterion #3)

**What:** Deploy `--composition nest` on the live VPS, adding NestGate, rhizoCrypt, loamSpine, sweetGrass alongside existing Tower.

**Command:**
```bash
cd ../../infra/plasmidBin
./deploy_membrane.sh deploy root@157.230.3.183 --composition nest --validate
```

**New services:**
| Service | Port | Data Directory |
|---------|------|----------------|
| nestgate-membrane | :9500 | /var/lib/membrane/nestgate |
| rhizocrypt-membrane | :9601 | — |
| loamspine-membrane | :9700 | /var/lib/membrane/loamspine |
| sweetgrass-membrane | :9850 | — |

**New UFW rules needed:** 9500/tcp, 9601/tcp, 9700/tcp, 9850/tcp

**Pre-flight checklist:**
- [ ] Confirm Tower services healthy (`deploy_membrane.sh status`)
- [ ] Verify primal binaries available on GitHub Releases
- [ ] Ensure VPS has sufficient disk/memory (2GB VM, nest adds 4 more services)
- [ ] Review `deploy_membrane.sh` nest validation gap (UFW check doesn't cover nest ports)

**Post-deploy validation:**
- [ ] All 4 nest services `systemctl is-active`
- [ ] UFW shows nest ports open
- [ ] NestGate reachable on :9500
- [ ] `deploy_membrane.sh status` reports nest composition

---

### Blocker 2: Sovereign DNS — Channel 1 Signal (Criterion #5)

**What:** Stand up knot-dns as secondary DNS on VPS for `primals.eco`, then primary cutover.

**Status:** No deployment logic exists in `deploy_membrane.sh` for Channel 1. This requires:
1. knot-dns package installation on VPS
2. Zone configuration for `primals.eco`
3. UFW port :53 tcp/udp open
4. Registrar NS record update (secondary → primary cutover)

**Pre-flight checklist:**
- [ ] Design knot-dns zone file for `primals.eco`
- [ ] Coordinate with registrar for NS delegation
- [ ] Test as secondary before primary cutover
- [ ] Validate with `dig @157.230.3.183 primals.eco`

**Dependencies:**
- ICANN registrar cooperation (permanently external)
- Current commercial DNS must remain available during transition

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

**Handoff:** `infra/wateringHole/handoffs/CELLMEMBRANE_SELF_HOSTED_RUNNERS_WAVE50_MAY26_2026.md`

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
validation — **80 tests** across 6 domain test modules, **zero clippy warnings**.

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

**Handoff:** `infra/wateringHole/handoffs/CELLMEMBRANE_FORMALIZATION_WAVE50_MAY26_2026.md`

**Deliverables:**
- [x] `specs/CELLMEMBRANE_ARCHITECTURE.md` — 3-channel model, crypto layers, firewall policy, K-Derm section
- [x] `specs/MEMBRANE_COMPOSITION_MODEL.md` — relay → rustdesk → tower → nest ladder
- [x] `specs/FIELDMOUSE_CONTRACT.md` — third-party deployment contract
- [x] `specs/MULTI_MEMBRANE_DEPLOYMENT.md` — provider abstraction, multi-region
- [x] `specs/K_DERM_TOPOLOGY.md` — monoderm/diderm, periplasm, bonding, channel proteins, vesicle transport
- [x] `crates/cellmembrane-types/` — Rust types, serde, validation (80 tests, 6 modules)
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

**Handoff:** `infra/wateringHole/handoffs/CELLMEMBRANE_DEEP_DEBT_WAVE51_MAY26_2026.md`

### S5 Forgejo Releases (Criteria #6 enabler)

Sovereign binary distribution channel replacing GitHub Releases. Currently `deploy_membrane.sh` fetches from `https://github.com/ecoPrimals/plasmidBin/releases/`. Coordinate with projectNUCLEUS on Forgejo `auto-harvest.yml` integration.

**Status:** Not started — depends on Forgejo operational stability + self-hosted runners

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
| 5. Enclave computing | Dual-tower ionic pattern | PASS (Tower), pending (Nest) |
