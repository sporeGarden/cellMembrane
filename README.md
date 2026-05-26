# cellMembrane

**Private operational repo for the cellMembrane fieldMouse deployment — the sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (ironGate) |
| **Class** | fieldMouse — Tower atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, 157.230.3.183, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | Tower (BearDog + Songbird + SkunkBat) + RustDesk (hbbs + hbbr) |
| **Escalation** | Phase 1 (Tower) — **current** → Phase 1.5 (Nest + DNS + TLS) next |

---

## Active Membrane Channels

| Channel | Function | Primal / Service | Port | Status |
|---------|----------|-----------------|------|--------|
| **2 Relay** | NAT traversal, TURN | Songbird | :3478 tcp/udp | **LIVE** |
| **2b RustDesk** | Sovereign remote desktop | hbbs + hbbr | :21115-21117 | **LIVE** |
| **3 Surface** | HTTPS, downloads, ACME | Caddy + NestGate | :80/:443 | **LIVE** — `membrane.primals.eco` (Let's Encrypt E8) |
| **1 Signal** | DNS resolution for `primals.eco` | knot-dns | :53 | **PLANNED** — glacial shift blocker |

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

### Rust Types (`crates/cellmembrane-types/`)

Typed domain models for membrane configuration, validation, and deployment:

```bash
cargo test                  # 46 tests — config parsing, composition, firewall, Dark Forest audit alignment
cargo doc --open            # Full API documentation
```

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

**This repo is private.** Classified as inner-membrane-only per `REPO_MEMBRANE_BOUNDARY.md` — Forgejo is the target remote; GitHub mirror is transitional.

---

## Quick Start

```bash
# Check cellMembrane status (all channels + services)
cd ../../infra/plasmidBin
./deploy_membrane.sh status root@157.230.3.183

# SSH to VPS
ssh root@157.230.3.183

# View Tower logs (BearDog → Songbird → SkunkBat)
ssh root@157.230.3.183 "journalctl -u beardog-membrane -u songbird-relay -u skunkbat-membrane -f"

# View RustDesk logs
ssh root@157.230.3.183 "journalctl -u hbbs-membrane -u hbbr-membrane -f"

# View Caddy / TLS logs
ssh root@157.230.3.183 "journalctl -u caddy -f"

# Manage SSH keys for multi-gate access
./deploy_membrane.sh keys list root@157.230.3.183
./deploy_membrane.sh keys add root@157.230.3.183 --name "friend-gate" --pubkey "ssh-ed25519 AAAA..."
./deploy_membrane.sh keys revoke root@157.230.3.183 --name "friend-gate"

# Deploy Nest expansion (next phase)
./deploy_membrane.sh deploy root@157.230.3.183 --composition nest --validate
```

---

## Hardening Status

| Check | Status |
|-------|--------|
| exim4 removed | DONE |
| droplet-agent purged | DONE |
| fail2ban active (systemd backend) | DONE |
| UFW: 22+3478+21115-21117+80+443 | DONE |
| SSH key-only auth (multi-gate managed) | DONE |
| credentials.env redundant plaintext removed | DONE |
| journald persistence | DONE |
| TURN credentials at /etc/songbird/relay-credentials | DONE |
| RustDesk hbbs+hbbr running (sovereign relay) | DONE |
| Caddy TLS with Let's Encrypt | DONE |
| Stripped static ELF binaries | DONE |
| Dark Forest audit: 17 PASS, 0 FAIL | DONE |
| Trio pipeline: 10/10 PASS on VPS | DONE |

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
| **1** | **Tower composition** | **Current** |
| **1.5** | **Nest expansion + Channel 1 DNS + Channel 3 TLS hardening** | **Next — glacial shift gate** |
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
| ID Server | `157.230.3.183` |
| Relay Server | `157.230.3.183` |
| Key | `YxLlA1Nb6mlH5FmcCQod6kDD6bIcXT5R3ex1CAFogMU=` |

Server public key stored at `/opt/membrane/rustdesk/id_ed25519.pub` on the VPS.

---

## Repository Structure

```
gardens/cellMembrane/
  Cargo.toml                  # Rust workspace root
  membrane.toml               # Reference config (live deployment)
  crates/
    cellmembrane-types/       # Typed domain models
      src/
        lib.rs                # Crate root, re-exports
        channels.rs           # Signal / Relay / Surface
        composition.rs        # Relay → RustDesk → Tower → Nest
        config.rs             # membrane.toml parser + validator
        credentials.rs        # age / BTSP vault / manual
        firewall.rs           # UFW rules from composition
        identity.rs           # Family ID, gate ID
        provider.rs           # DigitalOcean / Hetzner / bare metal / gate-local
        service.rs            # Binary, port, systemd, health
        validation.rs         # Report pattern (pass/fail/warn)
      tests/
        integration.rs        # 31 integration tests
  specs/                      # Formal architecture specs
  README.md
  RUNBOOKS.md
  GLACIAL_SHIFT_TRACKER.md
  VPS_STATE.md
  IRONGATE_VERIFICATION.md
  forgejo_sync.sh
  forgejo_pull_mirror.sh
  .gitignore
```

---

## Related Resources

| Resource | Location | Relationship |
|----------|----------|-------------|
| Deploy script | `infra/plasmidBin/deploy_membrane.sh` | Primary operational tool (982 lines) |
| Channel architecture | `infra/wateringHole/MEMBRANE_CHANNEL_ARCHITECTURE.md` | Channel isolation, port policy, crypto layers |
| fieldMouse spec | `infra/wateringHole/CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md` | Deployment class, hardening checklist, boot order |
| Config SSOT | `gardens/projectNUCLEUS/deploy/nucleus_config.sh` | Port map, VPS config, shadow settings |
| Dark Forest standard | `infra/wateringHole/DARK_FOREST_GLACIAL_GATE_STANDARD.md` | 5-pillar security audit |
| Glacial readiness | `infra/wateringHole/GLACIAL_SHIFT_READINESS.md` | 6 stadial entry criteria |
| Credential tooling | `infra/plasmidBin/membrane/share_credentials.sh` | Age-encrypted credential sharing |
| Validation | `gardens/projectNUCLEUS/validation/darkforest_membrane.sh` | Dark Forest audit harness |

---

## License

AGPL-3.0-or-later
