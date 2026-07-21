<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->

# cellMembrane Architecture

**Version**: 1.0.0
**Date**: May 26, 2026
**Status**: Active
**Authority**: cellMembrane team
**License**: AGPL-3.0-or-later
**Related**: `K_DERM_TOPOLOGY.md`, `MEMBRANE_COMPOSITION_MODEL.md`, `FIELDMOUSE_CONTRACT.md`, `infra/wateringHole/compositions/MEMBRANE_CHANNEL_ARCHITECTURE.md`

---

## Abstract

cellMembrane is the controlled boundary between NUCLEUS (the sovereign
interior) and the public internet. It exposes three process-isolated
channels — Signal, Relay, and Surface — each with distinct trust levels,
crypto layers, and port policies. This document defines the membrane
architecture as a typed, deployable system that others can instantiate
for their own sovereign infrastructure.

A membrane is not a primal. It is an **infrastructure composition** that
orchestrates existing primals (BearDog, Songbird, SkunkBat, NestGate, etc.)
into a hardened external surface. The membrane itself adds no domain logic;
it adds isolation, firewall policy, credential management, and channel
discipline.

---

## Membrane Model

### Biological Analogy

A cell membrane is selectively permeable. It does not process information
— it controls what crosses the boundary. NUCLEUS is the cell interior.
The membrane exposes specific receptors (channels) to the extracellular
environment (the public internet) while maintaining intracellular integrity
(BTSP, family seed, UDS IPC).

### K-Derm Topology (Cell Envelope Model)

The single-membrane biological analogy above describes a monoderm — one
boundary between cytoplasm and environment. Production deployments use a
**diderm** topology: two membrane boundaries with a periplasmic space
between them. See `specs/K_DERM_TOPOLOGY.md` for the full specification.

Absolute layer naming (inside out):

```
cytoplasm (gate NUCLEUS, UDS IPC)
  → plasma membrane (gate firewall)
    → periplasm (VPS relay, routing, telemetry, attribution)
      → outer membrane (VPS channels: Signal, Relay, Surface)
        → extracellular (public internet)
```

The VPS is always in the periplasm + outer membrane position. Gates own
the plasma membrane. This replaces the ambiguous "inner/outer membrane"
terminology that conflicted across documents (see K-Derm spec §5:
Terminology Reconciliation).

| Topology | Structure | Example |
|----------|-----------|---------|
| Monoderm | Gate → environment (no VPS) | ironGate on home LAN |
| Diderm | Gate → VPS periplasm → environment | Production `membrane-relay` |
| Nested diderm | One system's outer membrane = another's periplasm | University lab inside campus |

The K-Derm topology field in `membrane.toml` selects the envelope:

```toml
[membrane]
topology = "diderm"  # or "monoderm"
```

### Architectural Invariants

1. **Three channels only.** All external traffic enters through exactly one
   of Signal, Relay, or Surface. No additional ports are exposed.
2. **Process isolation.** Each channel runs in a separate process with its
   own systemd unit. No shared state, sockets, or memory between channels.
3. **Composition-aware firewall.** UFW rules are derived deterministically
   from the composition. Only ports required by the active composition are
   open. Everything else is denied by default.
4. **Rendezvous only.** A membrane is a broker, never a data plane. Content
   is cached, not originated. Keys are delegated, not generated (except
   the TURN credential, which is ephemeral).
5. **Dark Forest compliance.** The substrate provider (VPS host, cloud) is
   treated as a non-family observer. All secrets are encrypted at rest.
   Binaries are stripped static ELFs. No hostnames or paths are embedded.

---

## Channel Architecture

### Channel 1: Signal

| Attribute      | Value                                    |
|----------------|------------------------------------------|
| Function       | DNS resolution for the membrane domain   |
| Primal         | knot-dns (external, not an ecoPrimal)    |
| Port           | 53/tcp, 53/udp                           |
| Trust level    | Lowest — public data, no authentication  |
| Crypto layer   | DNSSEC (when configured)                 |
| Status         | Planned — not yet deployed               |

Signal is the most exposed channel. It serves public DNS records and
requires no authentication. It is the first channel to be probed by
attackers and the last to carry sensitive data.

**Typed contract** (see `channels.rs`):
```
MembraneChannel::Signal {
    port: 53,
    protocol: TcpAndUdp,
    trust_level: Public,
    crypto_layer: None | Dnssec,
    primal: "knot-dns",
}
```

