# Operational Runbooks

**Audience:** cellMembrane operators (sporeGate team)
**Last updated:** 2026-06-23 (Wave 126)
**VPS_IP:** Set `VPS_IP` from `nucleus_config.sh` → `MEMBRANE_VPS_IP`.

> **Note (Wave 126):** The `membrane` Rust CLI has fully replaced `deploy_membrane.sh`
> for all operational flows. Use: `membrane gate.status` (health), `membrane gate.bootstrap`
> (enrollment), `membrane plasmid.refresh` (binary push), `membrane temporal.cascade`
> (sync), `membrane rootpulse.status` (sovereignty). Wave 120+:
> `membrane topology.service <role>` (identity-based discovery), `membrane wireguard.generate`
> (declarative WG config), `membrane caddy.generate` (Caddyfile from manifest),
> `membrane gate.validate` (composition trust barrier — `pepti.validate` deprecated Wave 120).
> Wave 121+: `membrane topology.endpoint <gate> <cap>` (transport resolver),
> `membrane gate.quorum` (autonomous cascade timer). Legacy `deploy_membrane.sh`
> references below are retained as fossil record only — do not use operationally.

---

## Table of Contents

1. [Daily Health Check](#1-daily-health-check)
2. [Channel 2 Relay — Songbird TURN](#2-channel-2-relay--songbird-turn)
3. [Channel 2b — RustDesk](#3-channel-2b--rustdesk)
4. [Channel 3 Surface — Caddy TLS](#4-channel-3-surface--caddy-tls)
5. [Channel 1 Signal — Sovereign DNS (knot-dns)](#5-channel-1-signal--sovereign-dns-knot-dns)
6. [VPS Deployment Standard (Wave 56)](#6-vps-deployment-standard-wave-56)
7. [Nest Atomic Operations](#7-nest-atomic-operations-current-composition)
8. [Credential Management](#8-credential-management)
9. [SSH Key Management (Multi-Gate)](#9-ssh-key-management-multi-gate)
10. [Emergency Procedures](#10-emergency-procedures)
11. [Self-Hosted GitHub Actions Runner](#11-self-hosted-github-actions-runner)
12. [Sandbox Validation + Canary Pool](#12-sandbox-validation--canary-pool-wave-110)
13. [Mesh Join — Gate as Plasmodium Gate](#13-mesh-join--gate-as-plasmodium-gate)

---

## 1. Daily Health Check

```bash
# Full status via membrane CLI
membrane gate.health
membrane gate.status

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
  systemctl is-active caddy-tls knot
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
ssh root@$VPS_IP "journalctl -u caddy-tls -f"
```

### Check certificate status
```bash
# From local machine
echo | openssl s_client -connect $VPS_IP:443 -servername membrane.primals.eco 2>/dev/null | openssl x509 -noout -dates -issuer
```

### Restart Caddy
```bash
ssh root@$VPS_IP "systemctl restart caddy-tls"
```

### Verify content cache
```bash
ssh root@$VPS_IP "du -sh /var/cache/membrane/nestgate/ /var/cache/membrane/lab/"
```

Expected: ~19 MB sporePrint content, lab content varies.

### Deploy updated Caddyfile
```bash
# SSOT: infra/plasmidBin/membrane/Caddyfile → /etc/membrane/Caddyfile on VPS
scp plasmidBin/membrane/Caddyfile root@$VPS_IP:/etc/membrane/Caddyfile
ssh root@$VPS_IP "/opt/membrane/caddy validate --config /etc/membrane/Caddyfile && systemctl reload caddy-tls"
```

### Sync lab static content to VPS
```bash
rsync -avz --delete /path/to/lab/export/ root@$VPS_IP:/var/cache/membrane/lab/
ssh root@$VPS_IP "systemctl reload caddy-tls"
```

### Force certificate renewal
```bash
ssh root@$VPS_IP "/opt/membrane/caddy reload --config /etc/membrane/Caddyfile"
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

## 6. VPS Deployment Standard (Wave 56)

> **Standard:** primalSpring Wave 56 — three-step VPS deployment with UDS-only NUCLEUS.

### Full NUCLEUS deployment (UDS-only)

```bash
cd ../../infra/plasmidBin

# Step 1: Deploy NUCLEUS base (13 primals, UDS-only)
./deploy_membrane.sh deploy root@$VPS_IP --composition nucleus --uds-only --validate

# Step 2: Deploy spring overlay (e.g. hotspring)
./deploy_membrane.sh spring-overlay root@$VPS_IP --cell hotspring

# Step 3: Spring runtime discovers NUCLEUS via UDS (automatic)
# Spring uses CompositionContext::from_live_discovery() — no manual config needed
```

### Verify NUCLEUS launcher

```bash
ssh root@$VPS_IP "systemctl status nucleus-launcher"
ssh root@$VPS_IP "ls -la /run/membrane/*.sock"
```

### VPS-ready springs

Only springs with `vps_standard = true` in the cell manifest can be deployed:
- hotspring, wetspring, neuralspring, airspring, groundspring, healthspring

### What NOT to use on VPS

- `desktop_nucleus.sh` — desktop-only
- `cell_launcher.sh` — desktop-only
- TCP port flags for NUCLEUS primals — UDS-only is the standard
- Harness-based spawning — use `biomeos deploy` instead

---

## 7. Nest Atomic Operations (Current Composition)

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

## 8. Credential Management

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

## 9. SSH Key Management (Multi-Gate)

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

## 10. Emergency Procedures

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

## 11. Self-Hosted GitHub Actions Runner

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

---

## 13. Mesh Join — Gate as Plasmodium Gate

### Prerequisites

- All 13 NUCLEUS primal UDS sockets active under `/run/user/1000/biomeos/`
- BearDog running with TCP endpoint (`:9900` or UDS)
- Songbird federation port configured (`:7700`)
- Remote mesh partner operational (eastGate, strandGate)

### Environment Variables

```bash
# ironGate mesh configuration
export SONGBIRD_NODE_ID=iron-gate
export SONGBIRD_PEERS=east-gate@192.168.1.144:7700,strand-gate@192.168.1.132:7700
export SONGBIRD_FEDERATION_PORT=7700
export SONGBIRD_FEDERATION_ENABLED=true
export SECURITY_ENDPOINT=http://127.0.0.1:9900
export FAMILY_ID=irongate
```

### Capability Symlinks

Songbird and biomeOS discover capabilities via socket symlinks:

```bash
SOCKET_DIR=/run/user/1000/biomeos

# BearDog provides security capabilities
ln -sf $SOCKET_DIR/beardog-irongate.sock $SOCKET_DIR/security.sock
ln -sf $SOCKET_DIR/beardog-irongate.sock $SOCKET_DIR/btsp.sock
ln -sf $SOCKET_DIR/beardog-irongate.sock $SOCKET_DIR/crypto.sock

# Songbird provides mesh capabilities
ln -sf $SOCKET_DIR/songbird.sock $SOCKET_DIR/discovery.sock
ln -sf $SOCKET_DIR/songbird.sock $SOCKET_DIR/orchestration.sock
```

### Startup Sequence

1. Start BearDog (needs `FAMILY_ID`, `NODE_ID`)
2. Create security symlinks
3. Start Songbird (needs `SECURITY_ENDPOINT`, `SONGBIRD_FEDERATION_PORT`, `SONGBIRD_PEERS`)
4. Create discovery/orchestration symlinks
5. Mesh auto-bootstraps from `SONGBIRD_PEERS` on startup (Wave 73 fix: `spawn_mesh_seed()`)

```bash
# Songbird auto-bootstraps peers on boot via SONGBIRD_PEERS env var.
# Manual mesh.init is only needed if adding peers AFTER startup:
curl -s -X POST http://127.0.0.1:7700/jsonrpc -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0",
  "method": "mesh.init",
  "params": {
    "node_id": "iron-gate",
    "bootstrap_peers": ["east-gate@192.168.1.144:7700", "strand-gate@192.168.1.132:7700"]
  },
  "id": 1
}'
```

### Verification

```bash
# Check peer discovery
curl -s http://127.0.0.1:7700/jsonrpc -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0", "method": "discovery.peers", "params": {}, "id": 1
}' | jq '.result.total_count'

# Check mesh health
curl -s http://127.0.0.1:7700/jsonrpc -H "Content-Type: application/json" -d '{
  "jsonrpc": "2.0", "method": "mesh.health_check", "params": {}, "id": 1
}' | jq '.result.all_healthy'
```

### Known Limitations (Wave 74)

- Cross-subnet southGate (192.168.4.x) unreachable from 192.168.1.x — needs
  Eero inter-VLAN routing or TURN relay via cellMembrane VPS
- ironGate joins as 3rd plasmodium gate on same subnet (192.168.1.238)

### Resolved (Wave 73 Songbird fix, commit d6a6f714)

- `capability.call` cross-gate — now uses HTTP POST to `/jsonrpc` (was raw TCP)
- `SONGBIRD_PEERS` auto-bootstraps on startup (was requiring manual `mesh.init`)
- `mesh.init` accepts string format `"node@host:port"` (was object-only)
- `latency_ms` populated via periodic health probes (~2 min interval)

---

## 12. Sandbox Validation + Canary Pool (Wave 110+)

### Pre-deployment sandbox validation

```bash
# Validate a staged binary in isolation before promoting to production
membrane plasmid.sandbox --primal beardog

# Validate AND atomically promote if healthy
membrane plasmid.sandbox --primal beardog --promote

# List active sandbox instances
membrane plasmid.sandbox.list
```

### Canary pool management

```bash
# List canary pool (previous-good binaries)
membrane plasmid.canary.list

# Health-check all canaries
membrane plasmid.canary.health

# Rollback: promote canary back to production
membrane plasmid.canary.promote --primal beardog

# List healthy failover targets (for mesh routing)
membrane plasmid.canary.failover

# Tear down all canary instances (cleanup)
membrane plasmid.canary.teardown
```

### How cascade-restart uses sandbox + canary

When `temporal.cascade --with-restart` detects a new binary:
1. **Sandbox**: Spawns new binary in isolated UDS socket, probes JSON-RPC health
2. **Canary retire**: If sandbox passes, copies old production binary to canary dir
3. **Promote**: Overwrites production binary with new (atomic `fs::copy`)
4. **Restart**: `systemctl restart membrane-nucleus@{primal}`
5. **Fail-open**: If sandbox infra fails (directory missing, etc.), proceeds anyway

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `MEMBRANE_SANDBOX_SOCKET_DIR` | `/run/membrane/sandbox` | Sandbox UDS directory |
| `MEMBRANE_SANDBOX_BIN_DIR` | `/opt/membrane/sandbox` | Sandbox binary staging |
| `MEMBRANE_CANARY_SOCKET_DIR` | `/run/membrane/canary` | Canary UDS directory |
| `MEMBRANE_CANARY_BIN_DIR` | `/opt/membrane/canary` | Canary binary storage |
