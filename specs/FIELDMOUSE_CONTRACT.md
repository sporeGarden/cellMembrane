# fieldMouse Deployment Contract

**Version**: 1.0.0
**Date**: May 26, 2026
**Status**: Active
**Authority**: cellMembrane team
**License**: AGPL-3.0-or-later
**Related**: `CELLMEMBRANE_ARCHITECTURE.md`, `infra/wateringHole/CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md`

---

## Abstract

A fieldMouse is a membrane deployment on external substrate — infrastructure
you do not physically control (VPS, cloud VM, bare metal in a colo). This
contract defines what a fieldMouse deployment must satisfy to be considered
operational within the ecoPrimals ecosystem, and what a third party must
implement to run their own.

A fieldMouse is:
- **Minimal** — Tower atomic on external substrate, not a full NUCLEUS gate
- **Ephemeral** — can be torn down and reprovisioned without data loss
- **Sovereign** — all secrets encrypted at rest, provider treated as adversary
- **A broker** — routes traffic, never originates or stores primary data

---

## Deployment Classes

| Class         | Substrate         | biomeOS | Composition    | Example              |
|---------------|-------------------|---------|----------------|----------------------|
| **fieldMouse**| External VPS/cloud| No      | Tower or Nest  | `membrane.primals.eco` |
| **gate**      | Owned hardware    | Yes     | Full NUCLEUS   | ironGate, eastGate   |
| **niche**     | Gate-local        | Yes     | Domain spring  | wetSpring on southGate |

This contract applies to fieldMouse deployments only. Gates and niches
have their own deployment standards (see `DESKTOP_NUCLEUS_DEPLOYMENT.md`
and `SPRING_NICHE_DEPLOYMENT_GUIDE.md`).

---

## Hardware Requirements

### Minimum (Relay composition)

| Resource | Minimum     | Notes                          |
|----------|-------------|--------------------------------|
| CPU      | 1 vCPU      | Songbird is lightweight        |
| RAM      | 512 MB      | Static binary, no runtime      |
| Disk     | 10 GB       | OS + binaries                  |
| Network  | Public IPv4 | Required for TURN              |
| OS       | Debian 12+  | Or any systemd-based Linux     |

### Recommended (Nest composition)

| Resource | Recommended | Notes                           |
|----------|-------------|---------------------------------|
| CPU      | 2 vCPU      | NestGate content serving        |
| RAM      | 2 GB        | Caddy + NestGate cache          |
| Disk     | 25 GB       | Content cache + logs            |
| Network  | Public IPv4 | + domain with DNS control       |
| OS       | Debian 12+  | Stable, long-term support       |

---

## Hardening Checklist

Every fieldMouse must satisfy all items before being considered operational.

### SSH

- [ ] Password authentication disabled (`PasswordAuthentication no`)
- [ ] Root login restricted (`PermitRootLogin prohibit-password`)
- [ ] SSH keys are the only authentication method
- [ ] SSH on port 22 only (no obscurity)

### Firewall

- [ ] UFW enabled with `default deny incoming`
- [ ] Only composition-required ports open (see `MEMBRANE_COMPOSITION_MODEL.md`)
- [ ] No manual port additions outside the composition
- [ ] `default allow outgoing` (primals need to reach GitHub Releases, DNS)

### System Hardening

- [ ] Mail transfer agent removed (exim4, postfix — not needed)
- [ ] Provider agents removed (droplet-agent, cloud-init post-setup)
- [ ] fail2ban installed and active
- [ ] journald persistence enabled (`Storage=persistent`)
- [ ] Automatic security updates enabled (`unattended-upgrades`)

### Credential Security

- [ ] No plaintext credentials in `/opt/membrane/` (except `tower.env` which is 0600)
- [ ] `tower.env` permissions: `chmod 0600`, owned by root
- [ ] TURN credentials in `/etc/songbird/relay-credentials` (0600, root)
- [ ] Cross-gate credential sharing via `age` encryption only
- [ ] No credentials committed to git (enforced by `.gitignore`)

### Binary Security

- [ ] All primal binaries are static musl ELFs (no dynamic linking)
- [ ] Binaries are stripped (`strip --strip-all`)
- [ ] No hostnames, paths, or debug info embedded
- [ ] Binaries fetched from GitHub Releases with BLAKE3 checksum verification

---

