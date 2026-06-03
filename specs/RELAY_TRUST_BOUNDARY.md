# Relay Trust Boundary — Cross-Gate Security Audit

**Version**: 1.0.0
**Date**: 2026-06-03 (Wave 75)
**Status**: Active
**Authority**: cellMembrane team (ironGate)
**License**: AGPL-3.0-or-later
**Related**: `K_DERM_TOPOLOGY.md`, `CELLMEMBRANE_ARCHITECTURE.md`
**Reference**: gen5 `COVALENT_MESH_TRUST_VALIDATION.md`

---

## Purpose

Document the security boundary of the cellMembrane VPS relay infrastructure.
As the relay operator, ironGate must define precisely what data transits the
VPS, what is visible to the relay, and what guarantees hold for gate-to-gate
communication.

---

## Relay Architecture

The cellMembrane VPS operates three distinct relay functions:

```
┌─────────────────────────────────────────────────────────┐
│  cellMembrane VPS (golgiBody)                           │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │ Channel 2: Songbird TURN (:3478)                 │   │
│  │   UDP relay — encrypted media streams            │   │
│  │   Visibility: OPAQUE (DTLS-SRTP encrypted)       │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │ Channel 2b: Songbird Federation (:7700)          │   │
│  │   TCP mesh — JSON-RPC over HTTP POST             │   │
│  │   Visibility: STRUCTURED (method + params)       │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │ K-Derm Relay: git sync chain                     │   │
│  │   SSH — git objects (commits, trees, blobs)      │   │
│  │   Visibility: FULL (relay has repo clones)       │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

---

## Trust Analysis Per Channel

### Channel 2: Songbird TURN Relay (:3478 UDP)

| Property | Value |
|----------|-------|
| Protocol | TURN (RFC 5766) over UDP |
| Encryption | DTLS-SRTP (end-to-end between peers) |
| Relay visibility | **OPAQUE** — encrypted bytestream only |
| Authentication | TURN credentials (`/etc/songbird/relay-credentials`) |
| What relay sees | Source/dest IP:port pairs, packet sizes, timing |
| What relay CANNOT see | Payload content (encrypted), application semantics |

**Assessment**: The TURN relay is a pure network-layer relay. All media is
DTLS-SRTP encrypted end-to-end between gates. The VPS cannot inspect, modify,
or replay the relayed data. Even if the VPS is compromised, the attacker gains
only traffic metadata (who talks to whom, when, how much).

**BTSP token visibility**: TURN authentication uses a separate credential
(`nucleus-relay:<hex>`) unrelated to BTSP session tokens. BTSP tokens are
**never transmitted through the TURN relay** — they travel over the mesh
federation channel or direct UDS connections.

### Channel 2b: Songbird Federation (:7700 TCP)

| Property | Value |
|----------|-------|
| Protocol | HTTP POST with JSON-RPC 2.0 |
| Encryption | **None** (plaintext TCP on current deployment) |
| Relay visibility | **STRUCTURED** — full JSON-RPC method and params visible |
| Authentication | Peer identity via `node_id` in mesh registration |
| What relay sees | Method names, capability calls, peer registrations |
| What relay CANNOT see | N/A — all federation traffic is plaintext |

**Assessment**: The federation channel carries structured capability routing
data. On the VPS, Songbird is both a **participant** (with its own registered
primals) and a **hub** (mediating cross-gate mesh traffic). All JSON-RPC
payloads are visible to the VPS Songbird instance.

**BTSP token visibility**: BTSP tokens may appear in `capability.call` params
if a remote capability requires authentication. Currently, cross-gate
capability calls do not embed BTSP tokens in the federation payload — auth is
handled at the destination gate's BearDog instance. **BTSP tokens are opaque
to the federation relay** because they are resolved locally at the target gate.

**Mitigation for future**: When Songbird adds TLS for federation (planned),
the VPS will still be a participant. True end-to-end opacity requires
BearDog-signed envelopes (capability call payloads encrypted to the target
gate's public key). This is a gen5 design target.

### K-Derm Git Relay (SSH)

| Property | Value |
|----------|-------|
| Protocol | SSH + git protocol |
| Encryption | SSH transport (encrypted in transit) |
| Relay visibility | **FULL** — relay has complete repo clones |
| Authentication | SSH ed25519 keys (metallic bond) |
| What relay sees | All source code, all commit history, all branches |
| What relay CANNOT see | Encrypted credential blobs (`.age` files) |

**Assessment**: The K-Derm relay chain has **full visibility** of repository
content. This is by design — the relay is the sovereign git hosting layer
(Forgejo). The trust model is:

1. golgiBody-inner (cis-Golgi) is the **primary** — full authority
2. peptidoglycan (structural) is a **clone** — has all data
3. golgiBody-ext (trans-Golgi) is the **exit** — pushes to GitHub

All three nodes hold complete repository clones. Credentials are protected
by `age` encryption — `.age` files are present but cannot be decrypted without
the operator's private key.

**BTSP token visibility**: Forgejo API tokens are stored on the VPS
(`FORGEJO_TOKEN` env var, `~/.config/forgejo/token`). These are **VPS-scoped
Forgejo tokens** (not BTSP tokens). BTSP tokens are never stored in git or
transmitted through the relay chain.

---

## BTSP Token Opacity Summary

| Channel | BTSP Token Visible? | Reason |
|---------|-------------------|--------|
| TURN (:3478) | **NO** | Media-only relay, DTLS encrypted |
| Federation (:7700) | **NO** | Auth resolved at target gate, not embedded in RPC |
| K-Derm git relay | **NO** | Tokens not stored in repos, `.age` encrypted |
| BearDog TLS (:8443) | **LOCAL ONLY** | Token issued and consumed on same VPS |

**Conclusion**: BTSP tokens are opaque to the relay in all cross-gate paths.

---

## Attack Surface Assessment

### If VPS is compromised

| Attack | Impact | Mitigation |
|--------|--------|------------|
| Read git repos | Source code exposed | Repos are open-source (GitHub mirror exists) |
| Forge commits | Unsigned commits possible | Provenance trio (sweetGrass/loamSpine) detects via `provenance.toml` |
| Traffic analysis | Who-talks-to-whom metadata | Songbird peer rotation (future) |
| Intercept federation | Read capability routing | BearDog envelope encryption (gen5 target) |
| Steal TURN credentials | Unauthorized relay use | Credential rotation, per-session TURN tokens (Songbird roadmap) |
| Steal Forgejo token | Repo admin on VPS Forgejo | Token scoped to VPS instance only, not upstream |

### What the relay CANNOT do

1. **Decrypt BTSP sessions** — no access to family seeds or BearDog private keys
2. **Forge BearDog signatures** — Ed25519 keys held only on gates
3. **Modify encrypted content** — `.age` blobs cannot be altered without detection
4. **Impersonate a gate** — mesh peer identity is cryptographically bound
5. **Access local UDS sockets on gates** — only accessible on localhost

---

## Design Principles (for gen5 paper)

1. **Relay sees routing, not content** — The relay knows *where* to send data
   but not *what* the data means (TURN) or *who authorized it* (BTSP is local).

2. **Trust is additive, not transitive** — A gate trusts its local BearDog for
   auth. The relay is trusted for availability (routing) but not for
   confidentiality or integrity.

3. **Sovereign hosting is a tradeoff** — The K-Derm git relay has full content
   visibility because it IS the sovereign alternative to GitHub. The defense is
   that the operator owns both the relay and the content.

4. **Future hardening** — Federation TLS + BearDog envelope encryption would
   achieve full content opacity even on the federation channel. This is not
   critical today because all gates are covalent (same operator).

---

## Recommendations

1. **No action needed for covalent mesh** — All current gates share the same
   operator. Full trust is appropriate.

2. **Before ionic mesh (friend gates)** — Implement:
   - Federation TLS (Songbird)
   - Capability call envelope encryption (BearDog → target gate pubkey)
   - Per-session TURN credentials (Songbird)

3. **Before weak mesh (public gates)** — Implement:
   - Zero-knowledge relay (BingoCube)
   - Hardware attestation (SoloKey)
   - Onion routing for federation (multi-hop)
