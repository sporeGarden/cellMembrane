# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (sporeGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | NUCLEUS (13 primals: Tower + Nest + Compute + Meta) + RustDesk, 6-gate mesh |
| **Escalation** | Phase 2 (NUCLEUS) — **stadial-ready** (Wave 107+, through Wave 147a) |

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
cargo test                  # 1081 tests — pedantic clippy clean
cargo clippy                # Zero warnings (pedantic + nursery + option_if_let_else)
cargo doc --open            # Full API documentation with doc-tests
```

Current state (Wave 147a): ~9k lines types, ~36k lines shadow. All manifest fields
type-safe (`GateRole`, `CascadeSource`, `GateMobility`, `BindMode`, `EnvelopeTopology`,
`MembraneComposition`, `Platform`, `TargetArch`, `TransportEndpoint`).
Rich cross-field validation wired (`validate.rs`). SIGN-01 depot signing pipeline
(BLAKE3 + ed25519). Fail-closed sandbox. ELF DT_NEEDED enforcement. Sovereign-first
drift detection. OS Atheism Phase 1+2 (platform types, named pipes, process lifecycle).
Wave 147a: `gate.enroll` automated mesh enrollment (WG keygen, config render,
mesh verify, Forgejo SSH verify, Forgejo-first git remote config).
Deep debt sweep (140a–147a): visibility tightened, allocation hot paths optimized,
error taxonomy reclassified, domain constants centralized, CAC tree-parity checks,
CSPRNG unified via `getrandom`, service filter registry-derived, `ProbeResult` typed
gate probes, `build_err` consolidated, zero f64 casts in display formatting,
nested `if let` → let-chains (Rust 2024 edition).
Full evolution history in `GLACIAL_SHIFT_TRACKER.md` and git log.

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
membrane gate.status                      # Local gate health (native UDS probes + depot + mesh)
membrane gate.enroll <name> [--dry-run]    # Mesh enrollment (WG keys, config, remotes)
membrane gate.bootstrap <name> [--dry-run] [--mobile]  # Profile-driven deployment (7 phases)
membrane gate.profile <name>              # Read gate profile from ecosystem_manifest.toml
membrane gate.quorum [--interval 15] [--generate]      # Install autonomous cascade timer (Quorum P1)
membrane temporal.cascade                 # Manifest-driven cascade sync (38 repos)
membrane temporal.cascade --with-restart  # Cascade + fetch + restart updated primals
membrane temporal.cascade --with-rebuild  # Cascade + harvest stale + push to VPS
membrane plasmid.build <primal> [--target T]  # guideStone-grade single-primal build
membrane plasmid.fetch --source wan       # WAN HTTPS fetch + dual BLAKE3 verification
membrane plasmid.harvest                  # Build + checksum + auto-publish to git
membrane plasmid.harvest --local          # Build from local checkout (~10x faster)
membrane plasmid.harvest --target aarch64-linux-android  # NDK cross-compile
membrane plasmid.ndk.check                # Verify NDK toolchain readiness
membrane plasmid.refresh                  # Push depot binaries to VPS (atomic replace)
membrane plasmid.depot_sync               # Sync install-dir → depot on VPS
membrane plasmid.depot_sync --push        # Push local depot → remote VPS depot (builder mode)
membrane plasmid.pipeline                 # End-to-end: harvest → sandbox → refresh
membrane plasmid.trigger                  # Kick remote VPS pipeline via SSH
membrane plasmid.sandbox --primal beardog # Sandbox validation (isolated UDS probe)
membrane plasmid.sandbox --primal X --promote  # Validate + atomic promote to production
membrane plasmid.canary.list              # Show canary pool state (previous-good)
membrane plasmid.canary.health            # Health-check all canary instances
membrane plasmid.canary.promote --primal X  # Rollback: canary → production
membrane plasmid.canary.failover          # List healthy failover targets
membrane depot.integrity                  # Generate checksums.toml (BLAKE3) for all depot binaries
membrane depot.integrity --verify         # Verify existing checksums against depot
membrane caddy.depot.provision            # Provision /depot/ HTTPS file server
membrane caddy.status                     # VPS Caddy health + vhosts + TLS
membrane relay.run infra/wateringHole     # Full K-Derm relay: pull → impulse → ship
membrane manifest.validate                # Schema validation (cross-refs, counts, duplicates)
membrane gateway.sporeprint.units [gate] [--domain X]  # Generate 4-primal sporePrint NUCLEUS units
membrane gateway.sporeprint.check [gate]  # Pre-deploy readiness for sporePrint NUCLEUS
membrane topology.service <role>          # Find gate providing a service role
membrane topology.endpoint <gate> <cap>   # Resolve transport endpoint (UDS/TCP/relay)
membrane topology.roles                   # Map all service→gate assignments from manifest
membrane topology.mesh                    # Show WireGuard mesh topology
```

