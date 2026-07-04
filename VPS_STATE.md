# VPS State Snapshot

**Last updated:** 2026-07-04 (Wave 132d)
**Deployed composition:** Full NUCLEUS (Wave 61→118) — 13 primals + 4 symbiotic + federation + WAN depot + sandbox/canary pipeline
**VPS transport:** UDS + federation TCP :7700 — NUCLEUS primals on Unix domain sockets, Songbird federation on TCP for cross-gate mesh
**VPS workspace:** `/opt/ecoPrimals/` — 17 repos cloned from sovereign Forgejo, cascade via Rust `membrane` binary
**Deployment model:** Deterministic — `gate.bootstrap` (7 phases) + sandbox validation + atomic promote + canary retirement + cascade auto-fetch
**WAN depot:** `https://membrane.primals.eco/depot/` — 13 binaries + checksums.toml over HTTPS (zero SSH for WAN gates)
**K-Derm topology:** Diderm (gate firewall = plasma membrane, VPS = periplasm + outer membrane)
**Auth:** BTSP-only enforced (`BEARDOG_AUTH_MODE=enforced` since 2026-06-02, S4 GRADUATED)
**Mesh:** 5-node WireGuard (golgi 10.13.37.1, sporeGate .2, eastGate .5, flockGate .6, ironGate .7) + songbird:7700 federation
**Disk:** 60% (cleaned Wave 69)
**Caddy:** sovereign TLS on `membrane.primals.eco` (Let's Encrypt E8) + `/depot/` file_server
**Sandbox:** `/opt/membrane/sandbox/` + `/run/membrane/sandbox/` — ephemeral UDS validation (systemd `membrane-sandbox@`)
**Canary:** `/opt/membrane/canary/` + `/run/membrane/canary/` — previous-good fallback pool (systemd `membrane-canary@`)

---

## Infrastructure

| Attribute | Value |
|-----------|-------|
| Provider | DigitalOcean |
| Region | nyc1 |
| Size | s-1vcpu-2gb ($12/mo) |
| OS | Debian 12 x64 |
| IP | $VPS_IP |
| Hostname | membrane-relay |
| SSH user | root |

---

## Running Services (22 services, 13 primals + Forgejo + federation)

### Tower Tier (identity + relay + federation + audit)

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| BearDog | `beardog-membrane` | ACTIVE | `/run/membrane/beardog.sock` | v0.9.0 |
| BearDog TLS shadow | `beardog-tls-shadow` | ACTIVE | :8443 | v0.9.0 |
| **Songbird Federation** | `songbird-membrane` | **ACTIVE** | `/run/membrane/songbird.sock` + **:7700** | v0.2.1 |
| Songbird TURN | `songbird-relay` | ACTIVE | :3478 tcp/udp | v0.2.1 |
| SkunkBat | `skunkbat-membrane` | ACTIVE | 127.0.0.1:9140 | — |

### Node Tier (compute) — NEW Wave 59

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| toadStool | `toadstool-membrane` | ACTIVE | `/tmp/biomeos/compute-tarpc.sock` | v0.2.0 |
| barraCuda | `barracuda-membrane` | ACTIVE | `/run/membrane/barracuda.sock` | v0.4.0 |
| coralReef | `coralreef-membrane` | ACTIVE | `/run/membrane/coralreef.sock` | v0.2.0 |

### Nest Tier (provenance)

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| NestGate | `nestgate-membrane` | ACTIVE | `/run/membrane/nestgate.sock` + :9500 | v2.1.0 |
| rhizoCrypt | `rhizocrypt-membrane` | ACTIVE | `/run/membrane/rhizocrypt.sock` + :9602 | v0.14.0 |
| loamSpine | `loamspine-membrane` | ACTIVE | `/run/membrane/loamspine.sock` + :9700 | v0.9.16 |
| sweetGrass | `sweetgrass-membrane` | ACTIVE | `/run/membrane/sweetgrass.sock` + :9850 | v0.7.34 |

### Meta Tier (orchestration) — NEW Wave 59

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| biomeOS | `biomeos-membrane` | ACTIVE | `/run/membrane/biomeos.sock` | v0.1.0 |
| squirrel | `squirrel-membrane` | ACTIVE | `/run/membrane/squirrel.sock` | v0.1.0 |
| petalTongue | `petaltongue-membrane` | ACTIVE | `/run/membrane/petaltongue.sock` | v1.6.6 |

### Symbiotic (non-ecoPrimal)

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| RustDesk hbbs | `hbbs-membrane` | ACTIVE | :21115-21116 | — |
| RustDesk hbbr | `hbbr-membrane` | ACTIVE | :21117 | — |
| Caddy | `caddy-tls` | ACTIVE | :80/:443 | — |
| fail2ban | `fail2ban` | ACTIVE | — | — |
| knot-dns | `knot` | ACTIVE | :53 tcp/udp | DNSSEC enabled |
| Forgejo | `forgejo` | ACTIVE | 127.0.0.1:3000 + :2222 | v15.0.2 (golgiBody Phase A) |

### Capability Symlinks (auto-created at runtime)

| Symlink | Target | Capability |
|---------|--------|------------|
| `/run/membrane/btsp.sock` | beardog.sock | BTSP identity |
| `/run/membrane/crypto.sock` | beardog.sock | Cryptographic ops |
| `/run/membrane/security.sock` | beardog.sock | Security boundary |
| `/run/membrane/ed25519.sock` | beardog.sock | Ed25519 signing |
| `/run/membrane/x25519.sock` | beardog.sock | X25519 key exchange |
| `/run/membrane/ai.sock` | squirrel.sock | AI inference |
| `/run/membrane/visualization.sock` | petaltongue.sock | Visualization |

**Boot order (systemd):** BearDog → Songbird → SkunkBat → toadStool → barraCuda → coralReef → NestGate → rhizoCrypt → loamSpine → sweetGrass → biomeOS → squirrel → petalTongue

---

## Firewall (UFW)

| Rule | Reason |
|------|--------|
| 22/tcp ALLOW | SSH |
| 53/tcp ALLOW | knot-dns (Channel 1 Signal) |
| 53/udp ALLOW | knot-dns (Channel 1 Signal) |
| 80/tcp ALLOW | Caddy HTTP (ACME redirect) |
| 443/tcp ALLOW | Caddy HTTPS (Channel 3 Surface) |
| 3478/tcp ALLOW | Songbird TURN (Channel 2 Relay) |
| 3478/udp ALLOW | Songbird TURN (Channel 2 Relay) |
| 7700/tcp ALLOW | Songbird Federation (Channel 2b Mesh Hub) |
| 2222/tcp ALLOW | Forgejo SSH git (golgiBody) |
| 8443/tcp ALLOW | BearDog TLS shadow |
| 9500/tcp ALLOW | NestGate |
| 9602/tcp ALLOW | rhizoCrypt JSON-RPC |
| 9700/tcp ALLOW | loamSpine |
| 9850/tcp ALLOW | sweetGrass |
| 21115/tcp ALLOW | RustDesk NAT type test |
| 21116/tcp ALLOW | RustDesk ID registration |
| 21116/udp ALLOW | RustDesk hole punching |
| 21117/tcp ALLOW | RustDesk relay |
| 49152:65535/udp ALLOW | Songbird TURN relay data (ephemeral media channels) |

**Default policy:** deny incoming, allow outgoing

---

## Filesystem Layout

| Path | Contents |
|------|----------|
| `/opt/membrane/beardog` | BearDog binary (stripped static ELF) |
| `/opt/membrane/songbird` | Songbird binary |
| `/opt/membrane/skunkbat` | SkunkBat binary |
| `/opt/membrane/nestgate` | NestGate binary |
| `/opt/membrane/rhizocrypt` | rhizoCrypt binary |
| `/opt/membrane/loamspine` | loamSpine binary |
| `/opt/membrane/sweetgrass` | sweetGrass binary |
| `/opt/membrane/hbbs` | RustDesk ID server binary |
| `/opt/membrane/hbbr` | RustDesk relay binary |
| `/opt/membrane/tower.env` | Tower environment (FAMILY_ID, BEARDOG_FAMILY_SEED, MEMBRANE_ROLE) |
| `/opt/membrane/credentials.age` | Age-encrypted credential blob |
| `/opt/membrane/rustdesk/` | RustDesk keys and working directory |
| `/etc/songbird/relay-credentials` | TURN credentials (nucleus-relay:<hex>) |
| `/etc/membrane/` | Tower configuration |
| `/etc/membrane/Caddyfile` | Channel 3 Caddy config (SSOT: `plasmidBin/membrane/Caddyfile`) |
| `/etc/membrane/family/` | MitoBeacon seeds: `.beacon.seed`, `family.key`, `nodes/*.lineage.seed` |
| `/opt/ecoPrimals/` | Standard ecoPrimals workspace (17 repos from Forgejo) |
| `/opt/ecoPrimals/infra/plasmidBin/` | Deployment tooling (fetch.sh, nucleus_launcher.sh, start_primal.sh) |
| `/opt/ecoPrimals/infra/wateringHole/` | Ecosystem standards + membrane temporal.cascade |
| `/opt/ecoPrimals/primals/` | 13 primal source repos (cloned from sovereign Forgejo) |
| `/opt/ecoPrimals/gardens/` | cellMembrane + projectNUCLEUS |
| `/opt/membrane/nucleus_launcher.sh` | Symlink → plasmidBin launcher |
| `/opt/membrane/start_primal.sh` | Symlink → plasmidBin start script |
| `/opt/membrane/fetch.sh` | Symlink → plasmidBin binary fetch |
| `/run/membrane/` | Unix domain sockets (BearDog, Songbird, SkunkBat) |
| `/var/cache/membrane/nestgate/` | sporePrint content cache (19 MB synced from NestGate) |
| `/var/cache/membrane/lab/` | Static lab page root (intra layer — ecosystem dashboard) |
| `/var/lib/membrane/nestgate` | NestGate data directory |
| `/var/lib/membrane/loamspine` | loamSpine data directory |

---

## TLS / Certificate State — S1 OPERATIONAL

| Attribute | Value |
|-----------|-------|
| **Shadow status** | **OPERATIONAL** (13+ days clean, 7-day gate passed 2026-06-01) |
| CA | Let's Encrypt |
| Certificate chain | E8 intermediate |
| Domain | `membrane.primals.eco` |
| Renewal | Caddy automatic (ACME) |
| TTFB (measured) | 68ms sovereign vs 89ms GitHub Pages |
| Cutover note | Cloudflare TLS INACTIVE; Caddy + LE is sole TLS provider |

---

## Bonding / Trust (K-Derm Envelope Model)

| Bond Type | Channel Protein | Layer | Description |
|-----------|----------------|-------|-------------|
| Covalent | Aquaporin | Plasma membrane | SSH key access from gates → VPS (always-open, shared family seed) |
| Metallic | Aquaporin | Plasma membrane | Fleet compute coordination between gates |
| Ionic | Gated Ion | Periplasm / Outer membrane | BTSP scoped tokens for external services + provenance trio |
| Weak | Passive Diffusion | Extracellular | Read-only public API (no active transport) |

See `specs/K_DERM_TOPOLOGY.md` for the full cell envelope model.

---

## Validation Results

| Check | Result | Date |
|-------|--------|------|
| NUCLEUS deploy (13 primals) | ALL ACTIVE, UDS sockets healthy | 2026-05-28 |
| biomeOS health.liveness | healthy (JSON-RPC over UDS) | 2026-05-28 |
| Spring overlay graph validation | 14 nodes parsed, validated | 2026-05-28 |
| Dark Forest audit (Nest) | 21 PASS, 0 FAIL, 1 SKIP (MEM-09 b3sum) | 2026-05-22 |
| Provenance trio pipeline | 10/10 PASS | 2026-05-22 |
| Shadow orchestrator | 6/6 PASS | 2026-05-22 |
| `deploy_membrane.sh status` | All 19 services RUNNING | 2026-05-28 |
| `membrane temporal.cascade` | 17/17 repos synced from sovereign Forgejo | 2026-06-02 |
| `nucleus_launcher.sh --seed-only` | 13/13 primals registered in Songbird | 2026-05-29 |
| `benchScale vps-depot-lab` | 26/26 PASS — 7-node topology, 5 compositions validated | 2026-05-29 |
| `onboard-gate-relay.sh --dry-run` | Relay env generation validated | 2026-05-29 |
| `cargo test` (cellMembrane workspace) | 917 PASS, 0 FAIL, 0 clippy | 2026-07-04 |
| `cargo test` (benchScale) | 272 PASS, 0 FAIL | 2026-05-27 |
| `cargo test` (agentReagents) | 94 PASS, 0 FAIL | 2026-05-27 |

**Pending re-validation (NUCLEUS):** Dark Forest audit with 13 primals, provenance pipeline with full NUCLEUS.

---

## DNS (Channel 1 Signal)

| Attribute | Value |
|-----------|-------|
| Server | knot-dns |
| Zone | `primals.eco` |
| DNSSEC | Enabled |
| Status | Running on VPS, NS cutover to primary pending (registrar action) |

---

## VPS Deployment Standard (Wave 56)

Three-step deployment model from primalSpring coordination:

```
Step 1: deploy_membrane.sh deploy root@$VPS_IP --composition nucleus --uds-only
        → NUCLEUS base (13 primals, UDS-only, zero TCP ports for primals)
        → Binaries from plasmidBin GitHub Releases
        → nucleus_launcher start --uds-only manages all 13 primals

Step 2: deploy_membrane.sh spring-overlay root@$VPS_IP --cell <spring>
        → Spring overlay via biomeos deploy (spawn=false on all nodes)
        → Only for VPS-standard springs (6 of 9 in manifest)

Step 3: Spring runtime discovers NUCLEUS via UDS
        → CompositionContext::from_live_discovery()
        → UDS tiers 2-4, no harness, no shell scripts
```

**VPS-ready springs:** hotspring, wetspring, neuralspring, airspring, groundspring, healthspring
**Desktop-only:** nucleus_desktop, ludospring, esotericwebb

### Artifacts consumed from primalSpring

| Artifact | Location | Purpose |
|----------|----------|---------|
| Spring cell graphs | `graphs/cells/{spring}_cell.toml` | Deploy topologies (6 VPS-ready, all `spawn=false`) |
| Cell manifest | `graphs/cells/cells_manifest.toml` | Index with `vps_standard` field |
| Launch profiles | `config/primal_launch_profiles.toml` | Per-primal CLI/env wiring |
| Seed fingerprints | `validation/seed_fingerprints.toml` | Crypto tier 0 bootstrap |

---

## Resource Notes

| Resource | Value | Threshold |
|----------|-------|-----------|
| Disk | 8.2G / 9.7G (89%) | Resize or prune needed before fieldMouse evolution |
| RAM | 517M / 1.9G used | Healthy for current composition |

---

## Update Procedure

After any VPS change, update this file:

```bash
# Verify current state
ssh root@$VPS_IP "systemctl list-units 'beardog*' 'songbird*' 'skunkbat*' 'hbbs*' 'hbbr*' 'caddy*' 'nestgate*' 'rhizocrypt*' 'loamspine*' 'sweetgrass*' 'knot*' --no-pager"
ssh root@$VPS_IP "ufw status numbered"

# Then edit VPS_STATE.md with changes
```
