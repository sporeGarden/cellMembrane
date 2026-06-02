# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (ironGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | NUCLEUS (13 primals: Tower + Nest + Compute + Meta) + RustDesk |
| **Escalation** | Phase 2 (NUCLEUS) — **current** (Wave 69, 2026-06-02) |

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
cargo test                  # 209 tests — pedantic clippy clean
cargo clippy                # Zero warnings (pedantic + nursery), #![forbid(unsafe_code)]
cargo doc --open            # Full API documentation with doc-tests
```

**Wave 69 (Sovereignty Graduation):** Membrane binary deployed to VPS
(`x86_64-musl`, 6.1M static). Full K-Derm relay validated in Rust (zero bash
dependencies for relay chain). S4 auth gate activated (`BEARDOG_AUTH_MODE=enforced`).
Disk cleanup 69%→60%. Workspace resolution evolved for VPS sparse deployments.
Relay ship bug fixed (`git remote get-url` stdout leak). 209 tests.

**Wave 68 (Graduated Composition):** Neural Bridge wired into dispatch
(try-primal-first for 12 commands). `gate.pull`/`gate.check` evolved to use
Rust `membrane` binary on VPS. `PushResult` replaces silent push failures.
`#[must_use]` sweep across 7 modules. `resolve_workspace_root()` promoted
to crate-level utility. `forgejo_work_dir` config chain. 209 tests.

**Wave 67+ (Cascade Evolution Sprint):** dispatch.rs split into 5 domain
submodules (all <340L). Tree-parity divergence auto-resolution. `--publish-freshness`
wired. `post_sync_diverge()` + graduated merge strategies. Impulse ack safety
(separate ack files). Binary freshness tracking (`--check-installed`). All
hardcoded paths evolved to capability-based discovery. rsync eliminated
(SSH+cat). `ServicePaths` + `CredentialPaths` runtime resolvers.

**Wave 66 (Deep Debt Evolution):** Eliminated 3 external tool dependencies
(socat→native UDS, curl→reqwest, b3sum→blake3 crate). Removed deprecated
signal.rs. Real BLAKE3 verification via checksums.toml. K-Derm relay chain
implemented in Rust (relay.rs). `FetchSource` implements `FromStr` trait.

**Wave 65 (K-Derm Relay):** New `relay.rs` module — full peptidoglycan
relay chain in Rust: `relay::mediate()`, `relay::ship_extracellular()`,
`relay::run()`. Exposed as `membrane relay.run <repo_path>`. Replaces
`pepti-sync-relay.sh` (95L) and `ext-github-push.sh` (91L).

**Wave 59 (NUCLEUS composition):** Full 13-primal NUCLEUS composition tier
added to service registry. 6 new services: toadStool, barraCuda, coralReef
(compute), biomeOS, squirrel, petalTongue (meta). All UDS-only. Zero new
firewall ports. `has_biomeos()` capability query. Spring overlay readiness.

Wave 57: `clippy::pedantic` + `clippy::nursery` enforced. Typed `ConfigError`
via `thiserror`. `DeployPaths` configurable paths. Coverage 77% → 96%.

Wave 56: `TransportMode` enum (UDS-only / TCP default / TCP opt-in).
`HealthCheckMethod::SocketExists` for UDS socket checks.

The `membrane.toml` config file is the user-facing interface. Write one,
validate it with `cellmembrane-types`, and deploy with `deploy_membrane.sh`.

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
membrane relay.run infra/wateringHole    # Full K-Derm relay: pull → impulse → ship
membrane temporal.cascade                 # Manifest-driven cascade sync (Rust)
membrane mirror.sync-all                  # Trigger Forgejo mirror sync for all repos
membrane impulse.ack <id>                 # Acknowledge inter-gate impulse
membrane plasmid.fetch                    # Download primal binaries with BLAKE3 verification
```

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
| Deep debt sprint (Wave 57): 95.8% coverage, pedantic clean, typed errors | DONE |
| NUCLEUS composition typed (Wave 59): 13 primals, 17 services in registry | DONE |
| K-Derm relay chain in Rust (Wave 65): relay.rs replaces bash scripts | DONE |
| Deep debt evolution (Wave 66): socat/curl/b3sum → native Rust | DONE |
| Cascade evolution sprint (Wave 67+): dispatch split, tree-parity, freshness, ack safety, capability paths | DONE |
| Graduated composition (Wave 68): Neural Bridge in dispatch, gate bash→Rust, PushResult, #[must_use] sweep | DONE |
| Sovereignty graduation (Wave 69): membrane deployed to VPS, S4 auth enforced, relay Rust-native, disk 60% | DONE |

---

## Sovereignty Shadow Status

| Track | Sovereign Component | Commercial Shadow | Status | Cutover Gate |
|-------|--------------------|--------------------|--------|--------------|
| S1 TLS | Caddy + LE | Cloudflare (INACTIVE) | **OPERATIONAL** (13d clean, 7-day gate passed) | Graduated |
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
| 1.5 | Nest Atomic + Channel 1 DNS + TLS + VPS Standard + Deep Debt | Completed (Wave 57) |
| **2** | **NUCLEUS (13 primals) + biomeOS + Spring Overlays + Rust relay** | **Current** (Wave 69, 2026-06-02) |
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
        composition.rs        # Ladder ordering, specs, serde (6 tests)
        coverage.rs           # Deep coverage expansion (78 tests)
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
        plasmid.rs            # Primal binary fetch (reqwest + blake3, SSH for VPS)
        forgejo.rs            # Forgejo REST API (native reqwest)
        config.rs / ssh.rs    # Config resolution + SSH transport
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

scyBorg triple license:
- **AGPL-3.0-or-later** — code (Rust, TOML, shell scripts, tests)
- **ORC** — coordination patterns and mechanics
- **CC-BY-SA 4.0** — documentation and creative content