---

## Quick Start

```bash
# Bootstrap a new gate (one command — fetch, verify, mesh, start, health)
membrane gate.bootstrap ironGate

# Check local gate health (no SSH required)
membrane gate.status

# VPS health + service summary
membrane gate.health

# Cascade sync (manifest-driven)
membrane temporal.cascade

# Fetch all primals from WAN depot (BLAKE3 verified)
membrane plasmid.fetch --source wan

# Build from local checkout + push to VPS
membrane plasmid.harvest --local && membrane plasmid.refresh

# Or full pipeline (harvest → sandbox → refresh)
membrane plasmid.pipeline

# SSH to VPS
ssh root@$VPS_IP "journalctl -u beardog-membrane -u songbird-membrane -f"
```

---

## Hardening Status

All infrastructure hardening, sovereignty graduation, and evolution milestones
through Wave 147a are **DONE**. Full wave-by-wave audit trail is preserved in
`GLACIAL_SHIFT_TRACKER.md` and git log.

| Category | Summary | Status |
|----------|---------|--------|
| Infrastructure | exim4/droplet-agent purged, fail2ban, UFW, SSH key-only, journald persistence | DONE |
| TLS | Caddy + Let's Encrypt sovereign TLS, Cloudflare removed | DONE |
| Dark Forest | 21/21 PASS, 5-pillar compliance, stripped static ELF binaries | DONE |
| NUCLEUS | 13/13 primals ALIVE, 6-node WG mesh, UDS-only, sandbox + canary pipeline | DONE |
| Sovereignty | S1–S4 all GRADUATED, BTSP enforced, sovereign DNS + relay + content | DONE |
| Type safety | All manifest fields typed, `validate.rs` wired, `FromStr` for all CLI enums | DONE |
| Code quality | 1081 tests, zero clippy warnings (pedantic), all files <800L | DONE |
| Security | SIGN-01 depot signing (BLAKE3 + ed25519), fail-closed sandbox, ELF DT_NEEDED enforcement | DONE |
| Cross-platform | OS Atheism Phase 1+2: `Platform` types, `TransportEndpoint::NamedPipe`, `InitSystem::detect()` | DONE |
| Dependencies | `nix` eliminated, `#![forbid(unsafe_code)]`, zero production `unwrap()`, CSPRNG via `getrandom` | DONE |

---

## Sovereignty Shadow Status

| Track | Sovereign Component | Commercial Shadow | Status | Cutover Gate |
|-------|--------------------|--------------------|--------|--------------|
| S1 TLS | Caddy + LE | Cloudflare (INACTIVE) | **OPERATIONAL** (13d clean, 7-day gate passed) | Graduated |
| S2 NAT relay | Songbird TURN :3478 | cloudflared | **LIVE** | 7-day 100% reachable |
| S3 Content | NestGate + petalTongue | GitHub Pages | **LIVE** (68ms TTFB) | 7-day TTFB parity |
| S4 Auth | BearDog BTSP dual-auth | OAuth2/PAM | **GRADUATED** | 7-day p95 < 50ms |

---

## Escalation Ladder

