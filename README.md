# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (ironGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | Nest Atomic (Tower + NestGate + rhizoCrypt + loamSpine + sweetGrass) + RustDesk |
| **Escalation** | Phase 1.5 (Nest Atomic) — **current** (Wave 56, 2026-05-27) |

---

## Active Membrane Channels

| Channel | Function | Primal / Service | Port | Status |
|---------|----------|-----------------|------|--------|
| **2 Relay** | NAT traversal, TURN | Songbird | :3478 tcp/udp | **LIVE** |
| **2b RustDesk** | Sovereign remote desktop | hbbs + hbbr | :21115-21117 | **LIVE** |
| **3 Surface** | HTTPS, downloads, ACME, NestGate content | Caddy + NestGate | :80/:443/:9500 | **LIVE** — `membrane.primals.eco` (Let's Encrypt E8) |
| **1 Signal** | DNS resolution for `primals.eco` | knot-dns | :53 | **LIVE** — DNSSEC, NS cutover to primary pending |

### Channel 3 Surface Details

- Caddy reverse proxy with automatic Let's Encrypt TLS
- 19 MB sporePrint content cache synced from NestGate
- Sovereignty proof: 68ms TTFB (vs GitHub Pages 89ms)
- Domain: `membrane.primals.eco`

---

## What This Repo Is For

cellMembrane is both the **operational home** for the live membrane deployment
and the **typed specification** for sovereign membrane infrastructure that
others can deploy independently.

### Specifications (`specs/`)

Formal architecture for deployable membrane infrastructure:

| Spec | Purpose |
|------|---------|
| `CELLMEMBRANE_ARCHITECTURE.md` | 3-channel model, process isolation, crypto layers, firewall policy |
| `MEMBRANE_COMPOSITION_MODEL.md` | Composition ladder (relay → rustdesk → tower → nest) |
| `FIELDMOUSE_CONTRACT.md` | Deployment contract for third-party membrane operators |
| `MULTI_MEMBRANE_DEPLOYMENT.md` | Multi-provider, multi-region parameterization model |
| `K_DERM_TOPOLOGY.md` | K-Derm cell envelope model — monoderm/diderm, periplasm, bonding per layer |

### Rust Types (`crates/cellmembrane-types/`)

Typed domain models for membrane configuration, validation, and deployment:

```bash
cargo test                  # 93 tests — envelope, composition, channels, firewall, service, transport, config, validation
cargo clippy                # Zero warnings, #![forbid(unsafe_code)]
cargo doc --open            # Full API documentation
```

**Wave 56:** `TransportMode` enum (UDS-only / TCP default / TCP opt-in) for VPS
deployment standard. `HealthCheckMethod::SocketExists` for UDS socket checks.
`CompositionSpec::uds_socket_paths()` and `tcp_ports_uds_mode()` helpers.
13 transport-specific tests covering serde roundtrip, service classification,
socket path conventions, and composition-level UDS queries.

Wave 51: `FirewallRule.comment` zero-allocation (`&'static str`),
supplementary ports in service registry (hbbs 21115, caddy 80),
output-only types drop `Deserialize`.

The `membrane.toml` config file is the user-facing interface. Write one,
validate it with `cellmembrane-types`, and deploy with `deploy_membrane.sh`.

### Operational Docs

| File | Purpose |
|------|---------|
| `VPS_STATE.md` | Live VPS state snapshot |
| `GLACIAL_SHIFT_TRACKER.md` | Stadial entry blocker tracking |
| `RUNBOOKS.md` | Operational procedures for all channels |
| `IRONGATE_VERIFICATION.md` | ironGate acceptance checklist |

### Sync Scripts

| Script | Purpose |
|--------|---------|
| `forgejo_sync.sh` | Sync non-mirror repos GitHub → Forgejo |
| `forgejo_pull_mirror.sh` | Bulk Forgejo pull-mirror management |

Forgejo is the primary remote; GitHub is the public mirror. See `infra/wateringHole/REPO_MEMBRANE_BOUNDARY.md` for classification.

---

## Quick Start

```bash
# Check cellMembrane status (all channels + services)
cd ../../infra/plasmidBin
./deploy_membrane.sh status root@$VPS_IP

# SSH to VPS
ssh root@$VPS_IP

# View Tower logs (BearDog → Songbird → SkunkBat)
ssh root@$VPS_IP "journalctl -u beardog-membrane -u songbird-relay -u skunkbat-membrane -f"

# View Nest Atomic logs (provenance trio)
ssh root@$VPS_IP "journalctl -u nestgate-membrane -u rhizocrypt-membrane -u loamspine-membrane -u sweetgrass-membrane -f"

# View RustDesk logs
ssh root@$VPS_IP "journalctl -u hbbs-membrane -u hbbr-membrane -f"

# Manage SSH keys for multi-gate access
./deploy_membrane.sh keys list root@$VPS_IP
./deploy_membrane.sh keys add root@$VPS_IP --name "friend-gate" --pubkey "ssh-ed25519 AAAA..."
./deploy_membrane.sh keys revoke root@$VPS_IP --name "friend-gate"
```

---

## Hardening Status

| Check | Status |
|-------|--------|
| exim4 removed | DONE |
| droplet-agent purged | DONE |
| fail2ban active (systemd backend) | DONE |
| UFW: 22+53+3478+8443+9500+9602+9700+9850+21115-21117+80+443 | DONE |
| SSH key-only auth (multi-gate managed) | DONE |
| credentials.env redundant plaintext removed | DONE |
| journald persistence | DONE |
| TURN credentials at /etc/songbird/relay-credentials | DONE |
| RustDesk hbbs+hbbr running (sovereign relay) | DONE |
| Caddy TLS with Let's Encrypt | DONE |
| Stripped static ELF binaries | DONE |
| Dark Forest audit: 21 PASS, 0 FAIL, 1 SKIP | DONE (Wave 38, Nest Atomic) |
| Provenance trio pipeline: 10/10 PASS on VPS | DONE |
| Shadow orchestrator: 6/6 PASS | DONE |
| NestGate :9500, rhizoCrypt :9602, loamSpine :9700, sweetGrass :9850 | DONE |
| VPS deployment standard (Wave 56): UDS-only, TransportMode typed | DONE |

---

## Sovereignty Shadow Status

| Track | Sovereign Component | Commercial Shadow | Status | Cutover Gate |
|-------|--------------------|--------------------|--------|--------------|
| S1 TLS | BearDog :8443 | Cloudflare | Shadow live, not cut over | 7-day p95 ≤ 1.5× |
| S2 NAT relay | Songbird TURN :3478 | cloudflared | **LIVE** | 7-day 100% reachable |
| S3 Content | NestGate + petalTongue | GitHub Pages | **LIVE** (68ms TTFB) | 7-day TTFB parity |
| S4 Auth | BearDog BTSP dual-auth | OAuth2/PAM | Ready, incomplete | 7-day p95 < 50ms |

---

## Escalation Ladder

| Phase | Deliverable | Status |
|-------|-------------|--------|
| 0 | Relay only | Superseded |
| 0.5 | Relay + RustDesk + multi-gate SSH | Completed May 14 |
| 1 | Tower composition | Completed May 18 |
| **1.5** | **Nest Atomic + Channel 1 DNS + TLS + VPS Standard** | **Current** (Wave 56, 2026-05-27) |
| 2 | Encrypted-at-rest (BearDog Vault) | Planned |
| 3 | BingoCube zero-knowledge access | Future |
| 3.5 | SoloKey hardware attestation | Future |
| 4 | Full autonomy (BearDog auto-rotation) | Future |

---

## Ownership Boundaries

**cellMembrane team owns:**
- This repo — VPS state, runbooks, credentials, IP/key inventory
- Membrane channel deployment — Signal/DNS, Relay, Surface/TLS
- Caddy TLS certificate management and reverse proxy on VPS
- Sovereign DNS (knot-dns on VPS, replacing commercial DNS)
- RustDesk self-hosted remote access
- Multi-gate expansion (westGate, northGate provisioning)

**cellMembrane team does NOT own:**
- sporePrint (primalSpring, transferred Wave 46)
- Gate-level validation (projectNUCLEUS — Dark Forest + sovereignty checks)
- Deployment pipeline software (projectNUCLEUS ships `deploy_membrane.sh`; we operate it)
- biomeOS substrate

**Signal flow:** `primalSpring → upstream primals → biomeOS → projectNUCLEUS → cellMembrane`

---

## RustDesk Client Configuration

Configure RustDesk clients on each gate to use the cellMembrane as
rendezvous and relay:

| Setting | Value |
|---------|-------|
| ID Server | `$VPS_IP` |
| Relay Server | `$VPS_IP` |
| Key | (from `/opt/membrane/rustdesk/id_ed25519.pub` on VPS) |

Server public key stored at `/opt/membrane/rustdesk/id_ed25519.pub` on the VPS.

---

## Repository Structure

```
gardens/cellMembrane/
  Cargo.toml                  # Rust workspace root
  membrane.toml               # Reference config (live deployment)
  crates/
    cellmembrane-types/       # Typed domain models (#![forbid(unsafe_code)])
      src/
        lib.rs                # Crate root, re-exports, shared helpers
        channels.rs           # Signal / Relay / Surface
        composition.rs        # Relay → RustDesk → Tower → Nest
        config.rs             # membrane.toml parser + validator + ShadowMode
        credentials.rs        # age / BTSP vault / manual
        envelope.rs           # K-Derm topology — monoderm/diderm, bonding, channel proteins
        firewall.rs           # UFW rules from composition
        identity.rs           # Family ID, gate ID
        provider.rs           # DigitalOcean / Hetzner / bare metal / gate-local
        service.rs            # Static service registry — zero allocation, no Box::leak
        validation.rs         # Report pattern (pass/fail/warn)
      tests/
        channels.rs           # Channel trust, ports, crypto, serde (4 tests)
        composition.rs        # Ladder ordering, specs, serde (6 tests)
        envelope.rs           # K-Derm topology, layers, bonding, policies (27 tests)
        firewall.rs           # UFW derivation per composition (5 tests)
        service.rs            # Registry, binary integrity, credentials (15 tests)
        transport.rs          # TransportMode, UDS helpers, health checks (13 tests)
        integration.rs        # Cross-module: config parsing, validation, topology (23 tests)
  specs/                      # Formal architecture specs (5 documents)
  README.md
  RUNBOOKS.md
  GLACIAL_SHIFT_TRACKER.md
  VPS_STATE.md
  IRONGATE_VERIFICATION.md
  forgejo_sync.sh             # Sync non-mirror repos GitHub → Forgejo
  forgejo_pull_mirror.sh      # Bulk Forgejo pull-mirror management
  .gitignore
```

---

## Testing Infrastructure

cellMembrane K-Derm topology is validated by the ecosystem's testing infrastructure:

| Repo | Location | Role |
|------|----------|------|
| benchScale | `infra/benchScale/` | Reproducible isolated test environments, K-Derm diderm topology in `topologies/nucleus/` |
| agentReagents | `infra/agentReagents/` | Manifest-driven VM image building, `plasmidBin` integration |

Both are mature Rust codebases converged into `infra/` as of Wave 51 (272 + 94 tests).
Deep debt sprint (Wave 56): benchScale `senescence.rs` smart refactored (829L → types + mod),
`BackendType` String → enum, TTY-safe build pause, `DEFAULT_DEPLOY_DIR` evolved to `/opt/plasmidBin`.
agentReagents `CloudInitStatusInfo.status` String → typed `CloudInitStatus` enum.
plasmidBin remote dir centralized via `ECOPRIMALS_PLASMID_BIN` env var, stale socket dirs updated.

---

## Related Resources

| Resource | Location | Relationship |
|----------|----------|-------------|
| Deploy script | `infra/plasmidBin/deploy_membrane.sh` | Primary operational tool (1199 lines) |
| Channel architecture | `infra/wateringHole/MEMBRANE_CHANNEL_ARCHITECTURE.md` | Channel isolation, port policy, crypto layers |
| fieldMouse spec | `infra/wateringHole/CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md` | Deployment class, hardening checklist, boot order |
| K-NOME programming | `infra/whitePaper/gen3/about/K_NOME_PROGRAMMING.md` | K-Derm topology parallels K-NOME methodology |
| Config SSOT | `gardens/projectNUCLEUS/deploy/nucleus_config.sh` | Port map, VPS config, shadow settings |
| Dark Forest standard | `infra/wateringHole/DARK_FOREST_GLACIAL_GATE_STANDARD.md` | 5-pillar security audit |
| Glacial readiness | `infra/wateringHole/GLACIAL_SHIFT_READINESS.md` | 6 stadial entry criteria |
| Credential tooling | `infra/plasmidBin/membrane/share_credentials.sh` | Age-encrypted credential sharing |
| Validation | `gardens/projectNUCLEUS/validation/darkforest_membrane.sh` | Dark Forest audit harness |

---

## License

AGPL-3.0-or-later