### Channel 2: Relay

| Attribute      | Value                                        |
|----------------|----------------------------------------------|
| Function       | NAT traversal, TURN relay                    |
| Primal         | Songbird                                     |
| Port           | 3478/tcp, 3478/udp                           |
| Trust level    | Medium — metadata visible, content encrypted |
| Crypto layer   | TURN HMAC (extracellular), BTSP (intracellular) |
| Status         | LIVE                                         |

Relay carries opaque encrypted bytes between gates and the membrane.
The TURN protocol exposes connection metadata (source/dest IPs, timing)
but content is BTSP-encrypted end-to-end. The TURN credential is
generated at deploy time and stored in `/etc/songbird/relay-credentials`.

**Sub-channel 2b: RustDesk**

| Attribute      | Value                                     |
|----------------|-------------------------------------------|
| Function       | Remote desktop rendezvous + relay         |
| Primal         | RustDesk hbbs/hbbr (symbiotic, not ecoPrimal) |
| Ports          | 21115/tcp, 21116/tcp+udp, 21117/tcp       |
| Trust level    | Medium — encrypted opaque                  |
| Crypto layer   | RustDesk native encryption                 |

RustDesk is a symbiotic partner — not an ecoPrimal, but deployed alongside
the membrane for remote access sovereignty. Its version is pinned in
`manifest.toml` under `[membrane.rustdesk]`.

### Channel 3: Surface

| Attribute      | Value                                      |
|----------------|--------------------------------------------|
| Function       | HTTPS, content delivery, ACME certificates |
| Primal         | Caddy (TLS termination) + NestGate (content) |
| Ports          | 80/tcp (ACME), 443/tcp (TLS)               |
| Trust level    | Highest — TLS private keys, session state  |
| Crypto layer   | TLS 1.3 (extracellular), BTSP (intracellular) |
| Status         | LIVE — `membrane.primals.eco`              |

Surface is the browser-facing channel. Caddy terminates TLS and reverse-proxies
to NestGate for content delivery. The TLS private key is the most sensitive
asset on the membrane — it must never be exposed to other channels.

---

## Crypto Layers

The membrane maintains two independent crypto boundaries:

### Extracellular (membrane <-> internet)

| Channel | Mechanism      | Key material              |
|---------|----------------|---------------------------|
| Signal  | DNSSEC         | Zone signing key (ZSK)    |
| Relay   | TURN HMAC      | Shared TURN credential    |
| Surface | TLS 1.3        | Let's Encrypt certificate |

These protect traffic between external clients and the membrane surface.
They are visible to the substrate provider (the VPS host can observe TLS
handshakes, TURN metadata, DNS queries).

### Intracellular (membrane <-> NUCLEUS gates)

| Mechanism      | Key material         | Scope                |
|----------------|----------------------|----------------------|
| BTSP           | FAMILY_SEED (Ed25519)| All primal-to-primal |
| ChaCha20-Poly1305 | Session keys      | Per-connection       |

BTSP encrypts all primal-to-primal communication. The substrate provider
sees only opaque bytes. BTSP is enforced when `FAMILY_ID` is set in
`tower.env`. BearDog owns the BTSP implementation; other primals delegate
crypto operations to BearDog via JSON-RPC.

---

## Process Isolation Model

```
┌─────────────── UFW (deny default) ──────────────────┐
│                                                       │
│  ┌─ Channel 1 ──┐  ┌─ Channel 2 ──┐  ┌─ Ch. 3 ────┐│
│  │ knot-dns     │  │ songbird     │  │ caddy       ││
│  │ :53          │  │ :3478        │  │ :80/:443    ││
│  └──────────────┘  │              │  │ nestgate    ││
│                    │ hbbs :21115  │  │ :9500       ││
│                    │ hbbr :21117  │  └─────────────┘│
│                    └──────────────┘                  │
│                                                       │
│  ┌─ Intracellular (not externally reachable) ───────┐│
│  │ beardog   /run/membrane/beardog.sock (UDS only)  ││
│  │ skunkbat  127.0.0.1:9140 (loopback only)        ││
│  │ rhizocrypt, loamspine, sweetgrass (UDS/loopback) ││
│  └──────────────────────────────────────────────────┘│
└───────────────────────────────────────────────────────┘
```

Every channel process runs as a separate systemd unit with:
- `ProtectSystem=strict`
- Memory and CPU limits
- Dedicated binary at `/opt/membrane/{primal}`
- Optional `EnvironmentFile=/opt/membrane/tower.env`

