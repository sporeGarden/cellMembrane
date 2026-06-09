# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (ironGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | NUCLEUS (13 primals: Tower + Nest + Compute + Meta) + RustDesk |
| **Escalation** | Phase 2 (NUCLEUS) — **current** (Wave 105, 2026-06-09) |

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
| `RELAY_TRUST_BOUNDARY.md` | Cross-gate relay security audit — BTSP opacity, trust levels per channel |

### Rust Types (`crates/cellmembrane-types/`)

Typed domain models for membrane configuration, validation, and deployment:

```bash
cargo test                  # 161 tests + 5 doc-tests — pedantic clippy clean
cargo clippy                # Zero warnings (pedantic + nursery), #![forbid(unsafe_code)]
cargo doc --open            # Full API documentation with doc-tests
```

**Wave 105 (WAN Depot + aarch64 Sweep + Zero P1):**
WAN binary distribution SHIPPED — `plasmid.fetch --source wan` fetches from
`https://membrane.primals.eco/depot/` over HTTPS. `caddy.depot.provision` deploys
the file_server route. aarch64 cross-compile sweep complete: 14/14 primals build
cleanly for `aarch64-unknown-linux-musl`, zero C-dep violations ecosystem-wide.
Cascade conflict auto-resolve eliminates manual intervention on regenerable metadata.
`sha2`/`hmac` crates replace 200L hand-rolled crypto. `PostSyncPhase` enum replaces
bool flags. 39 typed `ENV_*` constants, zero inline env var strings. All hardcoded
`/tmp` paths replaced with `std::env::temp_dir()`. Harvest uses atomic rename
(`.new` + `rename(2)`). Webhook primal list deduplicated to service registry.
161 tests, zero clippy, zero `#[allow]`, zero `unsafe`, all files <800L.

**Wave 82c–104 (NUCLEUS Parity → Deep Debt):** plasmidBin ownership transferred.
`plasmid.refresh` (atomic binary push), Cloudflare module evolved, 5-domain TLS,
Neural Bridge dispatch, legacy cascade removed, all `too_many_lines` eliminated,
K-Derm relay in Rust, external tools (socat/curl/b3sum) replaced with native Rust.
Cascade evolution (tree-parity, freshness publishing, graduated merge strategies).
NUCLEUS 13/13 ALIVE. Sovereignty graduation (S4 auth enforced). All 226→161 tests
(consolidated — coverage maintained, redundant tests removed).

The `membrane.toml` config file is the user-facing interface. Write one,
validate it with `cellmembrane-types`, and deploy with the `membrane` CLI.

### Operational Docs

| File | Purpose |
|------|---------|
| `VPS_STATE.md` | Live VPS state snapshot |
| `GLACIAL_SHIFT_TRACKER.md` | Stadial entry blocker tracking |
| `RUNBOOKS.md` | Operational procedures for all channels |
| `IRONGATE_VERIFICATION.md` | ironGate acceptance checklist |

### Shadow Functions (`crates/membrane-shadow/`)

Typed Rust CLI for sovereign VPS control — replaces all bash sync/relay scripts:

```bash
membrane temporal.cascade                 # Manifest-driven cascade sync (38 repos)
membrane temporal.cascade --with-rebuild  # Cascade + harvest stale + push to VPS
membrane plasmid.fetch --source wan       # WAN HTTPS fetch (no SSH required)
membrane plasmid.harvest --target aarch64-unknown-linux-musl  # Cross-compile
membrane plasmid.refresh                  # Push local binaries to VPS (atomic replace)
membrane caddy.depot.provision            # Provision /depot/ HTTPS file server
membrane caddy.status                     # VPS Caddy health + vhosts + TLS
membrane relay.run infra/wateringHole     # Full K-Derm relay: pull → impulse → ship
```

---

## Quick Start

