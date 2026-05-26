# Membrane Composition Model

**Version**: 1.0.0
**Date**: May 26, 2026
**Status**: Active
**Authority**: cellMembrane team
**License**: AGPL-3.0-or-later
**Related**: `CELLMEMBRANE_ARCHITECTURE.md`, `infra/plasmidBin/ports.env`

---

## Abstract

A membrane composition defines which primals, channels, ports, and firewall
rules are active on a given membrane host. Compositions form a monotonic
ladder — each tier includes everything from the tier below plus additional
services. This document specifies the four composition tiers and the rules
for extending the model.

---

## Composition Ladder

```
  nest     = tower + NestGate + rhizoCrypt + loamSpine + sweetGrass
    ↑
  tower    = rustdesk + BearDog + SkunkBat (BTSP identity boundary)
    ↑
  rustdesk = relay + RustDesk hbbs/hbbr (remote desktop)
    ↑
  relay    = Songbird TURN (minimal viable membrane)
```

Each composition is a strict superset of the one below. You cannot deploy
`tower` without `relay` capabilities. You cannot deploy `nest` without
`tower`.

---

## Tier 1: Relay

The minimal viable membrane. A single Songbird TURN relay providing NAT
traversal for gates behind restrictive networks.

| Attribute       | Value                                       |
|-----------------|---------------------------------------------|
| Primals         | songbird                                    |
| Channels active | Channel 2 (Relay)                           |
| Ports           | 22/tcp, 3478/tcp+udp                        |
| Systemd units   | `songbird-relay.service`                    |
| Credentials     | TURN key in `/etc/songbird/relay-credentials` |
| Health check    | `songbird doctor` or TURN allocate probe    |
| Dark Forest     | Minimal — no identity, no BTSP              |

**Use case**: Temporary relay for gate-to-gate connectivity when no
identity boundary is needed. Testing. Bootstrapping before Tower is ready.

### `membrane.toml` example

```toml
[membrane]
name = "relay-only"
composition = "relay"

[membrane.channels.relay]
port = 3478
primal = "songbird"
```

---

## Tier 2: RustDesk

Adds RustDesk remote desktop rendezvous and relay on top of Songbird.
This is the first composition that provides direct interactive access
to gates behind NAT.

| Attribute       | Value                                          |
|-----------------|------------------------------------------------|
| Primals         | songbird                                       |
| Symbiotic       | hbbs, hbbr (RustDesk — not ecoPrimals)         |
| Channels active | Channel 2 (Relay), Channel 2b (RustDesk)       |
| Ports           | 22, 3478, 21115, 21116/tcp+udp, 21117          |
| Systemd units   | `songbird-relay`, `hbbs-membrane`, `hbbr-membrane` |
| Credentials     | TURN key + RustDesk key pair (auto-generated)  |
| Health check    | Songbird + `hbbs` TCP connect on 21116         |
| Dark Forest     | Minimal — RustDesk has its own encryption      |

**Use case**: Remote access sovereignty. Replace commercial remote desktop
(TeamViewer, AnyDesk) with self-hosted infrastructure.

---

## Tier 3: Tower

Adds the BTSP identity boundary (BearDog + SkunkBat) on top of RustDesk.
Tower is the trust perimeter — it establishes encrypted, authenticated
primal-to-primal communication via BTSP. This is the standard production
composition for a membrane.

| Attribute       | Value                                               |
|-----------------|-----------------------------------------------------|
| Primals         | beardog, songbird, skunkbat                         |
| Symbiotic       | hbbs, hbbr                                          |
| Channels active | Channel 2, Channel 2b                               |
| Ports           | 22, 3478, 21115, 21116/tcp+udp, 21117               |
| Systemd units   | `beardog-membrane`, `songbird-relay`, `skunkbat-membrane`, `hbbs-membrane`, `hbbr-membrane` |
| Credentials     | TURN key, RustDesk key, `tower.env` (FAMILY_SEED, FAMILY_ID) |
| Boot order      | BearDog → Songbird → SkunkBat (systemd `After=`)    |
| Health check    | `beardog doctor`, `songbird doctor`, SkunkBat liveness |
| Dark Forest     | Full — BTSP enforced, stripped binaries, encrypted secrets |

Tower introduces `tower.env` — the composition identity file containing
`BEARDOG_FAMILY_SEED`, `FAMILY_ID`, `MEMBRANE_ROLE=tower`, and
`MEMBRANE_GATE_ID`. This file is the root of trust for all intracellular
communication.

