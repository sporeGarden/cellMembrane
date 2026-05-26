# Glacial Shift Tracker

**Purpose:** Track cellMembrane's progress toward stadial entry (glacial shift).
**Last updated:** 2026-05-26
**Overall status:** BLOCKED — 2 direct blockers owned by cellMembrane
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

### Self-Hosted GitHub Actions Runners (new — Wave 50)

GitHub Actions incident on May 26 proved external CI dependency is unacceptable.
Self-hosted runners on LAN gates eliminate this: free minutes, zero cloud dependency,
and the path to Forgejo CI sovereignty.

**Status:** ironGate runner **ONLINE** (v2.334.0). Registered to ecoPrimals/plasmidBin.
Rust 1.95, musl x86_64 + aarch64 cross-compilation verified. systemd service enabled.

**Handoff:** `infra/wateringHole/handoffs/CELLMEMBRANE_SELF_HOSTED_RUNNERS_WAVE50_MAY26_2026.md`

**Acceptance:**
- [x] ironGate runner online: `irongate-runner online self-hosted,Linux,X64,x86_64,irongate`
- [x] `plasmidbin validate .` passes on ironGate: **98/98 PASS**
- [x] Static musl binary builds: x86_64 (2.8MB) + aarch64 cross-compile verified
- [ ] 2nd runner online (eastGate or southGate — needs their gate)
- [ ] Manual workflow dispatch runs on self-hosted runner
- [ ] Failover: one runner offline, other picks up jobs

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
