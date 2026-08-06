# cellMembrane

**Operational repo + typed Rust library for the cellMembrane — sovereign external surface of the ecoPrimals ecosystem.**

| | |
|-|-|
| **Owner** | cellMembrane team (sporeGate) |
| **Class** | fieldMouse — Nest Atomic on external substrate |
| **Role** | Rendezvous broker, never data plane |
| **VPS** | `membrane-relay`, Debian 12 x64, DigitalOcean nyc1 ($12/mo) |
| **Composition** | NUCLEUS (13 primals: Tower + Nest + Compute + Meta) + RustDesk, 10-gate mesh |
| **Escalation** | Phase 2 (NUCLEUS) — **stadial-ready** (Wave 107+, through Wave 156m) |

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
- Subdomain routing: `sporeprint.primals.eco`, `webb.primals.eco`, `depot.primals.eco`
- 19 MB sporePrint content cache synced from NestGate
- Sovereignty proof: 68ms TTFB (vs GitHub Pages 89ms)
- Root domain `primals.eco` redirects to `sporeprint.primals.eco`

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
cargo test                  # 1293 tests — pedantic clippy clean
cargo clippy                # Zero warnings (pedantic + nursery + option_if_let_else)
cargo doc --open            # Full API documentation with doc-tests
```

Current state (Wave 155r): ~11k lines types, ~38k lines shadow. Crash-loop breaker
detects and disables services stuck in restart loops (Wave 150x: nestgate 17,920 restarts,
biomeos-beacon 11,161 restarts — ISP throttled the gate). `tower.shadow` command ships
continuous WG vs Tower transport shadow metrics across the mesh.
All manifest fields type-safe (`GateRole`, `CascadeSource`, `GateMobility`, `BindMode`,
`EnvelopeTopology`, `MembraneComposition`, `Platform`, `TargetArch`, `TransportEndpoint`).
Rich cross-field validation wired (`validate.rs`). SIGN-01 depot signing pipeline
(BLAKE3 + ed25519). Fail-closed sandbox. ELF DT_NEEDED enforcement. Sovereign-first
drift detection. OS Atheism Phase 1+2 (platform types, named pipes, process lifecycle).
10-gate mesh topology (7 WG-enrolled: golgi, sporeGate, eastGate, flockGate,
ironGate, northGate, southGate; 3 pending: blueGate, westGate, grapheneGate).
Subdomain standard (`prefix.primals.eco`): `webb.primals.eco` vhost, CSP headers,
root domain redirect to `sporeprint.primals.eco`, depot at `depot.primals.eco`.
`gate.enroll` automated mesh enrollment + `hub.peer` hub-side addition.
Sovereign depot auto-build pipeline (Wave 150v): reactive CI trigger (Forgejo
post-receive), convergent drift detection, hard lineage enforcement for
`PostPrimordial` primals, build-pending mesh signal.
Depot provenance (Wave 151a): `provenance.toml` builder attribution uses gate
identity (`GATE_NAME` / `.gate`) instead of OS hostname. Multi-target harvest
wired from `[build.<primal>].targets` manifest field — drives x86_64 + aarch64
builds without CLI `--target` override. `plasmid.status` drift alarm warns when
depot is >7 days stale.
Cross-platform NUCLEUS (Wave 155b): `nucleus.rs` evolved from systemd-only to
`InitSystem::detect()` dispatch — systemd unit generation on Linux, bare process
spawn with PID file tracking on Windows/macOS/containers. `stop_bare_process()`,
`restart_bare_process()` for non-systemd lifecycle. `nucleus_restart.rs` refactored
to `converge_primal()` extraction (per-primal convergence with `ConvergeOutcome`
enum). `crash_loop.rs` guards systemd-only scan with `InitSystem` check. Dead code
cleanup in `verify.rs` (removed orphaned `ChecksumFile`/`ChecksumEntry` structs
after migration to shared `parse_checksums_toml()`).
Fleet convergence (Wave 155b): `gate/verify.rs` checksum verification migrated
from private `ChecksumEntry` to shared `parse_checksums_toml()` — handles both
struct `{ blake3 = "...", size = N }` and legacy plain-string `"hash"` formats.
`checksum` module promoted to `pub(crate)`. blueGate (distributed builder) and
westGate (cold storage) added to `MESH_REGISTRY`, `KNOWN_GATES`, and zone
fallbacks. Build authority foreman pattern already supported via
`ENV_BUILD_AUTHORITY` + manifest `build_authority` field. Composition profiles
validated: `compute` → Tower trust, `nest` → Nest trust, manifest profiles
control primal deployment lists.
Jelly string codification (Wave 155d): J1+J2 closed — `plasmid.push` first-class
command (was hidden behind `depot_sync --push`), `plasmid.harvest --push` flag
combines harvest→push in one command. Default `depot_sync` refactored from
monolithic embedded bash `for` loop to Rust-orchestrated per-primal SSH commands
(`sync_single_remote()` with per-binary BLAKE3 diff + atomic copy).
J6 completion (Wave 155f): `gate.configure` and `gate.apply` CLI commands —
declarative service config generation from gate composition profile. Reads
`ecosystem_manifest.toml`, builds `ServiceSpec` for each primal in the
composition, renders to detected init system (systemd/launchd/bare), and
optionally installs. Supports `--env K=V` overrides. Extracted to
`gate_configure.rs` module. `ServiceSpec` foundation (Wave 155d) wired through
to full CLI surface. `nucleus.rs` helpers (`systemctl`, `resolve_security_socket`,
`extra_exec_args`) promoted to `pub(crate)` for cross-module use.
Glibc depot target (Wave 155i): `targets_for_primal()` now auto-appends
`x86_64-unknown-linux-gnu` for GPU primals even when manifest `targets` is
explicit — closes P0 musl/glibc `dlopen` gap for compute primals
(barraCuda, coralReef) on strandGate RTX 3090.
WireGuard DNS (Wave 155i): `WgConfig` gains `dns` field, `to_wg_quick()` emits
`DNS =` in `[Interface]`, `manifest_to_wg_config()` resolves hub mesh IP as
DNS server. Fixes P1 WireGuard DNS catch-all.
J8 foundation (Wave 155f): SSH certificate lifecycle via sovereign step-ca CA.
`gate/key_portal.rs` module: `request_ssh_certificate()`, `renew_ssh_certificate()`,
`install_host_certificate()`, `inspect_certificates()`, `bootstrap_ca()`. New CLI
commands: `gate.keys` (status), `gate.keys.renew` (user/host cert renewal),
`gate.keys.renew --bootstrap` (CA trust init). `SshCertificate` and `SshCertType`
types in `cellmembrane-types/src/credentials.rs`. `CredentialModel::StepCa` variant.
step-ca constants (`DEFAULT_STEP_CA_URL`, `ENV_STEP_CA_FINGERPRINT`, etc.) in
`service/constants.rs`. `gate.enroll` phase 8 (`ssh_cert`) wires cert request into
enrollment flow (non-fatal if step-ca not deployed). Deployment team handoff for
step-ca on golgiBody (deployment pending sporeGate team).
Deep debt (Wave 155d–155f): Tower port `7780` → `DEFAULT_TOWER_PORT` +
`ENV_TOWER_PORT` constants. Bootstrap `write_gate_identity` arch triple →
`detect_target_triple()`. duplicate federation port const eliminated
(`tower/mod.rs` → `DEFAULT_FEDERATION_PORT`). Enroll `:7700` literal →
constant. `crash_loop.rs` `query_unit_restart_info` defense-in-depth
`InitSystem` guard. `gateway/mod.rs` domain fallback → `SURFACE_DOMAIN`.
Hardcoded arch triple in `tower/timer.rs` → `detect_target_triple()`.
BTSP evolution (Wave 151b): `btsp_client.rs` implements the 4-step `ClientHello`
handshake (HMAC-SHA256 challenge-response via `FAMILY_SEED`). All bearDog UDS
clients (`signing.rs`, `impulse/primal.rs`, `jsonrpc.rs`) now perform BTSP
handshake before crypto requests. Graceful fallback to plain JSON-RPC during
transition. Tower status probes use BTSP for bearDog socket.
Deep debt sweep (140a–151b): unified mesh registry (`MESH_REGISTRY` const table),
shared canary/sandbox staging, capability-based naming, visibility tightened,
allocation hot paths optimized, error taxonomy reclassified, domain constants
centralized, CAC tree-parity checks, CSPRNG unified via `getrandom` (0.4), service
filter registry-derived, `ProbeResult` typed gate probes, zero f64 casts,
nested `if let` → let-chains (Rust 2024 edition), timestamp/HTTP helpers centralized.
Large test extraction: `manifest/mod.rs` (785→333L), `webhook/mod.rs` (703→345L),
`gateway.rs` (833→371L), `harvest.rs` (804→422L), `post_sync.rs` (791→716L),
`plasmid/mod.rs` (788→662L) — all tests in dedicated files.
Zero files >800L. `MESH_REGISTRY` extended with `lan_ip` field for LAN peer
discovery (eastGate `192.168.4.244`, sporeGate `192.168.4.3`).
Hardcode elimination: tower timer sockets/paths, enroll hub IP, relay sovereign
remote, shadow domain literal, hub mesh IP — all resolved via capability registry
or centralized constants (`LAB_DOMAIN`, `DEFAULT_HUB_MESH_IP`).
`as` casts → `try_from`/`f64::from`. Unused `portable-atomic` dep removed.
Deep debt sweep (Wave 155i): P0 sandbox fail-closed fix (`sandbox_validate`
returned `true` on infra `Err`, now correctly fails deploy). Tower status
refactored from hardcoded primal names to registry-driven
`MembraneComposition::Tower.spec()` — discovers services by composition,
probes by capability (BTSP for `CryptoSigner`). Unified `resolve_primal_socket`
replaces 3 per-primal resolvers + duplicate in `enroll.rs`. `run_step()` helper
extracts 5 duplicate `step` CLI blocks in `key_portal.rs`. `detect_crash_loops()`
extracts shared scanning from 3 crash-loop variants. `push_depot_to_remote()`
deduplicates ~80 lines across depot push paths. Let-chains (`gate_configure.rs`,
`firewall.rs`). Net -135 lines.
Sovereign CI polish (Wave 155k): `membrane.exe` Windows cross-compile fix —
`jsonrpc.rs` UDS functions gated with `#[cfg(unix)]`, `#[cfg(not(unix))]` stubs
return platform-specific errors. `mesh.build_pending` wired to songBird UDS
(was tracing-only). UDS webhook listener (`webhook/listener.rs`) accepts
Forgejo/GitHub HTTP POSTs, verifies HMAC-SHA256 signatures, dispatches to
`handle_push()` pipeline. DNS manifest generators (`dns.configure`/`dns.apply`)
shipped — knot-dns zone file + config generation from `ecosystem_manifest.toml`.
User-space deploy readiness (Wave 155k): `MEMBRANE_INIT_SCOPE` env override
for init system selection (`bare`/`user`/`systemd`) — enables deployment on
read-only rootfs (`SteamOS` Steam Deck), non-root users, or containerized
environments. `prepare_socket_base()` respects `MEMBRANE_SOCKET_BASE`. All
`systemctl` helpers auto-inject `--user` when `MEMBRANE_INIT_SCOPE=user`.
Systemd unit dir resolves via `MEMBRANE_SYSTEMD_UNIT_DIR` or init scope
(user scope → `$HOME/.config/systemd/user`). `WantedBy` target adapts
(`multi-user.target` for system, `default.target` for user). Hardcoded
`/run/membrane` and `/var/lib/membrane` paths replaced with `MEMBRANE_SOCKET_BASE`
resolution. Bootstrap permissions phase uses env-resolved socket base.
Zero production `unwrap()` (test-only, confirmed via full audit).
Zero `unsafe` code (`#![forbid(unsafe_code)]` on all crates).
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
membrane gate.keys                                     # Show SSH certificate status (user + host)
membrane gate.keys.renew [--dry-run]                   # Renew SSH user cert from step-ca
membrane gate.keys.renew --host <hostname>             # Request/renew host certificate
membrane gate.keys.renew --bootstrap                   # Bootstrap step-ca trust on this gate
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
membrane gate.configure [--env K=V]       # Preview systemd/launchd units from manifest
membrane gate.apply [--env K=V]           # Write + enable service units
membrane dns.configure [--gate G]         # Preview knot-dns config + zone files from manifest
membrane dns.apply [--gate G] [--dry-run] # Write zone files + knot.conf and reload knot-dns
membrane webhook.listen [--socket PATH]   # UDS webhook listener (Forgejo/GitHub POSTs)
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
through Wave 155r are **DONE**. Full wave-by-wave audit trail is preserved in
`GLACIAL_SHIFT_TRACKER.md` and git log.