Intracellular primals (BearDog, SkunkBat, provenance trio) are never
exposed to the network. They communicate via Unix domain sockets or
loopback-only TCP. UFW does not open any ports for them.

---

## Firewall Policy

The firewall is **composition-deterministic**: given a composition name,
the exact set of UFW rules is fully determined. No manual port management.

| Rule           | Relay | RustDesk | Tower | Nest |
|----------------|-------|----------|-------|------|
| 22/tcp (SSH)   | Yes   | Yes      | Yes   | Yes  |
| 3478/tcp+udp   | Yes   | Yes      | Yes   | Yes  |
| 21115/tcp      | No    | Yes      | Yes   | Yes  |
| 21116/tcp+udp  | No    | Yes      | Yes   | Yes  |
| 21117/tcp      | No    | Yes      | Yes   | Yes  |
| 80/tcp (ACME)  | No    | No       | No    | Yes  |
| 443/tcp (TLS)  | No    | No       | No    | Yes  |
| 9500/tcp       | No    | No       | No    | Yes  |
| 53/tcp+udp     | No    | No       | No    | *    |

\* Channel 1 Signal ports open only when knot-dns is deployed.

SSH (port 22) is always open but hardened: key-only authentication,
`PasswordAuthentication no`, `PermitRootLogin prohibit-password`.

---

## Permanently External Dependencies

These cannot be eliminated by design and are accepted as permanent
external trust anchors:

| Dependency          | Reason                                    |
|---------------------|-------------------------------------------|
| DNS registrar       | ICANN controls domain registration        |
| Public IP / VPS     | NAT physics — must have a routable address|
| Certificate Authority| Browser trust requires CA-signed certs    |

Everything else is sovereign or on the sovereignty roadmap.

---

## Typed Interface (`cellmembrane-types`)

This architecture is encoded in the `cellmembrane-types` Rust crate:

| Type                  | Module           | Encodes                          |
|-----------------------|------------------|----------------------------------|
| `MembraneChannel`     | `channels.rs`    | Signal / Relay / Surface enum    |
| `ChannelConfig`       | `channels.rs`    | Port, trust, crypto, primal      |
| `MembraneComposition` | `composition.rs` | Relay / RustDesk / Tower / Nest  |
| `CompositionSpec`     | `composition.rs` | Required primals per composition |
| `EnvelopeTopology`    | `envelope.rs`    | Monoderm / Diderm (K-Derm)       |
| `EnvelopeLayer`       | `envelope.rs`    | Cytoplasm → Plasma → Periplasm → Outer → Extra |
| `ChannelProtein`      | `envelope.rs`    | Aquaporin / GatedIon / VoltageGated / Passive  |
| `BoundaryPolicy`      | `envelope.rs`    | Per-layer bond + braid policy    |
| `MembraneService`     | `service.rs`     | Binary, port, systemd unit       |
| `FirewallRuleset`     | `firewall.rs`    | UFW rules from composition       |
| `MembraneIdentity`    | `identity.rs`    | Host, domain, family ID, certs   |
| `ProviderConfig`      | `provider.rs`    | VPS / gate / bare metal          |
| `MembraneReport`      | `validation.rs`  | Validate config against spec     |

The config file `membrane.toml` is the user-facing interface. A third
party writes a `membrane.toml`, validates it with `cellmembrane-types`,
and feeds it to a deployer.

---

## Cross-References

- K-Derm topology: `specs/K_DERM_TOPOLOGY.md`
- K-NOME methodology: `infra/whitePaper/gen3/about/K_NOME_PROGRAMMING.md`
- Channel architecture prose: `infra/wateringHole/compositions/MEMBRANE_CHANNEL_ARCHITECTURE.md`
- fieldMouse deployment class: `infra/wateringHole/fossilRecord/wave132h_jul2026/CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md` (fossilized)
- Dark Forest standard: `springs/primalSpring/specs/DARK_FOREST_GLACIAL_GATE.md`
- Deploy graph: `primals/biomeOS/graphs/membrane_deploy.toml`
- Composition model: `specs/MEMBRANE_COMPOSITION_MODEL.md`
- Deployment contract: `specs/FIELDMOUSE_CONTRACT.md`
- Multi-membrane: `specs/MULTI_MEMBRANE_DEPLOYMENT.md`
