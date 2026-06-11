# ironGate Verification Procedure

**Purpose**: Confirm ironGate has operational control of the cellMembrane.
**Last validated:** Wave 110 (deep debt evolution, native UDS probes, gate expansion, 2026-06-11)
**Composition:** Full NUCLEUS — 13 primals, sovereign TLS, UDS-only posture, WAN depot + dual checksums, 6-gate mesh

---

## Prerequisites

- SSH access to the cellMembrane VPS (ed25519 key, multi-gate managed)
- `membrane` CLI binary built (`cargo build --release -p membrane-shadow`)
- Access to encrypted credential blob (via `share_credentials.sh`)
- `plasmidBin` accessible at `../../infra/plasmidBin`

## NUCLEUS Verification (Current)

```bash
# 1. Full gate health (Rust-native, replaces deploy_membrane.sh status)
membrane gate.health

# 2. Verify all 13 primals + infrastructure services
ssh root@$VPS_IP "
  echo '=== Tower (identity + relay + federation + audit) ==='
  systemctl is-active beardog-membrane songbird-membrane songbird-relay skunkbat-membrane beardog-tls-shadow
  echo '=== Compute (node tier) ==='
  systemctl is-active toadstool-membrane barracuda-membrane coralreef-membrane
  echo '=== Nest (provenance quartet) ==='
  systemctl is-active nestgate-membrane rhizocrypt-membrane loamspine-membrane sweetgrass-membrane
  echo '=== Meta (intelligence) ==='
  systemctl is-active biomeos-membrane squirrel-membrane petaltongue-membrane
  echo '=== Infrastructure ==='
  systemctl is-active caddy hbbs-membrane hbbr-membrane knot
  echo '=== Security ==='
  fail2ban-client status sshd
  ufw status | head -30
"

# 3. Verify UDS sockets (NUCLEUS standard)
ssh root@$VPS_IP "ls -la /run/membrane/*.sock"

# 4. Verify 5-domain sovereign TLS
for domain in primals.eco mesh.primal.eco auth.primal.eco api.primal.eco nestgate.io; do
  echo "--- $domain ---"
  curl -sI "https://$domain/" | head -3
done

# 5. Verify Channel 1 Signal (DNS)
dig @$VPS_IP primals.eco A
dig @$VPS_IP primals.eco DNSKEY

# 6. Binary freshness check
membrane plasmid.refresh --dry-run

# 7. Cascade sync (VPS workspace)
membrane temporal.cascade --dry-run
```

## Success Criteria

All checks must pass:

- [ ] `membrane gate.health` reports all services ACTIVE
- [ ] 13/13 primal systemd units active
- [ ] Tower: beardog, songbird (UDS+federation:7700), skunkbat (localhost:9140)
- [ ] Compute: toadstool, barracuda, coralreef (all UDS)
- [ ] Nest: nestgate, rhizocrypt, loamspine, sweetgrass (all UDS)
- [ ] Meta: biomeos, squirrel, petaltongue (all UDS)
- [ ] RustDesk: hbbs + hbbr active
- [ ] Caddy active, 5 domains serving HTTPS (sovereign TLS via Let's Encrypt)
- [ ] knot-dns active, DNSSEC responding
- [ ] UDS sockets present at `/run/membrane/*.sock`
- [ ] `fail2ban` protecting SSH
- [ ] UFW: zero externally-exposed primal TCP ports
- [ ] Federation mesh port :7700 operational
- [ ] `socat` bridges operational for UDS→private-network proxying

## Ownership After Verification

ironGate owns:
- VPS uptime and monitoring
- Credential rotation
- Channel deployment decisions (all 3 channels operational)
- Multi-gate SSH key management
- Caddy TLS certificate lifecycle + reverse proxy wiring
- knot-dns zone management + NS cutover coordination
- plasmidBin — binary harvesting, checksums, CI, refresh cycles
- VPS deployment ops — systemd, bridges, firewall
- Peptidoglycan self-refresh evolution

projectNUCLEUS retains:
- Architecture standards
- Gate-level validation (Dark Forest, membrane provenance)
- Deploy graph definitions

primalSpring retains:
- Composition experimentation and bonding models
- Validation scenario definitions
- Primal capability registry patterns
