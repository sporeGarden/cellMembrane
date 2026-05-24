# ironGate Verification Procedure

**Purpose**: Confirm ironGate has operational control of the cellMembrane.
**Last validated:** Phase 1 (Tower composition + Channel 3 Surface)

---

## Prerequisites

- SSH access to the cellMembrane VPS (ed25519 key, multi-gate managed)
- `doctl` CLI installed
- Access to encrypted credential blob (via `share_credentials.sh`)
- `plasmidBin` cloned locally at `../../infra/plasmidBin`

## Phase 1 Verification (Current)

```bash
# 1. Decrypt credential blob
cd ../../infra/plasmidBin
./membrane/share_credentials.sh decrypt membrane-credentials.age

# 2. Authenticate with DigitalOcean
doctl auth init --access-token <decrypted-token>

# 3. Verify VPS exists and is healthy
doctl compute droplet list --tag-name membrane

# 4. Full status check via deploy script
./deploy_membrane.sh status root@157.230.3.183

# 5. Verify Tower composition services
ssh root@157.230.3.183 "
  echo '=== Tower ==='
  systemctl is-active beardog-membrane songbird-relay skunkbat-membrane
  echo '=== RustDesk ==='
  systemctl is-active hbbs-membrane hbbr-membrane
  echo '=== Channel 3 Surface ==='
  systemctl is-active caddy
  echo '=== Security ==='
  fail2ban-client status sshd
  ufw status | head -15
"

# 6. Verify Channel 3 TLS
curl -sI https://membrane.primals.eco/ | head -5
echo | openssl s_client -connect 157.230.3.183:443 -servername membrane.primals.eco 2>/dev/null | openssl x509 -noout -dates

# 7. Verify content cache
ssh root@157.230.3.183 "du -sh /var/cache/membrane/nestgate/"

# 8. TTFB sovereignty check
curl -w "TTFB: %{time_starttransfer}s\n" -o /dev/null -s https://membrane.primals.eco/
```

## Success Criteria

All checks must pass:

- [ ] Credential blob decrypts successfully
- [ ] `doctl` authenticates and lists the membrane droplet
- [ ] `deploy_membrane.sh status` reports all services RUNNING
- [ ] Tower services active: beardog-membrane, songbird-relay, skunkbat-membrane
- [ ] RustDesk services active: hbbs-membrane, hbbr-membrane
- [ ] Caddy active, TLS certificate valid (Let's Encrypt E8)
- [ ] `membrane.primals.eco` resolves and serves HTTPS
- [ ] sporePrint content cache present (~19 MB)
- [ ] TTFB ≤ 100ms (sovereignty parity with GitHub Pages)
- [ ] `fail2ban` protecting SSH
- [ ] UFW shows 9 ALLOW rules (22, 80, 443, 3478×2, 21115, 21116×2, 21117)
- [ ] Dark Forest audit: 17 PASS, 0 FAIL
- [ ] Trio pipeline: 10/10 PASS

## Phase 1.5 Verification (After Nest Expansion)

Additional checks for when `--composition nest` is deployed:

```bash
# Nest services
ssh root@157.230.3.183 "systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane"

# Nest data directories
ssh root@157.230.3.183 "ls -la /var/lib/membrane/"

# Nest ports
ssh root@157.230.3.183 "ufw status | grep -E '9500|9601|9700|9850'"
```

Additional success criteria:
- [ ] 4 nest services active
- [ ] Data dirs at `/var/lib/membrane/nestgate` and `/var/lib/membrane/loamspine`
- [ ] UFW includes nest ports (9500, 9601, 9700, 9850)

## After Verification

ironGate now owns:
- VPS uptime and monitoring
- Credential rotation
- Channel deployment decisions
- Multi-gate SSH key management
- Caddy TLS certificate lifecycle
- Nest expansion deployment timing
- Sovereign DNS (Channel 1) deployment

projectNUCLEUS retains:
- Deployment tooling maintenance (`deploy_membrane.sh`)
- Upstream capability evolution (BearDog Vault, BingoCube)
- Architecture standards
- Gate-level validation (Dark Forest, membrane provenance)

primalSpring retains:
- Coordination standards
- Validation scenario definitions
- Primal capability registry