| Category | Summary | Status |
|----------|---------|--------|
| Infrastructure | exim4/droplet-agent purged, fail2ban, UFW, SSH key-only, journald persistence | DONE |
| TLS | Caddy + Let's Encrypt sovereign TLS, Cloudflare removed | DONE |
| Dark Forest | 21/21 PASS, 5-pillar compliance, stripped static ELF binaries | DONE |
| NUCLEUS | 13/13 primals ALIVE, 7-node WG mesh (10 named, 3 pending), UDS-only, sandbox + canary pipeline | DONE |
| Sovereignty | S1–S4 all GRADUATED, BTSP enforced, sovereign DNS + relay + content | DONE |
| Type safety | All manifest fields typed, `validate.rs` wired, `FromStr` for all CLI enums | DONE |
| Code quality | 1293 tests, zero clippy warnings (pedantic), all files <800L | DONE |
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
- Multi-gate expansion (10-gate mesh: 7 WG-enrolled + 3 pending enrollment)
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
        credentials.rs        # age / BTSP vault / manual / step-ca SSH certs
        dns.rs                # DNS zone + knot config types (zonefile renderer)
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
          nucleus.rs          # Cross-platform NUCLEUS (systemd + bare process, PID files)
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
          signing.rs          # Depot signing (BLAKE3 + ed25519 via CryptoSigner UDS)
          sandbox.rs          # Ephemeral isolated validation
          canary.rs           # Previous-good pool (retire → failover)
          drift.rs            # Source divergence detection
          download.rs         # SSH + WAN binary download
          toolchain.rs        # ELF validation + NDK cross-compile + strip
        caddy/                # Manifest-driven Caddy config generation + TLS + depot
        dns/                  # Sovereign DNS (knot-dns zone + config generation)
        gateway/              # Tower HTTP gateway (Caddy replacement)
        webhook/              # Webhook receiver + UDS listener (Forgejo + GitHub)
        btsp_client.rs        # BTSP ClientHello handshake (bearDog auth)
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
  receipts/                   # Operational receipts (key generation, deploy)
  .forgejo/workflows/ci.yml   # Forgejo CI pipeline
