# Operational Runbooks

**Audience:** cellMembrane operators (ironGate team)
**VPS:** 157.230.3.183 (root)

---

## Table of Contents

1. [Daily Health Check](#1-daily-health-check)
2. [Channel 2 Relay — Songbird TURN](#2-channel-2-relay--songbird-turn)
3. [Channel 2b — RustDesk](#3-channel-2b--rustdesk)
4. [Channel 3 Surface — Caddy TLS](#4-channel-3-surface--caddy-tls)
5. [Channel 1 Signal — Sovereign DNS (knot-dns)](#5-channel-1-signal--sovereign-dns-knot-dns)
6. [Nest Expansion Deployment](#6-nest-expansion-deployment)
7. [Credential Management](#7-credential-management)
8. [SSH Key Management (Multi-Gate)](#8-ssh-key-management-multi-gate)
9. [Emergency Procedures](#9-emergency-procedures)

---

## 1. Daily Health Check

```bash
# Full status via deploy script
cd ../../infra/plasmidBin
./deploy_membrane.sh status root@157.230.3.183

# Quick manual check
ssh root@157.230.3.183 "
  echo '=== Services ==='
  systemctl is-active beardog-membrane songbird-relay skunkbat-membrane hbbs-membrane hbbr-membrane caddy
  echo '=== Firewall ==='
  ufw status | grep -c ALLOW
  echo '=== Disk ==='
  df -h /
  echo '=== Memory ==='
  free -h
  echo '=== Uptime ==='
  uptime
"
```

**Expected:** All 6 services active, 9 UFW ALLOW rules, disk < 80%.

---

## 2. Channel 2 Relay — Songbird TURN

### View logs
```bash
ssh root@157.230.3.183 "journalctl -u songbird-relay -f"
```

### Restart
```bash
ssh root@157.230.3.183 "systemctl restart songbird-relay"
```

### Verify TURN credentials
```bash
ssh root@157.230.3.183 "cat /etc/songbird/relay-credentials"
```

Format: `nucleus-relay:<hex-key>`

### Test connectivity
```bash
# From a gate machine
stun 157.230.3.183:3478
```

---

## 3. Channel 2b — RustDesk

### View logs
```bash
ssh root@157.230.3.183 "journalctl -u hbbs-membrane -u hbbr-membrane -f"
```

### Restart
```bash
ssh root@157.230.3.183 "systemctl restart hbbs-membrane hbbr-membrane"
```

### Get public key (for client config)
```bash
ssh root@157.230.3.183 "cat /opt/membrane/rustdesk/id_ed25519.pub"
```

### Client settings
| Setting | Value |
|---------|-------|
| ID Server | 157.230.3.183 |
| Relay Server | 157.230.3.183 |
| Key | (output of above command) |

---

## 4. Channel 3 Surface — Caddy TLS

### View logs
```bash
ssh root@157.230.3.183 "journalctl -u caddy -f"
```

### Check certificate status
```bash
# From local machine
echo | openssl s_client -connect 157.230.3.183:443 -servername membrane.primals.eco 2>/dev/null | openssl x509 -noout -dates -issuer
```

### Restart Caddy
```bash
ssh root@157.230.3.183 "systemctl restart caddy"
```

### Verify content cache
```bash
ssh root@157.230.3.183 "du -sh /var/cache/membrane/nestgate/"
```

Expected: ~19 MB sporePrint content.

### Force certificate renewal
```bash
ssh root@157.230.3.183 "caddy reload --config /etc/caddy/Caddyfile"
```

Caddy auto-renews via ACME. Manual renewal should rarely be needed.

### TTFB measurement
```bash
curl -w "TTFB: %{time_starttransfer}s\n" -o /dev/null -s https://membrane.primals.eco/
```

Sovereignty threshold: must be ≤ 1.5× GitHub Pages TTFB (~89ms).

---

## 5. Channel 1 Signal — Sovereign DNS (knot-dns)

> **Status: NOT DEPLOYED.** This is a glacial shift blocker. Procedures below are for when deployment begins.

### Pre-deployment

1. Install knot-dns on VPS:
```bash
ssh root@157.230.3.183 "apt update && apt install -y knot knot-dnsutils"
```

2. Open UFW ports:
```bash
ssh root@157.230.3.183 "ufw allow 53/tcp comment 'Channel 1 DNS' && ufw allow 53/udp comment 'Channel 1 DNS'"
```

3. Configure zone file for `primals.eco` at `/etc/knot/knot.conf`

4. Start as secondary first (point to current commercial DNS as primary)

### Validation
```bash
dig @157.230.3.183 primals.eco A
dig @157.230.3.183 membrane.primals.eco A
dig @157.230.3.183 primals.eco NS
```

### Primary cutover
1. Validate secondary serving correct records for 7+ days
2. Update registrar NS records to include 157.230.3.183
3. Remove commercial DNS after TTL expiry + monitoring period

---

## 6. Nest Expansion Deployment

### Pre-flight
```bash
# Confirm Tower is healthy
cd ../../infra/plasmidBin
./deploy_membrane.sh status root@157.230.3.183

# Verify VPS resources
ssh root@157.230.3.183 "free -h && df -h / && nproc"
```

### Deploy
```bash
./deploy_membrane.sh deploy root@157.230.3.183 --composition nest --validate
```

This deploys Tower + RustDesk + Nest atomically. Adds:
- nestgate-membrane (:9500)
- rhizocrypt-membrane (:9601)
- loamspine-membrane (:9700)
- sweetgrass-membrane (:9850)

### Post-deploy validation
```bash
ssh root@157.230.3.183 "
  systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane
  ufw status | grep -E '950[0]|960[1]|970[0]|985[0]'
  ls -la /var/lib/membrane/
"
```

### Known issue
`deploy_membrane.sh --validate` UFW verification does not check nest ports (9500-9850). Nest ports may trigger "unexpected UFW rules" warnings — this is a false positive.

---

## 7. Credential Management

### Encrypt credentials for sharing
```bash
cd ../../infra/plasmidBin
./membrane/share_credentials.sh encrypt
```

Creates `membrane-credentials.age` encrypted with SSH ed25519 keys.

### Decrypt credentials
```bash
./membrane/share_credentials.sh decrypt membrane-credentials.age
```

### Push encrypted blob to VPS
```bash
./membrane/share_credentials.sh push root@157.230.3.183
```

Deploys to `/opt/membrane/credentials.age` on VPS.

### Pull and decrypt from VPS
```bash
./membrane/share_credentials.sh pull root@157.230.3.183
```

### Credential contents
| Key | Description |
|-----|-------------|
| DOCTL_TOKEN | DigitalOcean API token |
| SONGBIRD_TURN_KEY | TURN relay HMAC key |
| SONGBIRD_TURN_USERNAME | TURN relay username |
| MEMBRANE_VPS_IP | VPS IP (optional) |

### Evolution path
- **Current:** age encryption via SSH ed25519 keys
- **Phase 2:** BearDog BTSP secrets management
- **Phase 4:** Autonomous Tower rotation

---

## 8. SSH Key Management (Multi-Gate)

### List authorized keys
```bash
cd ../../infra/plasmidBin
./deploy_membrane.sh keys list root@157.230.3.183
```

### Add a gate's key
```bash
./deploy_membrane.sh keys add root@157.230.3.183 \
  --name "eastGate" \
  --pubkey "ssh-ed25519 AAAA..."
```

Keys are tagged in `authorized_keys` with `# gate:<name> added:<date>`.

### Revoke a gate's key
```bash
./deploy_membrane.sh keys revoke root@157.230.3.183 --name "eastGate"
```

### Audit keys manually
```bash
ssh root@157.230.3.183 "cat /root/.ssh/authorized_keys"
```

---

## 9. Emergency Procedures

### Service down — single service restart
```bash
ssh root@157.230.3.183 "systemctl restart <unit-name>"
```

### Full Tower restart (preserves boot order)
```bash
ssh root@157.230.3.183 "
  systemctl restart beardog-membrane
  sleep 2
  systemctl restart songbird-relay
  sleep 2
  systemctl restart skunkbat-membrane
"
```

Boot order is critical: BearDog → Songbird → SkunkBat.

### VPS unreachable — reprovisioning
```bash
cd ../../infra/plasmidBin

# Check DO status
doctl compute droplet list --tag-name membrane

# If droplet exists but unresponsive, access via DO console
# If droplet destroyed, reprovision:
./deploy_membrane.sh provision --region nyc1

# Redeploy current composition
./deploy_membrane.sh deploy root@<new-ip> --composition tower --validate

# Push credentials
./membrane/share_credentials.sh push root@<new-ip>

# Re-add gate SSH keys
./deploy_membrane.sh keys add root@<new-ip> --name "<gate>" --pubkey "..."
```

### Teardown (destructive)
```bash
./deploy_membrane.sh teardown --name membrane-relay
```

Requires typing `destroy` to confirm. Destroys the DO droplet.

### Firewall lockout recovery
Access VPS via DigitalOcean console (web UI), then:
```bash
ufw disable
ufw reset
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw enable
```

Then redeploy firewall rules via `deploy_membrane.sh`.

---

## 10. Self-Hosted GitHub Actions Runner

### Status check
```bash
# Service status
sudo systemctl status actions.runner.ecoPrimals-plasmidBin.irongate-runner.service

# GitHub-side status
gh api repos/ecoPrimals/plasmidBin/actions/runners --jq '.runners[] | "\(.name) \(.status)"'
```

### Restart runner
```bash
cd ~/actions-runner
sudo ./svc.sh stop
sudo ./svc.sh start
```

### Local validation (GitHub-independent)
```bash
cd ~/Development/ecoPrimals/infra/plasmidBin
cargo run -p plasmidbin -- validate .
```

### Cross-compilation targets
```bash
# x86_64 static binary
cargo build -p plasmidbin --release --target x86_64-unknown-linux-musl

# aarch64 cross-compile
cargo build -p plasmidbin --release --target aarch64-unknown-linux-musl
```

### Runner agent location
- Agent: `~/actions-runner/`
- Work dir: `~/actions-runner/_work/`
- Service: `actions.runner.ecoPrimals-plasmidBin.irongate-runner.service`
- Labels: `self-hosted, Linux, X64, x86_64, irongate`