| Phase | Deliverable | Status |
|-------|-------------|--------|
| 0 | Relay only | Superseded |
| 0.5 | Relay + RustDesk + multi-gate SSH | Completed May 14 |
| 1 | Tower composition | Completed May 18 |
| 1.5 | Nest Atomic + Channel 1 DNS + TLS + VPS Standard + Deep Debt | Completed (Wave 57) |
| **2** | **NUCLEUS (13 primals) + biomeOS + WAN depot + aarch64 + deterministic deployment** | **Stadial-ready** (Wave 107, 2026-06-10) |
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
- VPS deployment ops — systemd units, UDS probes, firewall, refresh cycles
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
        config/               # membrane.toml parser + validator + DeployPaths
        credentials.rs        # age / BTSP vault / manual
        cytoplasm.rs          # ZoneLabel, mesh address, BOOTSTRAP_GATES
        envelope.rs           # K-Derm topology — monoderm/diderm, bonding, channel proteins
        error.rs              # Typed ConfigError (thiserror)
        caddy.rs              # Caddyfile generation from manifest roles
        firewall.rs           # UFW + nftables rules from composition
        identity.rs           # Family ID, gate ID, GateMobility, BindMode, GateRole
        wireguard.rs          # WireGuard wg-quick config generation from manifest peers
        provider.rs           # DigitalOcean / Hetzner / bare metal / gate-local
        service/              # Static service registry + path constants
          mod.rs              # Types, enums, ServicePaths, env vars, path constants
          registry.rs         # 17 const service entries + ALL_SERVICES array
        arch.rs               # Platform, TargetOs, CpuArch, LinkModel (OS Atheism)
        process.rs            # ServiceStatus, InitSystem, ServiceOutcome
        transport.rs          # TransportEndpoint (UDS, TCP, NamedPipe, MeshRelay)
        signal.rs             # Ribocipher signal types
        signing.rs            # DepotSignature, DepotTrustPolicy, SignaturesFile
        sync.rs               # Sync config, GateTransport, CascadeSource
        topology.rs           # TopologyMap TOML parser
        validation.rs         # Report pattern (pass/fail/warn) + doc-tests
    membrane-shadow/          # Sovereign shadow functions CLI (#![forbid(unsafe_code)])
      src/
        dispatch/             # CLI command router (8 domain submodules)
          mod.rs              # Top-level run() router + rootpulse + Neural Bridge
          temporal.rs         # cascade, check, sync dispatch
          impulse.rs          # impulse + potential sense dispatch
          infra.rs            # repo, mirror, service, token (remote VPS API)
          gate.rs             # gate status, health, bootstrap, provision
          data.rs             # manifest, identity, context, plasmid, relay, topology
          plasmid_dispatch.rs # plasmid.harvest, depot_sync, pipeline, trigger
          relay_dispatch.rs   # relay.run/mediate/ship dispatch
          sovereign.rs        # sovereignty + sovereign deploy dispatch
        gate/                 # Gate operations (modular)
          bootstrap.rs        # Local deployment (per-phase timeouts, spawn_blocking)
          enroll.rs           # Mesh enrollment (WG keygen, config, Forgejo-first remotes)
          health.rs           # Native async UDS probes + rootpulse + status
          verify.rs           # Dual checksum verification (git + WAN)
          mesh.rs             # Mesh peer configuration (transport, songbird UDS)
          nucleus.rs          # NUCLEUS systemd management (unit generation, secrets)
          local.rs            # Shared helpers (identity via identity::resolve, depot paths)
          interface.rs        # Network interface detection (sysfs + /proc/net)
          preflight.rs        # Pre-bootstrap checks (ports, services, ARP)
          sovereignty.rs      # Sovereignty verification probes
        relay.rs              # K-Derm relay chain (SSH+cat, no rsync)
        ssh.rs                # SSH transport (exec, raw, on_host, cat_remote, scp)
        git_ops.rs            # Git operations (add/commit/push, rev-parse, reconcile)
        impulse/              # Inter-gate impulse (native UDS JSON-RPC)
        temporal/             # Temporal sync + cascade + post_sync rootpulse
        freshness.rs          # Wave freshness, current_wave(), binary drift detection
        context.rs            # Context braid lifecycle
        plasmid/              # Primal binary lifecycle
          mod.rs              # Registry-derived primal list, graceful_kill, shared utils
          depot.rs            # Depot resolution, sources.toml auto-provision
          depot_sync.rs       # Depot sync (VPS ↔ local, --push mode)
          fetch.rs            # Fetch + WAN checksum verification + BLAKE3
          harvest.rs          # Build + checksum + sign + atomic publish to git
          harvest_manifest.rs # Manifest build config integration
          signing.rs          # Depot signing (BLAKE3 + ed25519 via bearDog UDS)
          sandbox.rs          # Ephemeral isolated validation
          canary.rs           # Previous-good pool (retire → failover)
          drift.rs            # Source divergence detection
          download.rs         # SSH + WAN binary download
          toolchain.rs        # ELF validation + NDK cross-compile + strip
        caddy/                # Caddy TLS + depot provisioning (deprecated Wave 132)
        gateway/              # Tower HTTP gateway (Caddy replacement)
        webhook/              # Webhook receiver (Forgejo + GitHub cascade wiring)
        bridge.rs             # Neural API bridge (UDS discovery)
        jsonrpc.rs            # Centralized JSON-RPC client (UDS, TCP, relay)
        resolve.rs            # Transport endpoint resolution
        ribocipher.rs         # Cryptographic functions (HKDF, HMAC, CSPRNG)
        identity.rs           # Gate identity resolution (canonical)
        config.rs             # ShadowConfig resolution
        manifest/             # Ecosystem manifest parser
          mod.rs              # EcosystemManifest, GateProfile, load/resolve
          validate.rs         # 11-check cross-field manifest validation
          wave.rs             # WaveState lifecycle + ExitCriterion
        sovereignty_ledger.rs # rootpulse sovereignty ledger
  specs/                      # Formal architecture specs (6 documents)
  config/                     # capability_registry.toml (specification artifact)
  deploy/                     # Systemd units, hooks, provisioning
  experiments/                # Validated experiment records (fossil record)
  .forgejo/workflows/ci.yml   # Forgejo CI pipeline