**Boot order constraint**: BearDog must start first (it provides BTSP).
Songbird depends on BearDog for authenticated relay. SkunkBat depends on
both for audit context. This is enforced via systemd `After=` and
`Requires=` directives.

**Use case**: Production membrane with identity and audit. The standard
deployment for `membrane.primals.eco`.

---

## Tier 4: Nest

Adds NestGate (sovereign storage) and the provenance trio (rhizoCrypt,
loamSpine, sweetGrass) on top of Tower. Nest enables Channel 3 Surface
with TLS content delivery.

| Attribute       | Value                                                    |
|-----------------|----------------------------------------------------------|
| Primals         | beardog, songbird, skunkbat, nestgate, rhizocrypt, loamspine, sweetgrass |
| Symbiotic       | hbbs, hbbr, caddy (TLS termination)                     |
| Channels active | Channel 2, Channel 2b, Channel 3 (Surface)              |
| Ports           | 22, 3478, 21115-21117, 80, 443, 9500, 9601, 9700, 9850 |
| Systemd units   | Tower units + `nestgate-membrane`, `rhizocrypt-membrane`, `loamspine-membrane`, `sweetgrass-membrane`, `caddy-tls` |
| Credentials     | Tower credentials + NestGate config, Caddy TLS certs    |
| Boot order      | Tower boot → NestGate → provenance trio → Caddy         |
| Health check    | Tower checks + NestGate :9500 + Caddy TLS probe         |
| Dark Forest     | Full + content provenance validation                     |

Nest adds significant surface area. The additional ports (9500, 9601,
9700, 9850) are for Nest-internal primal communication; they should be
firewall-restricted to known gate IPs in production.

NestGate runs in `cache-only` mode on the membrane (`NESTGATE_MODE=cache-only`).
It caches content synced from gates but does not originate content. The
membrane remains a broker, never a data plane.

**Use case**: Full sovereign content delivery. Replace GitHub Pages and
CDN with self-hosted membrane. The `membrane.primals.eco` deployment
at Phase 1.5.

---

## Composition Requirements Table

| Requirement                      | Relay | RustDesk | Tower | Nest |
|----------------------------------|-------|----------|-------|------|
| Songbird binary                  | Yes   | Yes      | Yes   | Yes  |
| RustDesk binaries (hbbs/hbbr)    | No    | Yes      | Yes   | Yes  |
| BearDog binary                   | No    | No       | Yes   | Yes  |
| SkunkBat binary                  | No    | No       | Yes   | Yes  |
| NestGate binary                  | No    | No       | No    | Yes  |
| Provenance trio (3 binaries)     | No    | No       | No    | Yes  |
| Caddy binary                     | No    | No       | No    | Yes  |
| `tower.env` identity file        | No    | No       | Yes   | Yes  |
| BTSP enforced                    | No    | No       | Yes   | Yes  |
| TURN credentials                 | Yes   | Yes      | Yes   | Yes  |
| TLS certificates                 | No    | No       | No    | Yes  |
| Dark Forest full compliance      | No    | No       | Yes   | Yes  |

---

## Extending the Model

To add a new composition tier:

1. **Define the tier** in this document with the full attribute table.
2. **Add to `MembraneComposition` enum** in `cellmembrane-types/composition.rs`.
3. **Define the `CompositionSpec`** — list required primals, ports, services.
4. **Update `FirewallRuleset`** derivation in `cellmembrane-types/firewall.rs`.
5. **Add systemd unit templates** to `infra/plasmidBin/membrane/`.
6. **Update `deploy_membrane.sh`** case statement (or future Rust CLI).
7. **Validate** with `cellmembrane-types` against the reference `membrane.toml`.

To add a new primal to an existing tier:

1. **Add to the tier's primal list** in this document and in `CompositionSpec`.
2. **Register the port** in both this document and `ports.env`.
3. **Create a systemd unit** in `membrane/`.
4. **Update firewall rules** if the primal needs an externally-reachable port.

---

## Alignment with biomeOS Atomics

| Membrane Composition | biomeOS Atomic   | Primals in Common         |
|----------------------|------------------|---------------------------|
| Tower                | Tower (electron) | beardog, songbird, skunkbat |
| Nest                 | Nest (neutron)   | + nestgate, rhizocrypt, loamspine, sweetgrass |

The membrane does **not** deploy the Node atomic (toadstool, barracuda,
coralreef). Compute stays on gates, never on the membrane. This is by
design — the membrane is a rendezvous broker, not a compute substrate.

The full NUCLEUS atomic (Tower + Node + Nest, 10 primals) runs only on
gates managed by projectNUCLEUS, never on a membrane fieldMouse.