```

---

## Testing

1,273 tests cover types, manifest validation, dispatch, git_ops, cascade, plasmid,
enrollment, sovereignty, BTSP, checksum verification, DNS, HTTP client, and user-space deploy.
Tests use both inline `#[cfg(test)]` modules and dedicated test files
(`gateway_tests.rs`, `harvest_tests.rs`, `manifest/tests.rs`, `webhook/tests.rs`)
— no external fixtures.

```bash
cargo test                  # Full suite (1293 tests)
cargo clippy                # Pedantic + nursery, zero warnings
cargo doc --open            # Full API docs
```

Wave-by-wave evolution history is preserved in `GLACIAL_SHIFT_TRACKER.md` and git log.

---

## Related Resources

| Resource | Location | Relationship |
|----------|----------|-------------|
| Ecosystem manifest | `infra/wateringHole/ecosystem_manifest.toml` | Single source of truth for all primals, repos, gates |
| Channel architecture | `infra/wateringHole/compositions/MEMBRANE_CHANNEL_ARCHITECTURE.md` | Channel isolation, port policy, crypto layers |
| K-NOME programming | `infra/whitePaper/gen3/about/K_NOME_PROGRAMMING.md` | K-Derm topology parallels K-NOME methodology |
| Dark Forest standard | `infra/wateringHole/foundations/DARK_FOREST_GLACIAL_GATE_STANDARD.md` | 5-pillar security audit |
| Fossil record | `infra/fossilRecord/cellMembrane/` | Archived Wave 59/119 scripts (deploy, provision) |

---

## License

scyBorg triple license:
- **AGPL-3.0-or-later** — code (Rust, TOML, shell scripts, tests)
- **ORC** — coordination patterns and mechanics
- **CC-BY-SA 4.0** — documentation and creative content
