# VPS State Snapshot

**Last updated:** 2026-05-28
**Deployed composition:** Nest Atomic (Wave 38, deployed 2026-05-22)
**VPS transport:** UDS-only (Wave 56 standard) — NUCLEUS primals on Unix domain sockets, zero TCP ports
**VPS_IP:** Set via `nucleus_config.sh` → `MEMBRANE_VPS_IP`. All `$VPS_IP` references below resolve from there.
**K-Derm topology:** Diderm (gate firewall = plasma membrane, VPS = periplasm + outer membrane)

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

## Running Services (14 services, 7 primals)

| Service | Unit Name | Status | Port / Socket | Version |
|---------|-----------|--------|---------------|---------|
| BearDog | `beardog-membrane` | ACTIVE | `/run/membrane/beardog.sock` | v0.9.0 |
| BearDog TLS shadow | `beardog-tls-shadow` | ACTIVE | :8443 | v0.9.0 |
| Songbird TURN | `songbird-relay` | ACTIVE | :3478 tcp/udp | v0.2.1 |
| SkunkBat | `skunkbat-membrane` | ACTIVE | 127.0.0.1:9140 | — |
| NestGate | `nestgate-membrane` | ACTIVE | :9500 | v2.1.0 |
| rhizoCrypt | `rhizocrypt-membrane` | ACTIVE | :9602 | v0.14.0 |
| loamSpine | `loamspine-membrane` | ACTIVE | :9700 | v0.9.16 |
| sweetGrass | `sweetgrass-membrane` | ACTIVE | :9850 | v0.7.34 |
| RustDesk hbbs | `hbbs-membrane` | ACTIVE | :21115-21116 | — |
| RustDesk hbbr | `hbbr-membrane` | ACTIVE | :21117 | — |
| Caddy | `caddy-tls` | ACTIVE | :80/:443 | — |
| petalTongue | `petaltongue-web` | ACTIVE | :8080 | — |
| fail2ban | `fail2ban` | ACTIVE | — | — |
| knot-dns | `knot` | ACTIVE | :53 tcp/udp | DNSSEC enabled |

**Boot order constraint (systemd):** BearDog → Songbird → SkunkBat → NestGate → rhizoCrypt → loamSpine → sweetGrass

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
| 8443/tcp ALLOW | BearDog TLS shadow |
| 9500/tcp ALLOW | NestGate |
| 9602/tcp ALLOW | rhizoCrypt JSON-RPC |
| 9700/tcp ALLOW | loamSpine |
| 9850/tcp ALLOW | sweetGrass |
| 21115/tcp ALLOW | RustDesk NAT type test |
| 21116/tcp ALLOW | RustDesk ID registration |
| 21116/udp ALLOW | RustDesk hole punching |
| 21117/tcp ALLOW | RustDesk relay |

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
| `/run/membrane/` | Unix domain sockets (BearDog, SkunkBat) |
| `/var/cache/membrane/nestgate/` | sporePrint content cache (19 MB synced from NestGate) |
| `/var/lib/membrane/nestgate` | NestGate data directory |
| `/var/lib/membrane/loamspine` | loamSpine data directory |

---

## TLS / Certificate State

| Attribute | Value |
|-----------|-------|
| CA | Let's Encrypt |
| Certificate chain | E8 intermediate |
| Domain | `membrane.primals.eco` |
| Renewal | Caddy automatic (ACME) |
| TTFB (measured) | 68ms sovereign vs 89ms GitHub Pages |

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
| Dark Forest audit | 21 PASS, 0 FAIL, 1 SKIP (MEM-09 b3sum) | 2026-05-22 |
| Provenance trio pipeline | 10/10 PASS | 2026-05-22 |
| Shadow orchestrator | 6/6 PASS | 2026-05-22 |
| `deploy_membrane.sh status` | All 11 services RUNNING | 2026-05-22 |
| `cargo test` (cellmembrane-types) | 160 PASS, 0 FAIL, 0 clippy warnings (95.8% coverage) | 2026-05-28 |
| `cargo test` (benchScale) | 272 PASS, 0 FAIL | 2026-05-27 |
| `cargo test` (agentReagents) | 94 PASS, 0 FAIL | 2026-05-27 |

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

## Update Procedure

After any VPS change, update this file:

```bash
# Verify current state
ssh root@$VPS_IP "systemctl list-units 'beardog*' 'songbird*' 'skunkbat*' 'hbbs*' 'hbbr*' 'caddy*' 'nestgate*' 'rhizocrypt*' 'loamspine*' 'sweetgrass*' 'knot*' --no-pager"
ssh root@$VPS_IP "ufw status numbered"

# Then edit VPS_STATE.md with changes
```
