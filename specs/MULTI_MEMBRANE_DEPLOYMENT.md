# Multi-Membrane Deployment

**Version**: 1.0.0
**Date**: May 26, 2026
**Status**: Active
**Authority**: cellMembrane team
**License**: AGPL-3.0-or-later
**Related**: `CELLMEMBRANE_ARCHITECTURE.md`, `FIELDMOUSE_CONTRACT.md`

---

## Abstract

A single ecoPrimals ecosystem may operate multiple membranes — external
VPS fieldMice in different regions, gate-local membrane layers on the LAN,
and third-party operated membranes for separate families. This document
defines the parameterization model that makes membranes portable across
providers, regions, and operators.

---

## Deployment Topology

```
                    ┌─── Internet ───┐
                    │                │
        ┌───────────┴───┐    ┌──────┴────────┐
        │ membrane-nyc  │    │ membrane-eu   │
        │ DO nyc1       │    │ Hetzner fsn1  │
        │ Tower+Nest    │    │ Relay only    │
        └───────┬───────┘    └──────┬────────┘
                │                   │
                └─────── WAN ───────┘
                         │
            ┌────────────┴────────────┐
            │    LAN Mesh (TURN)      │
            │                         │
     ┌──────┴──────┐          ┌───────┴─────┐
     │ ironGate    │          │ eastGate    │
     │ gate-local  │          │ gate-local  │
     │ membrane    │          │ membrane    │
     └─────────────┘          └─────────────┘
```

Each membrane is independently configured via its own `membrane.toml`.
Membranes discover each other through Songbird relay (TURN), not through
direct networking. A membrane does not need to know about other membranes
at deploy time.

---

## The `membrane.toml` Schema

This is the complete typed configuration for a membrane deployment.
Parsed and validated by `cellmembrane-types`.

```toml
[membrane]
name = "membrane-nyc"
domain = "membrane.primals.eco"
composition = "nest"

[membrane.identity]
family_id = "membrane-alpha"
# gate_id is auto-generated if not specified
gate_id = "nyc-01"

[membrane.provider]
type = "digitalocean"
region = "nyc1"
size = "s-1vcpu-2gb"
# provider-specific fields are allowed via serde(flatten)
image = "debian-12-x64"
tags = ["membrane", "production"]

[membrane.channels.signal]
enabled = true
primal = "knot-dns"
port = 53
dnssec = true

[membrane.channels.relay]
enabled = true
port = 3478
primal = "songbird"

[membrane.channels.surface]
enabled = true
port = 443
primal = "caddy"
tls_domain = "membrane.primals.eco"
acme_email = "ops@primals.eco"

[membrane.credentials]
model = "age"
# Future: "btsp_vault", "manual"
age_recipients = [
    "ssh-ed25519 AAAA... irongate",
    "ssh-ed25519 AAAA... eastgate",
]

[membrane.hardening]
fail2ban = true
unattended_upgrades = true
remove_mail_agent = true
remove_provider_agent = true
```

### Schema Rules

- `membrane.name` — unique identifier for this membrane instance.
- `membrane.composition` — one of: `relay`, `rustdesk`, `tower`, `nest`.
- `membrane.identity.family_id` — shared across all membranes in a family.
- `membrane.provider.type` — determines provider-specific deploy logic.
- `membrane.channels.*` — override defaults per channel. Channels not
  listed inherit from the composition's defaults.
- All fields have sensible defaults. A minimal `membrane.toml` needs only
  `name` and `composition`.

---

## Provider Abstraction

The deploy system supports multiple infrastructure providers through a
typed abstraction. Each provider has its own provisioning logic but shares
the same deploy, status, and teardown interface.

### Supported Providers

| Provider       | Type string       | Provisioning           | Status      |
|----------------|-------------------|------------------------|-------------|
| DigitalOcean   | `digitalocean`    | `doctl` API            | Implemented |
| Hetzner        | `hetzner`         | `hcloud` API           | Planned     |
| Bare metal     | `bare_metal`      | Manual (SSH only)      | Planned     |
| LAN gate       | `gate_local`      | Local systemd          | Planned     |
| Custom         | `custom`          | User-provided script   | Planned     |

### Provider Config Fields

Each provider type accepts different fields under `[membrane.provider]`:

