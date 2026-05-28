# ironGate Verification Procedure

**Purpose**: Confirm ironGate has operational control of the cellMembrane.
**Last validated:** Phase 1.5 (Nest Atomic, Wave 38 — 2026-05-22)
**Updated:** 2026-05-27 (Wave 56 VPS deployment standard + deep debt sprint)

---

## Prerequisites

- SSH access to the cellMembrane VPS (ed25519 key, multi-gate managed)
- `doctl` CLI installed
- Access to encrypted credential blob (via `share_credentials.sh`)
- `plasmidBin` cloned locally at `../../infra/plasmidBin`

## Nest Atomic Verification (Current)

```bash
# 1. Decrypt credential blob
cd ../../infra/plasmidBin
./membrane/share_credentials.sh decrypt membrane-credentials.age

# 2. Authenticate with DigitalOcean
doctl auth init --access-token <decrypted-token>

# 3. Verify VPS exists and is healthy
doctl compute droplet list --tag-name membrane

# 4. Full status check via deploy script
./deploy_membrane.sh status root@$VPS_IP

# 5. Verify Nest Atomic composition services
ssh root@$VPS_IP "
  echo '=== Tower ==='
  systemctl is-active beardog-membrane songbird-relay skunkbat-membrane
  echo '=== Nest (Provenance Trio) ==='
  systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane
  echo '=== RustDesk ==='
  systemctl is-active hbbs-membrane hbbr-membrane
  echo '=== Channel 3 Surface ==='
  systemctl is-active caddy-tls
  echo '=== Channel 1 Signal (DNS) ==='
  systemctl is-active knot
  echo '=== Security ==='
  fail2ban-client status sshd
  ufw status | head -25
"

# 6. Verify Channel 3 TLS
curl -sI https://membrane.primals.eco/ | head -5
echo | openssl s_client -connect $VPS_IP:443 -servername membrane.primals.eco 2>/dev/null | openssl x509 -noout -dates

# 7. Verify Channel 1 Signal (DNS)
dig @$VPS_IP primals.eco A
dig @$VPS_IP primals.eco DNSKEY  # DNSSEC

# 8. Verify Nest data directories
ssh root@$VPS_IP "ls -la /var/lib/membrane/ && du -sh /var/cache/membrane/nestgate/"

# 9. Verify UDS sockets (Wave 56 VPS standard)
ssh root@$VPS_IP "ls -la /run/membrane/*.sock"

# 10. TTFB sovereignty check
curl -w "TTFB: %{time_starttransfer}s\n" -o /dev/null -s https://membrane.primals.eco/
```

## Success Criteria

All checks must pass:

- [ ] Credential blob decrypts successfully
- [ ] `doctl` authenticates and lists the membrane droplet
- [ ] `deploy_membrane.sh status` reports all 11+ services RUNNING
- [ ] Tower services active: beardog-membrane, songbird-relay, skunkbat-membrane
- [ ] Nest services active: nestgate-membrane, rhizocrypt-membrane, loamspine-membrane, sweetgrass-membrane
- [ ] RustDesk services active: hbbs-membrane, hbbr-membrane
- [ ] Caddy active, TLS certificate valid (Let's Encrypt E8)
- [ ] knot-dns active, DNSSEC responding
- [ ] `membrane.primals.eco` resolves and serves HTTPS
- [ ] sporePrint content cache present (~19 MB)
- [ ] UDS sockets present at `/run/membrane/*.sock` (Wave 56 VPS standard)
- [ ] TTFB ≤ 100ms (sovereignty parity with GitHub Pages)
- [ ] `fail2ban` protecting SSH
- [ ] UFW shows 16+ ALLOW rules (22, 53×2, 80, 443, 3478×2, 8443, 9500, 9602, 9700, 9850, 21115, 21116×2, 21117)
- [ ] Dark Forest audit: 21 PASS, 0 FAIL, 1 SKIP (MEM-09 b3sum)
- [ ] Provenance trio pipeline: 10/10 PASS
- [ ] Shadow orchestrator: 6/6 PASS

## Ownership After Verification

ironGate owns:
- VPS uptime and monitoring
- Credential rotation
- Channel deployment decisions (all 3 channels operational)
- Multi-gate SSH key management
- Caddy TLS certificate lifecycle
- knot-dns zone management + NS cutover coordination
- Forgejo Releases coordination (NC-3.4)
- Sovereign DNS primary cutover timing (NC-3.3)

projectNUCLEUS retains:
- Deployment tooling maintenance (`deploy_membrane.sh`)
- Upstream capability evolution (BearDog Vault, BingoCube)
- Architecture standards
- Gate-level validation (Dark Forest, membrane provenance)

primalSpring retains:
- Coordination standards
- Validation scenario definitions (`s_membrane_composition`, `s_kderm_boundary`, etc.)
- Primal capability registry