## Operational Contract

### The fieldMouse MUST:

1. **Respond to `deploy_membrane.sh status`** with accurate channel health.
2. **Maintain TURN relay availability** — 99.9% uptime target for Channel 2.
3. **Preserve `tower.env` across redeploys** — the family seed is persistent identity.
4. **Keep binaries current** — track plasmidBin releases, redeploy on security updates.
5. **Pass Dark Forest audit** — all 17 MEM checks from `darkforest_membrane.sh`.

### The fieldMouse MUST NOT:

1. **Originate content** — it caches, proxies, and relays only.
2. **Store user data** — NestGate runs in `cache-only` mode.
3. **Run biomeOS** — no orchestration kernel on external substrate.
4. **Expose intracellular ports** — BearDog, SkunkBat, provenance trio are loopback/UDS only.
5. **Trust the substrate** — provider is a non-family observer per Dark Forest model.

### The fieldMouse MAY:

1. **Be torn down and reprovisioned** from `membrane.toml` + `tower.env` backup.
2. **Run on any provider** — DigitalOcean, Hetzner, Linode, bare metal, LAN.
3. **Use a custom domain** — configure via `membrane.toml` `domain` field.
4. **Scale vertically** — upgrade VPS size without redeployment.
5. **Serve multiple families** — future multi-tenant mode via family ID isolation.

---

## Dark Forest Compliance

Five pillars from `DARK_FOREST_GLACIAL_GATE_STANDARD.md` applied to fieldMouse:

| Pillar                        | fieldMouse Implementation                      |
|-------------------------------|------------------------------------------------|
| 1. Zero metadata leakage      | Stripped binaries, no hostnames embedded        |
| 2. Zero port exposure          | Composition-aware UFW, deny default             |
| 3. Songbird sole network surface | All external relay traffic through Songbird   |
| 4. BTSP crypto integrity       | `BTSP_MODE=enforced` in Tower+ compositions    |
| 5. Enclave computing           | UDS-first IPC, loopback for TCP primals        |

Pillar 3 is nuanced on a membrane: Caddy (Channel 3) and knot-dns
(Channel 1) are exceptions — they are purpose-built for public exposure.
The pillar applies to all other traffic.

---

## Validation

A fieldMouse deployment is validated at three levels:

### Level 1: Structural (pre-deploy)

Validate `membrane.toml` against `cellmembrane-types`:
- Composition is valid
- Required primals listed
- Ports consistent with composition
- Provider config present

### Level 2: Runtime (post-deploy)

Run `deploy_membrane.sh status root@<host>`:
- All systemd units active
- UFW rules match composition
- Health probes pass (TURN allocate, TCP connect, TLS handshake)

### Level 3: Security (audit)

Run `darkforest_membrane.sh`:
- 17 MEM checks (MEM-01 through MEM-17)
- SSH hardening, credential perms, port inventory, binary provenance
- Must be 17/17 PASS for production status

---

## Lifecycle

```
provision ──► harden ──► deploy ──► validate ──► operate
                                                    │
                                                    ├── status (health)
                                                    ├── redeploy (binary update)
                                                    ├── keys (SSH management)
                                                    └── teardown (destroy)
```

Provisioning creates the substrate (VPS). Hardening locks down SSH and
firewall. Deployment installs binaries and starts services. Validation
confirms operational status. The operate phase is indefinite, with
periodic health checks and binary updates.

Teardown is non-destructive to the ecosystem — the membrane can be
reprovisioned from `membrane.toml`. The only state that must be backed
up is `tower.env` (family seed) and any TLS certificates not managed
by ACME auto-renewal.

---

## For Third-Party Operators

To deploy your own membrane:

1. **Write a `membrane.toml`** — see `MULTI_MEMBRANE_DEPLOYMENT.md` for the schema.
2. **Provision a VPS** — any provider with public IPv4 and Debian 12+.
3. **Run the deployer** — `deploy_membrane.sh deploy root@<ip> --composition <tier>`.
4. **Validate** — `deploy_membrane.sh status root@<ip> --validate`.
5. **Register your domain** — point DNS to the VPS IP, configure in `membrane.toml`.
6. **Back up `tower.env`** — this is your membrane's persistent identity.

No ecoPrimals account, API key, or coordination is required. The membrane
is self-contained. Binaries are fetched from public GitHub Releases.
