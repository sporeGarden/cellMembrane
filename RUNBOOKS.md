# Operational Runbooks

**Audience:** cellMembrane operators (ironGate team)
**VPS_IP:** Set `VPS_IP` from `nucleus_config.sh` → `MEMBRANE_VPS_IP`, or `deploy_membrane.sh` resolves it.
All `$VPS_IP` references below are the membrane relay host.

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
./deploy_membrane.sh status root@$VPS_IP

# Quick manual check
ssh root@$VPS_IP "
  echo '=== Services ==='
  echo '=== Tower ==='
  systemctl is-active beardog-membrane songbird-relay skunkbat-membrane
  echo '=== Nest ==='
  systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane
  echo '=== RustDesk ==='
  systemctl is-active hbbs-membrane hbbr-membrane
  echo '=== Surface + DNS ==='
  systemctl is-active caddy knot
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

**Expected:** All 11+ services active, 16+ UFW ALLOW rules, disk < 80%.

---

## 2. Channel 2 Relay — Songbird TURN

### View logs
```bash
ssh root@$VPS_IP "journalctl -u songbird-relay -f"
```

### Restart
```bash
ssh root@$VPS_IP "systemctl restart songbird-relay"
```

### Verify TURN credentials
```bash
ssh root@$VPS_IP "cat /etc/songbird/relay-credentials"
```

Format: `nucleus-relay:<hex-key>`

### Test connectivity
```bash
# From a gate machine
stun $VPS_IP:3478
```

---

## 3. Channel 2b — RustDesk

### View logs
```bash
ssh root@$VPS_IP "journalctl -u hbbs-membrane -u hbbr-membrane -f"
```

### Restart
```bash
ssh root@$VPS_IP "systemctl restart hbbs-membrane hbbr-membrane"
```

### Get public key (for client config)
```bash
ssh root@$VPS_IP "cat /opt/membrane/rustdesk/id_ed25519.pub"
```

### Client settings
| Setting | Value |
|---------|-------|
| ID Server | $VPS_IP |
| Relay Server | $VPS_IP |
| Key | (output of above command) |

---

## 4. Channel 3 Surface — Caddy TLS

### View logs
```bash
ssh root@$VPS_IP "journalctl -u caddy -f"
```

### Check certificate status
```bash
# From local machine
echo | openssl s_client -connect $VPS_IP:443 -servername membrane.primals.eco 2>/dev/null | openssl x509 -noout -dates -issuer
```

### Restart Caddy
```bash
ssh root@$VPS_IP "systemctl restart caddy"
```

### Verify content cache
```bash
ssh root@$VPS_IP "du -sh /var/cache/membrane/nestgate/"
```

Expected: ~19 MB sporePrint content.

### Force certificate renewal
```bash
ssh root@$VPS_IP "caddy reload --config /etc/caddy/Caddyfile"
```

Caddy auto-renews via ACME. Manual renewal should rarely be needed.

### TTFB measurement
```bash
curl -w "TTFB: %{time_starttransfer}s\n" -o /dev/null -s https://membrane.primals.eco/
```

Sovereignty threshold: must be ≤ 1.5× GitHub Pages TTFB (~89ms).

---

## 5. Channel 1 Signal — Sovereign DNS (knot-dns)

> **Status: DEPLOYED** (Wave 38, 2026-05-22). knot-dns running with DNSSEC. NS cutover to primary pending (registrar action).

### Current state

knot-dns v3.2.6 is installed and running on the VPS with DNSSEC zone signing enabled.
UFW ports 53/tcp and 53/udp are open. Zone file configured for `primals.eco`.

### Remaining: NS cutover to primary

1. Validate current resolution: `dig @$VPS_IP primals.eco A`
2. Update registrar NS records to include $VPS_IP as primary
3. Monitor for 7+ days before removing commercial DNS fallback

### Validation
```bash
dig @$VPS_IP primals.eco A
dig @$VPS_IP membrane.primals.eco A
dig @$VPS_IP primals.eco NS
```

### Primary cutover
1. Validate secondary serving correct records for 7+ days
2. Update registrar NS records to include $VPS_IP
3. Remove commercial DNS after TTL expiry + monitoring period

---

## 6. Nest Atomic Operations (Current Composition)

> **Status: LIVE** (Wave 38, 2026-05-22). Nest Atomic deployed with provenance trio.

