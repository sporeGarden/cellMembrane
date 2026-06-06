# Experiment 001: NUCLEUS Full Deploy to VPS — Validated

**Date:** May 28, 2026 (Wave 59)
**VPS:** membrane-relay, $VPS_IP, DigitalOcean nyc1
**Composition:** NUCLEUS (13 primals + RustDesk + Caddy + knot-dns = 17 services)
**Predecessor:** Nest Atomic deploy (Wave 38, 7 primals)

---

## Summary

First successful end-to-end NUCLEUS deployment on the sovereign VPS. All 13
primals active and responding on UDS sockets. Spring overlay (hotSpring)
graph validation passed (14 nodes parsed). biomeOS Neural API healthy.

---

## Process

### Step 1: Type System Evolution (code)

Added `MembraneComposition::Nucleus` tier to `cellmembrane-types`:
- 6 new `MembraneService` entries (toadStool, barraCuda, coralReef, biomeOS, squirrel, petalTongue)
- All UDS-only, zero new firewall ports
- 175 tests, zero clippy warnings (pedantic + nursery)
- `membrane.toml` → `composition = "nucleus"`

### Step 2: Deploy (operational)

```bash
./deploy_membrane.sh deploy root@$VPS_IP --composition nucleus
```

Deploy executed tier-by-tier (Tower → RustDesk → Node → Nest → Meta):
- Tower: beardog, songbird, skunkbat — **already present** (idempotent)
- Node: toadstool, barracuda, coralreef — **freshly fetched** from plasmidBin releases
- Nest: nestgate, rhizocrypt, loamspine, sweetgrass — **already present**
- Meta: biomeos, squirrel, petaltongue — **freshly fetched** (biomeos v0.1.0, squirrel v0.1.0)

Total deploy time: ~57 seconds (including SSH, file transfer, systemd reload).

### Step 3: Health Verification

| Primal | Socket | `health.liveness` |
|--------|--------|-------------------|
| beardog | `/run/membrane/beardog.sock` | alive (v0.9.0) |
| songbird | `:3478` TCP | active |
| skunkbat | systemd active | active |
| toadstool | `/tmp/biomeos/compute-tarpc.sock` | active (tarpc convention) |
| barracuda | `/run/membrane/barracuda.sock` | alive |
| coralreef | `/run/membrane/coralreef.sock` | alive |
| nestgate | `/run/membrane/nestgate.sock` + `:9500` | active |
| rhizocrypt | `/run/membrane/rhizocrypt.sock` + `:9601` | active |
| loamspine | `/run/membrane/loamspine.sock` + `:9700` | active |
| sweetgrass | `/run/membrane/sweetgrass.sock` + `:9850` | active |
| biomeos | `/run/membrane/biomeos.sock` | healthy (v0.1.0) |
| squirrel | `/run/membrane/squirrel.sock` | alive (v0.1.0) |
| petaltongue | `/run/membrane/petaltongue.sock` | active |

Capability symlinks created by primals at runtime:
- `/run/membrane/btsp.sock` → beardog.sock
- `/run/membrane/crypto.sock` → beardog.sock
- `/run/membrane/ai.sock` → squirrel.sock
- `/run/membrane/visualization.sock` → petaltongue.sock

### Step 4: Spring Overlay Test

```bash
./deploy_membrane.sh spring-overlay root@$VPS_IP --cell hotspring
```

Result:
- Cell graph pushed to VPS: `/opt/membrane/cells/hotspring_cell.toml`
- biomeOS parsed 14 nodes successfully
- biomeOS validated graph structure
- `graph.execute` not yet implemented in v0.1.0 (expected — tracked upstream)

---

## Observations

### What Worked

1. **Tier-by-tier deploy is idempotent** — existing services not disrupted
2. **biomeOS responds to JSON-RPC** over UDS (`health.liveness` → healthy)
3. **Capability symlinks** auto-created by primals at startup (beardog creates btsp/crypto/ed25519/x25519, squirrel creates ai, petaltongue creates visualization)
4. **UDS socket ecosystem** fully operational — 13 sockets in `/run/membrane/`
5. **Zero new firewall ports** needed — all new services are UDS-only

### What Needs Evolution

1. **toadStool socket path**: Binds to `/tmp/biomeos/compute-tarpc.sock` instead of `/run/membrane/toadstool.sock` (env debt — `--socket` flag not consumed)
2. **biomeOS `graph.execute`**: Not implemented in v0.1.0 — the deploy CLI validates but can't orchestrate node execution over UDS yet
3. **FAMILY_ID not set**: `tower.env` lacks `FAMILY_ID` — biomeOS runs in "standalone" mode
4. **`nucleus_launcher` not in releases**: The `--uds-only` deploy path (preferred) requires this binary from upstream
5. **coralReef `--version` output**: Emits ERROR to stderr on version check (cosmetic, service is healthy)

---

## Validation Still Needed

### Security (Dark Forest)

- [ ] Full `darkforest_membrane.sh` re-run with 13 primals (was 7)
- [ ] Verify toadStool/barraCuda/coralReef bind to loopback or UDS only
- [ ] Verify no new TCP ports exposed (UFW unchanged)
- [ ] Verify capability symlinks don't create unintended access paths
- [ ] biomeOS socket permissions (currently `srw-------` root:root — correct)

### Sovereignty

- [ ] All 13 primals built from pure Rust (no C deps) — verify via `ldd`
- [ ] BLAKE3 checksums match plasmidBin `checksums.toml` for new binaries
- [ ] No phoning-home from toadStool, barraCuda, coralReef, biomeOS, squirrel
- [ ] Provenance trio pipeline re-run (was 10/10 with 7 primals)
- [ ] Shadow orchestrator re-run (was 6/6 with Nest Atomic)

### postPrimordial Patterns

- [ ] biomeOS `graph.execute` over UDS (gated on biomeOS v0.2)
- [ ] `CompositionContext::from_live_discovery()` validates against live UDS sockets
- [ ] Spring binary (`hotspring_primal`) available in plasmidBin
- [ ] Column U: hotSpring passes full emission cycle through biomeOS
- [ ] lithoSpore postPrimordial emission (gated on 2 column U passes)

### Operational

- [ ] `deploy_membrane.sh status` shows all 13 primals without errors
- [ ] `toadstool --socket /run/membrane/toadstool.sock` consumed (upstream fix)
- [ ] `nucleus_launcher` binary available for `--uds-only` deploy path
- [ ] Forgejo releases configured (sovereign binary channel)
- [ ] 7-day uptime monitoring for new services

---

## Resource Consumption

```
VPS: s-1vcpu-2gb ($12/mo)
Load: 0.00, 0.00, 0.00 (negligible with 17 services)
Memory: well within 2GB (all primals are static musl ELFs, minimal footprint)
Disk: /opt/membrane/* — all binaries stripped static
```

---

## Conclusion

NUCLEUS deployment to sovereign VPS is **validated**. The infrastructure layer
is complete. The remaining gap is the orchestration layer (biomeOS `graph.execute`)
which enables spring emissions without manual intervention. This is purely a
biomeOS code evolution — the infrastructure supports it today.

**Status: P0 RESOLVED. P0b VALIDATED (graph parse pass, execution gated on biomeOS v0.2).**
