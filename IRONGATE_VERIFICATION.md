# ironGate Verification Procedure

**Purpose**: Confirm ironGate has operational control of the cellMembrane.
This is the first secure validation milestone.

---

## Prerequisites

- SSH access to the cellMembrane VPS (same `ecoPrimal` ed25519 key)
- `doctl` CLI installed
- Access to encrypted credential blob (via `share_credentials.sh`)
- `plasmidBin` cloned locally

## Steps

```bash
# 1. Decrypt credential blob (requires the ecoPrimal SSH private key)
cd ../plasmidBin
./membrane/share_credentials.sh decrypt membrane-credentials.age

# 2. Authenticate with DigitalOcean
doctl auth init --access-token <decrypted-token>

# 3. Verify VPS exists and is healthy
doctl compute droplet list --tag-name membrane

# 4. Check cellMembrane status via deploy script
./deploy_membrane.sh status root@157.230.3.183

# 5. Verify services directly
ssh root@157.230.3.183 "systemctl is-active songbird-relay && \
    fail2ban-client status sshd && \
    ufw status | head -10"
```

## Success Criteria

All five checks must pass:

- [ ] Credential blob decrypts successfully
- [ ] `doctl` authenticates and lists the membrane droplet
- [ ] `deploy_membrane.sh status` reports relay RUNNING
- [ ] `songbird-relay` is active on the VPS
- [ ] `fail2ban` is protecting SSH, firewall shows only 22+3478

## After Verification

ironGate now owns:
- VPS uptime and monitoring
- Credential rotation
- Channel deployment decisions
- Scaling (Model A → Model B if needed)

primalSpring retains:
- Deployment tooling maintenance
- Upstream capability evolution (BearDog Vault, BingoCube)
- Architecture standards