### Health check
```bash
ssh root@$VPS_IP "
  systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane
  echo '=== Nest Ports ==='
  ss -tlnp | grep -E '9500|9602|9700|9850'
  echo '=== Data Dirs ==='
  ls -la /var/lib/membrane/
"
```

### Service details

| Service | Port | Health check |
|---------|------|-------------|
| nestgate-membrane | :9500 | `curl -s http://$VPS_IP:9500/health` |
| rhizocrypt-membrane | :9602 (JSON-RPC) | `curl -s -X POST http://127.0.0.1:9602` |
| loamspine-membrane | :9700 | `curl -s http://127.0.0.1:9700/health` |
| sweetgrass-membrane | :9850 | TCP connection probe |

### Redeployment (upgrade or recovery)
```bash
cd ../../infra/plasmidBin
./deploy_membrane.sh deploy root@$VPS_IP --composition nest --validate
```

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
./membrane/share_credentials.sh push root@$VPS_IP
```

Deploys to `/opt/membrane/credentials.age` on VPS.

### Pull and decrypt from VPS
```bash
./membrane/share_credentials.sh pull root@$VPS_IP
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
./deploy_membrane.sh keys list root@$VPS_IP
```

### Add a gate's key
```bash
./deploy_membrane.sh keys add root@$VPS_IP \
  --name "eastGate" \
  --pubkey "ssh-ed25519 AAAA..."
```

Keys are tagged in `authorized_keys` with `# gate:<name> added:<date>`.

### Revoke a gate's key
```bash
./deploy_membrane.sh keys revoke root@$VPS_IP --name "eastGate"
```

### Audit keys manually
```bash
ssh root@$VPS_IP "cat /root/.ssh/authorized_keys"
```

---

## 9. Emergency Procedures

### Service down — single service restart
```bash
ssh root@$VPS_IP "systemctl restart <unit-name>"
```

### Full Nest Atomic restart (preserves boot order)
```bash
ssh root@$VPS_IP "
  systemctl restart beardog-membrane && sleep 2
  systemctl restart songbird-relay && sleep 2
  systemctl restart skunkbat-membrane && sleep 2
  systemctl restart nestgate-membrane && sleep 2
  systemctl restart rhizocrypt-membrane && sleep 2
  systemctl restart loamspine-membrane && sleep 2
  systemctl restart sweetgrass-membrane
"
```

Boot order: BearDog → Songbird → SkunkBat → NestGate → rhizoCrypt → loamSpine → sweetGrass.

### VPS unreachable — reprovisioning
```bash
cd ../../infra/plasmidBin

# Check DO status
doctl compute droplet list --tag-name membrane

# If droplet exists but unresponsive, access via DO console
# If droplet destroyed, reprovision:
./deploy_membrane.sh provision --region nyc1

# Redeploy current composition (Nest Atomic)
./deploy_membrane.sh deploy root@<new-ip> --composition nest --validate

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
# Service status (org-level runner)
sudo systemctl status actions.runner.ecoPrimals.irongate-runner.service

# GitHub-side status (org-level)
gh api orgs/ecoPrimals/actions/runners --jq '.runners[] | "\(.name) \(.status) [\(.labels | map(.name) | join(","))]"'
```

### Restart runner
```bash
sudo systemctl restart actions.runner.ecoPrimals.irongate-runner.service
```

### Fall back to GitHub-hosted runners
Set the org variable `USE_GITHUB_HOSTED=true` in GitHub org settings to route
all workflows back to `ubuntu-latest`. Remove the variable to return to self-hosted.

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
- Service: `actions.runner.ecoPrimals.irongate-runner.service`
- Labels: `self-hosted, Linux, X64, x86_64, irongate, lan`
- Scope: **org-level** (serves all ecoPrimals repos)

### Workflow sovereignty level
| Workflow | Self-hosted ready | Raw git checkout | Marketplace-free |
|----------|-------------------|------------------|------------------|
| validate.yml | Yes | Yes | **Yes** |
| auto-harvest.yml | Yes | No | No |
| smoke.yml | Yes | No | No |
| check-updates.yml | Yes | No | No |
| tier23-harvest.yml | Yes | No | No |

Only `validate.yml` can run during a full GitHub codeload outage. Other workflows
still need `actions/checkout@v4` and `actions/upload-artifact@v4` downloaded from
GitHub. Evolving these to raw git + direct upload is future work.