**DigitalOcean:**
```toml
[membrane.provider]
type = "digitalocean"
region = "nyc1"          # DO region slug
size = "s-1vcpu-2gb"     # DO size slug
image = "debian-12-x64"  # DO image slug
tags = ["membrane"]      # DO tags
```

**Hetzner:**
```toml
[membrane.provider]
type = "hetzner"
location = "fsn1"
server_type = "cx22"
image = "debian-12"
```

**Bare metal:**
```toml
[membrane.provider]
type = "bare_metal"
host = "203.0.113.50"
ssh_user = "root"
ssh_port = 22
```

**Gate-local:**
```toml
[membrane.provider]
type = "gate_local"
# No remote host — deploys to localhost
```

---

## Substrate Profiles

Different deployment contexts have different constraints:

| Profile              | Substrate     | biomeOS | Channels       | Use case               |
|----------------------|---------------|---------|----------------|------------------------|
| VPS fieldMouse       | External VPS  | No      | 2, 2b, 3       | Production membrane    |
| Remote covalent      | Remote VPS    | No      | 2              | WAN relay for flockGate|
| Gate-local membrane  | Owned hardware| Yes     | 2 (local only) | LAN relay              |
| Dev fieldMouse       | Local VM      | No      | 2              | Testing                |

The substrate profile affects:
- Which hardening steps are applied (VPS gets full hardening; gate-local
  trusts the physical network)
- Whether biomeOS integration is available
- Which channels make sense (gate-local doesn't need Channel 3 Surface)
- Credential management (VPS uses age; gate-local uses BTSP vault directly)

---

## Multi-Membrane Coordination

Membranes do not coordinate directly. They are independently deployed
and independently operated. Coordination happens through:

1. **Shared family ID** — membranes in the same family use the same
   `FAMILY_ID` in their `tower.env`. BTSP sessions between gates and
   any membrane in the family are authenticated.

2. **Songbird relay** — gates discover membranes through TURN relay
   addresses. The relay address is configured on the gate side, not on
   the membrane.

3. **DNS** — the membrane's domain (if any) is registered with a public
   registrar. Gates resolve it through standard DNS.

There is no membrane-to-membrane communication protocol. If two membranes
need to exchange data, they do so through a gate (which connects to both
via Songbird).

---

## Multi-Region Topology

For resilience, deploy membranes in different geographic regions:

```toml
# membrane-nyc.toml
[membrane]
name = "membrane-nyc"
composition = "nest"
domain = "membrane.primals.eco"

[membrane.provider]
type = "digitalocean"
region = "nyc1"

# membrane-eu.toml
[membrane]
name = "membrane-eu"
composition = "relay"

[membrane.provider]
type = "hetzner"
location = "fsn1"
```

DNS can round-robin or failover between regions. Each membrane operates
independently — if one goes down, the other continues serving.

---

## Third-Party Deployment

Anyone can deploy their own membrane without ecoPrimals coordination:

### Quick Start

1. Install `deploy_membrane.sh` (from `infra/plasmidBin/`).
2. Write a `membrane.toml`:
   ```toml
   [membrane]
   name = "my-membrane"
   composition = "tower"

   [membrane.provider]
   type = "bare_metal"
   host = "my-server.example.com"
   ssh_user = "root"
   ```
3. Provision and deploy:
   ```bash
   ./deploy_membrane.sh deploy root@my-server.example.com \
       --composition tower --validate
   ```
4. Back up `tower.env` from the server.

### What You Get

- A hardened VPS with Songbird TURN relay
- BearDog BTSP identity boundary (Tower+)
- RustDesk remote access (Tower+)
- NestGate content delivery + Caddy TLS (Nest)
- All binaries are public plasmidBin releases — no private repos needed

### What You Manage

- Your VPS provider account and billing
- Your domain DNS records (if using Channel 3)
- Your `tower.env` backup (family identity)
- Binary updates (re-run deploy to update)

---

## Evolution Path

| Phase     | External Membrane        | Inner Membrane          | Coordination     |
|-----------|--------------------------|-------------------------|------------------|
| Current   | VPS fieldMouse (GitHub)  | Forgejo trailing mirror | GitHub dispatches |
| Next      | VPS + self-hosted CI     | Forgejo + Actions       | Dual dispatch    |
| Covalent  | Forgejo primary          | Forgejo leads           | Sovereign        |

The target state: membranes are provisioned from Forgejo releases,
validated by Forgejo CI, and coordinated through sovereign DNS. GitHub
becomes a public mirror, not the source of truth.