```

---

## Testing

1,081 tests cover types, manifest validation, dispatch, git_ops, cascade, plasmid,
enrollment, and sovereignty. All tests are inline (`#[cfg(test)]`) — no external fixtures.

```bash
cargo test                  # Full suite (1081 tests)
cargo clippy                # Pedantic + nursery, zero warnings
cargo doc --open            # Full API docs
```

Wave-by-wave evolution history is preserved in `GLACIAL_SHIFT_TRACKER.md` and git log.

---

## Related Resources

| Resource | Location | Relationship |
|----------|----------|-------------|
| Ecosystem manifest | `infra/wateringHole/ecosystem_manifest.toml` | Single source of truth for all primals, repos, gates |
| Channel architecture | `infra/wateringHole/MEMBRANE_CHANNEL_ARCHITECTURE.md` | Channel isolation, port policy, crypto layers |
| fieldMouse spec | `infra/wateringHole/CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md` | Deployment class, hardening checklist, boot order |
| K-NOME programming | `infra/whitePaper/gen3/about/K_NOME_PROGRAMMING.md` | K-Derm topology parallels K-NOME methodology |
| Dark Forest standard | `infra/wateringHole/DARK_FOREST_GLACIAL_GATE_STANDARD.md` | 5-pillar security audit |
| Glacial readiness | `infra/wateringHole/GLACIAL_SHIFT_READINESS.md` | 6 stadial entry criteria |
| Fossil record | `infra/fossilRecord/cellMembrane/` | Archived Wave 59/119 scripts (deploy, provision) |

---

## License

scyBorg triple license:
- **AGPL-3.0-or-later** — code (Rust, TOML, shell scripts, tests)
- **ORC** — coordination patterns and mechanics
- **CC-BY-SA 4.0** — documentation and creative content
