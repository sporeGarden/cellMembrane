# VPS State Snapshot

**Last updated:** 2026-05-23
**Deployed composition:** Tower (Phase 1)

---

## Infrastructure

| Attribute | Value |
|-----------|-------|
| Provider | DigitalOcean |
| Region | nyc1 |
| Size | s-1vcpu-2gb ($12/mo) |
| OS | Debian 12 x64 |
| IP | 157.230.3.183 |
| Hostname | membrane-relay |
| SSH user | root |

---

## Running Services

| Service | Unit Name | Status | Port / Socket |
|---------|-----------|--------|---------------|
| BearDog | `beardog-membrane` | ACTIVE | `/run/membrane/beardog.sock` |
| Songbird TURN | `songbird-relay` | ACTIVE | :3478 tcp/udp |
| SkunkBat | `skunkbat-membrane` | ACTIVE | 127.0.0.1:9140 |
| RustDesk hbbs | `hbbs-membrane` | ACTIVE | :21115-21116 |
| RustDesk hbbr | `hbbr-membrane` | ACTIVE | :21117 |
| Caddy | `caddy` | ACTIVE | :80/:443 |
| fail2ban | `fail2ban` | ACTIVE | — |

**Boot order constraint (systemd):** BearDog → Songbird → SkunkBat

---

## Firewall (UFW)

| Rule | Reason |
|------|--------|
| 22/tcp ALLOW | SSH |
| 80/tcp ALLOW | Caddy HTTP (ACME redirect) |
| 443/tcp ALLOW | Caddy HTTPS (Channel 3 Surface) |
| 3478/tcp ALLOW | Songbird TURN (Channel 2 Relay) |
| 3478/udp ALLOW | Songbird TURN (Channel 2 Relay) |
| 21115/tcp ALLOW | RustDesk NAT type test |
| 21116/tcp ALLOW | RustDesk ID registration |
| 21116/udp ALLOW | RustDesk hole punching |
| 21117/tcp ALLOW | RustDesk relay |

**Default policy:** deny incoming, allow outgoing

---

## Filesystem Layout

| Path | Contents |
|------|----------|
| `/opt/membrane/beardog` | BearDog binary (stripped static ELF) |
| `/opt/membrane/songbird` | Songbird binary |
| `/opt/membrane/skunkbat` | SkunkBat binary |
| `/opt/membrane/hbbs` | RustDesk ID server binary |
| `/opt/membrane/hbbr` | RustDesk relay binary |
| `/opt/membrane/tower.env` | Tower environment (FAMILY_ID, BEARDOG_FAMILY_SEED, MEMBRANE_ROLE) |
| `/opt/membrane/credentials.env` | **REMOVED** (plaintext credentials purged) |
| `/opt/membrane/credentials.age` | Age-encrypted credential blob |
| `/opt/membrane/rustdesk/` | RustDesk keys and working directory |
| `/etc/songbird/relay-credentials` | TURN credentials (nucleus-relay:&lt;hex&gt;) |
| `/etc/membrane/` | Tower configuration |
| `/run/membrane/` | Unix domain sockets (BearDog, SkunkBat) |
| `/var/cache/membrane/nestgate/` | sporePrint content cache (19 MB synced from NestGate) |

---

## TLS / Certificate State

| Attribute | Value |
|-----------|-------|
| CA | Let's Encrypt |
| Certificate chain | E8 intermediate |
| Domain | `membrane.primals.eco` |
| Renewal | Caddy automatic (ACME) |
| TTFB (measured) | 68ms sovereign vs 89ms GitHub Pages |

---

## Bonding / Trust

| Type | Description |
|------|-------------|
| Covalent | SSH key access from gates → VPS |
| Ionic | BTSP tokens for external services |

---

## Validation Results

| Check | Result | Date |
|-------|--------|------|
| Dark Forest audit | 17 PASS, 0 FAIL | 2026-05 |
| Trio pipeline | 10/10 PASS | 2026-05 |
| `deploy_membrane.sh status` | All services RUNNING | 2026-05 |

---

## Pending Deployments (Not Yet on VPS)

| Component | Port | Composition | Blocker |
|-----------|------|-------------|---------|
| knot-dns (Channel 1 Signal) | :53 | — | Sovereign DNS config, UFW port open |
| NestGate | :9500 | nest | Nest expansion deploy |
| rhizoCrypt | :9601 | nest | Nest expansion deploy |
| loamSpine | :9700 | nest | Nest expansion deploy |
| sweetGrass | :9850 | nest | Nest expansion deploy |

Nest expansion creates data dirs at `/var/lib/membrane/nestgate` and `/var/lib/membrane/loamspine`.

---

## Update Procedure

After any VPS change, update this file:

```bash
# Verify current state
ssh root@157.230.3.183 "systemctl list-units 'beardog*' 'songbird*' 'skunkbat*' 'hbbs*' 'hbbr*' 'caddy*' 'nestgate*' 'rhizocrypt*' 'loamspine*' 'sweetgrass*' --no-pager"
ssh root@157.230.3.183 "ufw status numbered"

# Then edit VPS_STATE.md with changes
```
