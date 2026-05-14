# cellMembrane

**Private operational repo for the cellMembrane fieldMouse deployment.**

| | |
|-|-|
| **Owner** | projectNUCLEUS / ironGate team |
| **Class** | fieldMouse — Tower atomic on external substrate |
| **VPS** | `membrane-relay`, 157.230.3.183, Debian 12 x64, DigitalOcean nyc1 |
| **Composition** | Tower (BearDog + Songbird + SkunkBat) |
| **Active Channel** | Channel 2: Relay (Songbird TURN on :3478) |
| **Status** | Hardened — exim4 purged, fail2ban active, firewall 22+3478 only |

---

## What This Repo Is For

This is the **operational home** for the cellMembrane deployment. Unlike the
public architecture docs in `wateringHole` and deployment tooling in
`plasmidBin`, this repo holds:

- Operational state and VPS-specific configuration
- Credential management procedures
- Team-internal runbooks and status
- Anything that references specific IPs, keys, or access patterns

**This repo is private.** Sensitive operational details belong here, not in
public repos.

---

## Quick Start

```bash
# Check cellMembrane status
cd ../plasmidBin
./deploy_membrane.sh status root@157.230.3.183

# SSH to VPS
ssh root@157.230.3.183

# View relay logs
ssh root@157.230.3.183 "journalctl -u songbird-relay -f"

# Deploy Tower composition (when ready)
./deploy_membrane.sh deploy root@157.230.3.183 --composition tower
```

---

## Hardening Status

| Check | Status |
|-------|--------|
| exim4 removed | DONE |
| fail2ban active (systemd backend) | DONE |
| Firewall: 22/tcp + 3478/udp+tcp only | DONE |
| SSH key-only auth | DONE |
| credentials.env redundant plaintext removed | DONE |
| journald persistence | DONE |
| TURN credentials at /etc/songbird/relay-credentials | DONE |

---

## Escalation Ladder

```
Phase 0: Relay only ← CURRENT
Phase 1: Tower composition (BearDog + Songbird + SkunkBat)
Phase 2: Encrypted-at-rest (BearDog Vault)
Phase 3: BingoCube zero-knowledge access control
Phase 4: Full autonomy (BearDog auto-rotation)
```

---

## Related Repos (public)

| Repo | What |
|------|------|
| [`plasmidBin`](https://github.com/ecoPrimals/plasmidBin) | Deployment tooling: `deploy_membrane.sh`, systemd units, `share_credentials.sh` |
| [`wateringHole`](https://github.com/ecoPrimals/wateringHole) | Architecture docs: `MEMBRANE_CHANNEL_ARCHITECTURE.md`, `CELLMEMBRANE_FIELDMOUSE_DEPLOYMENT.md` |
| [`primalSpring`](https://github.com/syntheticChemistry/primalSpring) | Coordination: capability registry, validation, Primal enum |

---

## License

AGPL-3.0-or-later