```bash
# Check cellMembrane status (all channels + services)
membrane gate.health

# Deploy / refresh primal binaries on VPS
membrane plasmid.refresh --primal beardog
membrane plasmid.refresh                    # All nucleus primals

# Cascade sync (manifest-driven, Rust-native)
membrane temporal.cascade

# SSH to VPS
ssh root@$VPS_IP

# View Tower logs (BearDog → Songbird → SkunkBat)
ssh root@$VPS_IP "journalctl -u beardog-membrane -u songbird-relay -u skunkbat-membrane -f"

# View Nest Atomic logs (provenance trio)
ssh root@$VPS_IP "journalctl -u nestgate-membrane -u rhizocrypt-membrane -u loamspine-membrane -u sweetgrass-membrane -f"
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
| Deep debt sprint (Wave 57): 95.8% coverage, pedantic clean, typed errors | DONE |
| NUCLEUS composition typed (Wave 59): 13 primals, 17 services in registry | DONE |
| K-Derm relay chain in Rust (Wave 65): relay.rs replaces bash scripts | DONE |
| Deep debt evolution (Wave 66): socat/curl/b3sum → native Rust | DONE |
| Cascade evolution sprint (Wave 67+): dispatch split, tree-parity, freshness, ack safety, capability paths | DONE |
| Graduated composition (Wave 68): Neural Bridge in dispatch, gate bash→Rust, PushResult, #[must_use] sweep | DONE |
| Sovereignty graduation (Wave 69): membrane deployed to VPS, S4 auth enforced, relay Rust-native, disk 60% | DONE |
| Deep debt evolution (Wave 69+): all too_many_lines eliminated, CascadeMode enum, freshness wired, all files <800L | DONE |
| Capability expansion (Wave 71): legacy cascade() removed, relay.status/gate.health/content.verify, S3 VPS READY | DONE |
| Evolution sprint (Wave 74+): zero #[allow], HardeningStep enum, fetch decomposed, mesh join (3rd gate) | DONE |
| NUCLEUS full parity (Wave 82c): 13/13 ALIVE, Caddy 5-domain TLS, plasmidBin owned | DONE |
| WAN depot + aarch64 sweep (Wave 105): WAN fetch HTTPS, 14/14 aarch64, sha2/hmac crates, zero P1 | DONE |

---

## Sovereignty Shadow Status

| Track | Sovereign Component | Commercial Shadow | Status | Cutover Gate |
|-------|--------------------|--------------------|--------|--------------|
| S1 TLS | Caddy + LE | Cloudflare (INACTIVE) | **OPERATIONAL** (13d clean, 7-day gate passed) | Graduated |
| S2 NAT relay | Songbird TURN :3478 | cloudflared | **LIVE** | 7-day 100% reachable |
| S3 Content | NestGate + petalTongue | GitHub Pages | **LIVE** (68ms TTFB) | 7-day TTFB parity |
| S4 Auth | BearDog BTSP dual-auth | OAuth2/PAM | **7-day gate ending** | 7-day p95 < 50ms |

---

## Escalation Ladder

| Phase | Deliverable | Status |
|-------|-------------|--------|
| 0 | Relay only | Superseded |
| 0.5 | Relay + RustDesk + multi-gate SSH | Completed May 14 |
| 1 | Tower composition | Completed May 18 |
| 1.5 | Nest Atomic + Channel 1 DNS + TLS + VPS Standard + Deep Debt | Completed (Wave 57) |
| **2** | **NUCLEUS (13 primals) + biomeOS + WAN depot + aarch64 universal** | **Current** (Wave 105, 2026-06-09) |
| 2.5 | Encrypted-at-rest (BearDog Vault) | Planned |
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
- plasmidBin — binary harvesting, checksums, `sources.toml`, CI workflows
- VPS deployment ops — systemd units, `socat` bridges, firewall, refresh cycles
- Peptidoglycan self-refresh timer and auto-fetch evolution

**cellMembrane team does NOT own:**
- sporePrint (primalSpring, transferred Wave 46)
- Gate-level validation (projectNUCLEUS — Dark Forest + sovereignty checks)
- biomeOS substrate
- Upstream primal blurb generation (wateringHole overwatch)

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
  Cargo.toml                  # Rust workspace root (pedantic + nursery lints)
  membrane.toml               # Reference config (live deployment)
  rustfmt.toml                # Format config (edition 2024, 100 col)
  deny.toml                   # cargo-deny ecoBin ban list
  LICENSE                     # AGPL-3.0-or-later
  LICENSE-ORC                 # ORC (mechanics)
  LICENSE-CC-BY-SA            # CC-BY-SA 4.0 (creative)
  crates/
    cellmembrane-types/       # Typed domain models (#![forbid(unsafe_code)])
      src/
        lib.rs                # Crate root, re-exports, doc-tests
        channels.rs           # Signal / Relay / Surface
        composition.rs        # Relay → RustDesk → Tower → Nest + iter_binaries()
        config.rs             # membrane.toml parser + validator + DeployPaths
        credentials.rs        # age / BTSP vault / manual
        envelope.rs           # K-Derm topology — monoderm/diderm, bonding, channel proteins
        error.rs              # Typed ConfigError (thiserror)
        firewall.rs           # UFW rules from composition (SSH_PORT constant)
        identity.rs           # Family ID, gate ID
        provider.rs           # DigitalOcean / Hetzner / bare metal / gate-local
        service.rs            # Static service registry — zero allocation, const fn
        validation.rs         # Report pattern (pass/fail/warn) + doc-tests
      tests/
        channels.rs           # Channel trust, ports, crypto, serde (4 tests)
        composition.rs        # Ladder ordering, specs, NUCLEUS, serde (21 tests)
        coverage.rs           # Deep coverage expansion (63 tests)
        envelope.rs           # K-Derm topology, layers, bonding, policies (27 tests)
        firewall.rs           # UFW derivation per composition (5 tests)
        service.rs            # Registry, binary integrity, credentials (15 tests)
        transport.rs          # TransportMode, UDS helpers, health checks (13 tests)
        integration.rs        # Cross-module: config parsing, validation, topology (23 tests)
    membrane-shadow/          # Sovereign shadow functions CLI (#![forbid(unsafe_code)])
      src/
        dispatch/             # CLI command router (5 domain submodules, all <340L)
          mod.rs              # Top-level run() router
          temporal.rs         # cascade, check, sync dispatch
          impulse.rs          # impulse + potential sense dispatch
          infra.rs            # repo, mirror, service, gate, token
          data.rs             # manifest, identity, context, plasmid, relay
        relay.rs              # K-Derm relay chain (SSH+cat, no rsync)
        impulse/              # Inter-gate impulse (7 submodules, native UDS JSON-RPC)
        temporal.rs           # Manifest-driven temporal cascade + tree-parity
        freshness.rs          # Wave freshness publishing + binary drift detection
        plasmid/              # Primal binary lifecycle
          mod.rs              # Registry-derived primal list, target triple, shared utils
          fetch.rs            # Fetch from Forgejo/GitHub releases + BLAKE3 verify
          refresh.rs          # Atomic push to VPS via SCP + service restart
        cloudflare.rs         # Cloudflare API v4 (DNS, cache, SSL, zones)
        forgejo.rs            # Forgejo REST API (native reqwest)
        bridge.rs             # Neural API bridge (UDS discovery)
        config.rs / ssh.rs    # Config resolution + SSH/SCP transport
  specs/                      # Formal architecture specs (5 documents)
  experiments/                # Validated experiment records
  README.md
  RUNBOOKS.md
  GLACIAL_SHIFT_TRACKER.md
  VPS_STATE.md
  IRONGATE_VERIFICATION.md
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
| Deploy script | `infra/plasmidBin/deploy_membrane.sh` | Operational tool (being absorbed into Rust CLI) |
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

scyBorg triple license:
- **AGPL-3.0-or-later** — code (Rust, TOML, shell scripts, tests)
- **ORC** — coordination patterns and mechanics
- **CC-BY-SA 4.0** — documentation and creative content
