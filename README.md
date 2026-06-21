# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (ironGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | NUCLEUS (13 primals: Tower + Nest + Compute + Meta) + RustDesk |
| **Escalation** | Phase 2 (NUCLEUS) — **stadial-ready** (Wave 107+, through Wave 120) |

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
cargo test                  # 731 tests — pedantic clippy clean
cargo clippy                # Zero warnings (pedantic + nursery + option_if_let_else)
cargo doc --open            # Full API documentation with doc-tests
```

**Wave 120 (Deployment Isomorphism — Identity-Based Resolution):**
`topology.service` identity-based service discovery — find any service by role, not host.
Manifest-authoritative `wg_ip` overrides `BOOTSTRAP_GATES` static registry. `wireguard.generate`
produces `wg-quick` configs from manifest peers. `caddy.generate` renders Caddyfile from manifest
roles + topology hosts. `topology.roles` maps all service→gate assignments. `topology.mesh` now
prefers manifest IP. pepti decommissioned (Wave 120). `GateProfile.roles` and `GateProfile.wg_ip`
fields. `gate.validate` composition-tier trust barrier validation — generic evolution of
`pepti.validate`, validates any gate against its declared composition. `wireguard.*` dispatch
routing fixed. Dependency upgrades: `toml` 0.8→1.x, `nix` 0.29→0.31. Manifest-first
federation peer resolution (`resolve_federation_peer()`). `to_nftables_script` refactored
into chain helpers. Deep debt: `.leak()` memory debt eliminated (owned `String` gate identity),
`HEALTH_REQUEST` const centralized (4 call sites), corrupt TOML parse now warns instead of
silently resetting, `CanaryStalenessReport` disambiguated from depot variant,
`FirewallProtocol` derives `Ord` (removed manual `as u8` cast), mesh response parsing
deduped. 731 tests, zero clippy.

**Wave 119+ (Native Detection + Error Normalization):**
Shell-outs evolved to native Rust: `ss` → `/proc/net/{tcp,udp}`, `ip link/addr` → sysfs +
`/proc/net/route`, `systemctl is-active` → cgroup detection. `ShadowError::Parse` normalized
to `Config`/`Ssh`/`Io` across 29 files (22 genuine `Parse` remain). `.expect()` →
`let-else + unreachable!()` in ribocipher. `PLASMID_BIN_DIR` constant eliminates 8 hardcoded
literals. `reqwest` errors use `?` via `From` impl. 711 tests, zero clippy.

**Wave 116–118 (Deep Debt Evolution + Topology Convergence):**
Webhook cascade wiring (Forgejo→`temporal.sync`, GitHub→`relay.mediate`). Manifest-driven
cascade repos (replaces static list). `rootpulse.commit/verify/status` dispatch + gate health
probe. SSH consolidated into `ssh.rs` extensions (`exec_on_host`, `cat_remote`, `scp_to_host`).
Git ops centralized through `git_ops.rs` (`git_output_opt`, `head_short`). `current_wave()`
deduplicated into canonical `freshness.rs` helper. Gate identity unified through
`identity::resolve()` with `tracing::warn!` on fallback. All hardcoded `infra/*` paths replaced
with constants. `CytoplasmZone` split to `cytoplasm.rs` with `ZoneLabel` + `BOOTSTRAP_GATES`.
Topology-aware mesh discovery. 680 tests, zero clippy.

**Wave 115 (Sovereign Mesh & Gate Hardening):**
`gate.bootstrap` per-phase timeouts, identity detection, depot integrity, bootstrap smart
refactor (861L→555L), `spawn_blocking` for all fs ops. 539 tests, zero clippy.

**Wave 107–109 (Deterministic Deployment + guideStone):**
`gate.bootstrap` (6 phases), `gate.status`, `plasmid.build`, `gate.profile`,
`deployment.toml`, BUILD-ELF-01.

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
membrane gate.bootstrap <name> [--dry-run] [--mobile]  # Profile-driven enrollment (7 phases)
membrane gate.profile <name>              # Read gate profile from ecosystem_manifest.toml
membrane temporal.cascade                 # Manifest-driven cascade sync (38 repos)
membrane temporal.cascade --with-restart  # Cascade + fetch + restart updated primals
membrane temporal.cascade --with-rebuild  # Cascade + harvest stale + push to VPS
membrane plasmid.build <primal> [--target T]  # guideStone-grade single-primal build
membrane plasmid.fetch --source wan       # WAN HTTPS fetch + dual BLAKE3 verification
membrane plasmid.harvest                  # Build + checksum + auto-publish to git
membrane plasmid.harvest --target aarch64-linux-android  # NDK cross-compile
membrane plasmid.ndk.check                # Verify NDK toolchain readiness
membrane plasmid.refresh                  # Push depot binaries to VPS (atomic replace)
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

# Build + push + auto-publish checksums
membrane plasmid.harvest && membrane plasmid.refresh

# SSH to VPS
ssh root@$VPS_IP "journalctl -u beardog-membrane -u songbird-membrane -f"
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
| Cross-topology validation (Wave 106): gate.bootstrap, cascade auto-fetch, mesh persistence, 3-gate collective, deterministic deploy standard | DONE |
| Post-stadial hardening (Wave 107): gate.status, --dry-run, WAN checksums, source divergence fix, atomic publish, checksum coherence | DONE |
| guideStone convergence (Wave 109): plasmid.build, gate.profile, deployment.toml, JSON-RPC health, BUILD-ELF-01 | DONE |
| Deep debt evolution (Wave 110): native UDS (tokio::net::UnixStream), gate/ modular split, agnostic config, agentic resolver, dual checksum, cascade-restart | DONE |
| Primal composition grade (Wave 110+): ServiceCapability discovery, temporal/resolve.rs + plasmid/toolchain.rs extracted, all paths env-configurable, DRY socket resolution, zero prod unwrap/TODO/allow | DONE |
| Sandbox NUCLEUS + canary pool (Wave 110+): sandbox.rs ephemeral validation, canary.rs fallback pool, atomic blue/green promotion, cascade-restart with canary retirement, service/registry.rs extracted | DONE |
| riboCipher + dispatch SRP (Wave 111): mito-tier HKDF+HMAC complete, dispatch gate.rs extracted (762L→264+518L), error propagation modernized, Neural API constants shared, 391 tests | DONE |
| Deep debt evolution (Wave 113): zero-copy temporal (Arc manifest, Cow defaults), idiomatic Rust (safe casts, structured logging), ecosystem compliance (SPDX headers, cargo-deny CI) | DONE |
| Bootstrap robustness + depot integrity (Wave 115): per-phase timeouts, identity detection, depot.integrity command, bootstrap 861→555L smart refactor, spawn_blocking for fs ops, 514 tests | DONE |
| Gate enrollment + topology convergence (Wave 116): preflight checks, InterfaceRole, ARP probes, CytoplasmZone→ZoneLabel split, topology-aware mesh, 5-node WG mesh, 620 tests | DONE |
| Deep debt consolidation (Wave 116–118): webhook cascade wiring, rootpulse sovereignty pipeline, SSH/git_ops consolidation, manifest-driven cascade repos, current_wave dedup, identity unification, hardcoded path constants, 680 tests | DONE |
| Native evolution (Wave 119+): ss→/proc/net, ip→sysfs, systemctl→cgroup, ShadowError normalization (Parse→Config/Ssh/Io), .expect()→let-else+unreachable, PLASMID_BIN_DIR constant, reqwest From impl, 711 tests | DONE |
| Deployment isomorphism (Wave 120): topology.service identity-based discovery, manifest-driven mesh IP (wg_ip), WireGuard config generation, Caddyfile generation from roles, topology.roles command, pepti decommission, 729 tests | DONE |
| Deep debt sweep (Wave 120): `.leak()` → owned String, HEALTH_REQUEST const, TOML parse warnings, CanaryStalenessReport rename, derived Ord, mesh parsing dedupe, `format_resolved` test-only, 731 tests | DONE |
| Pipeline dedupe (Wave 120): build/harvest unified — shared `stage_to_depot_async`, `drift::clone_source`, `git_ops::head_short`. Service port constants (`DEFAULT_FORGEJO_HTTP_PORT`, `DEFAULT_DEPOT_HTTP_PORT`). BLAKE3 hash failure sentinel, 731 tests | DONE |

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
        identity.rs           # Family ID, gate ID
        wireguard.rs          # WireGuard wg-quick config generation from manifest peers
        provider.rs           # DigitalOcean / Hetzner / bare metal / gate-local
        service/              # Static service registry + path constants
          mod.rs              # Types, enums, ServicePaths, env vars, path constants
          registry.rs         # 17 const service entries + ALL_SERVICES array
        signal.rs             # Ribocipher signal types
        sync.rs               # Sync config, GateTransport
        topology.rs           # TopologyMap TOML parser
        validation.rs         # Report pattern (pass/fail/warn) + doc-tests
    membrane-shadow/          # Sovereign shadow functions CLI (#![forbid(unsafe_code)])
      src/
        dispatch/             # CLI command router (7 domain submodules)
          mod.rs              # Top-level run() router + rootpulse + Neural Bridge
          temporal.rs         # cascade, check, sync dispatch
          impulse.rs          # impulse + potential sense dispatch
          infra.rs            # repo, mirror, service, token (remote VPS API)
          gate.rs             # gate status, health, bootstrap, provision
          data.rs             # manifest, identity, context, plasmid, relay, topology
          relay_dispatch.rs   # relay.run/mediate/ship dispatch
        gate/                 # Gate operations (modular)
          bootstrap.rs        # Local enrollment (per-phase timeouts, spawn_blocking)
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
          fetch.rs            # Fetch + WAN checksum verification + BLAKE3
          harvest.rs          # Build + checksum + atomic publish to git
          sandbox.rs          # Ephemeral isolated validation
          canary.rs           # Previous-good pool (retire → failover)
          drift.rs            # Source divergence detection
          download.rs         # SSH + WAN binary download
          toolchain.rs        # ELF validation + NDK cross-compile + strip
        caddy/                # Caddy TLS + depot provisioning
        webhook/              # Webhook receiver (Forgejo + GitHub cascade wiring)
        bridge.rs             # Neural API bridge (UDS discovery)
        identity.rs           # Gate identity resolution (canonical)
        config.rs             # ShadowConfig resolution
        manifest.rs           # Ecosystem manifest parser
        sovereignty_ledger.rs # rootpulse sovereignty ledger
  specs/                      # Formal architecture specs (6 documents)
  config/                     # capability_registry.toml
  deploy/                     # Systemd units, hooks, provisioning
  experiments/                # Validated experiment records (fossil record)
  .forgejo/workflows/ci.yml   # Forgejo CI pipeline
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
| Deploy script | `infra/plasmidBin/deploy_membrane.sh` | Legacy — fully replaced by `membrane` CLI (fossil record) |
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
