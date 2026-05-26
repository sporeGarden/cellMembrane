# K-Derm Topology

**Version**: 1.0.0
**Date**: May 26, 2026
**Status**: Active
**Authority**: cellMembrane team
**License**: AGPL-3.0-or-later
**Related**: `CELLMEMBRANE_ARCHITECTURE.md`, `MEMBRANE_COMPOSITION_MODEL.md`, `infra/whitePaper/gen4/architecture/SOVEREIGN_HPC_EVOLUTION.md`, `infra/whitePaper/gen4/architecture/SOVEREIGN_TRANSACTION_MEMBRANE.md`

---

## Parallel to K-NOME

K-NOME (Knowledge-Numeric Observed & Mentored Evolutionary programming)
models directly from evolutionary biology — Lenski populations, Darwinian
selection, Lamarckian inheritance via handoffs — and extends into a
programming methodology for human-AI collaboration.

K-Derm models directly from cell envelope biology — monoderm/diderm
bacteria, eukaryotic organelle membranes, vesicle transport,
endosymbiosis — and extends into a network topology model for sovereign
infrastructure.

Same pattern: observe nature, formalize the model, extend beyond the
biological into computational infrastructure.

**K-NOME is how we build. K-Derm is how what we build is shaped.**

---

## The Problem: Franklin's Current

Three existing documents use conflicting inner/outer labels for the VPS:

- `SOVEREIGN_HPC_EVOLUTION.md` calls VPS the **outer membrane**, gates
  the **inner membrane**
- `CELLMEMBRANE_FIELDMOUSE_ARCHITECTURE.md` calls VPS the **inner
  membrane**, GitHub/CDN the **outer membrane**
- `CELLMEMBRANE_ARCHITECTURE.md` avoids the issue — uses
  "extracellular/intracellular" without inner/outer

This is exactly the electron flow / conventional current problem.
Franklin chose a convention that turned out to be backwards; both
conventions then persisted, creating confusion for every student who
encounters them. The gram-positive/gram-negative labels in
`SOVEREIGN_HPC_EVOLUTION` compound it — they encode a staining
technique, not the architecture.

The fix: **absolute positions in the cell envelope**, not relative
inner/outer labels. Use **monoderm** (single membrane boundary) and
**diderm** (double membrane boundary with periplasm) to describe
topology, since these terms describe actual structure.

---

## Section 1: The Cell Envelope Model

Replace relative inner/outer with absolute layers named from inside out:

```
Monoderm (single boundary):
  cytoplasm (gate NUCLEUS) → plasma membrane (gate firewall) → environment

Diderm (double boundary):
  cytoplasm (gate NUCLEUS) → plasma membrane (gate firewall) →
  periplasm (VPS relay/routing/telemetry) → outer membrane (VPS channels) →
  environment (internet)
```

The VPS is ALWAYS in the **periplasm + outer membrane** position. It is
never "inner" — that term confused two different reference frames. The
gate's plasma membrane is the only inner boundary.

### Layer Definitions

| Layer | Position | What occupies it | Bond types within |
|-------|----------|------------------|-------------------|
| Cytoplasm | Innermost | NUCLEUS processes, UDS IPC, shared memory | Covalent only |
| Plasma membrane | Gate boundary | Gate firewall (UFW/nftables) | Covalent, Metallic |
| Periplasm | Between plasma and outer | VPS relay, routing, telemetry, attribution | Ionic, Metallic |
| Outer membrane | VPS boundary | VPS channels (Signal/Relay/Surface) | Weak, Ionic |
| Extracellular | Outermost | Public internet | Weak |

---

## Section 2: Periplasm as Cross-Bonding Mediator

The periplasm is the routing/economics/telemetry space between the
gate's plasma membrane and the VPS's outer membrane channels. It
contains:

- **Routing**: `routing_config.toml`, content-aware request dispatch
- **Telemetry**: `membrane_telemetry.sh`, shadow validation, SkunkBat
  correlation
- **Attribution**: sweetGrass braid verification at the boundary
- **Bonding classification**: which bond type applies to inbound traffic

This is where cross-bonding-model interactions become navigable. The
university example:

```
Student laptop (ionic bond to campus)
  → campus periplasm (classifies: student credential → ionic contract)
  → lab compute plasma membrane (accepts ionic → upgrades to covalent if braided)
  → HPC cluster (metallic internally, ionic from lab)
  → external cloud (weak from periplasm)
```

If the student's data is already aligned via braid (sweetGrass
provenance chain intact from prior covalent sessions), the ionic →
metallic handoff to HPC doesn't need re-authentication of the data
lineage — the braid carries the provenance across bond-type boundaries.

---

## Section 3: Monoderm and Diderm Topologies

Mapped to existing ecosystem:

| Topology | Layers | Bonding at boundary | Example |
|----------|--------|---------------------|---------|
| Monoderm | Cytoplasm → plasma membrane → environment | Covalent LAN mesh | Home lab: ironGate directly on LAN, no VPS |
| Diderm | Cytoplasm → plasma → periplasm → outer → env | Mixed per channel | Production: ironGate + VPS `membrane-relay` |
| Multi-diderm | Shared periplasm, multiple outer membranes | Per-VPS channel policy | Future: `membrane-nyc` + `membrane-eu` |
| Nested diderm | One system's outer membrane = another's periplasm | Ionic → covalent escalation | University: lab membrane is campus periplasm |

The **nested diderm** is the key new concept: a membrane boundary to one
system (e.g. the lab's outer membrane) is the periplasm to another
system (e.g. campus-wide routing). This is how complex institutional
topologies compose without requiring a single global membrane — each
administrative domain owns its own envelope, and bonding model
determines what crosses between them.

---

## Section 4: Bonding at Each Envelope Layer

Map the organo-metallo-salt model to envelope positions:

| Envelope Layer | Bond Types Crossing | What Crosses | What Does NOT Cross |
|----------------|---------------------|--------------|---------------------|
| Outer membrane → environment | Weak, Ionic | Public content, scoped API tokens | Family seed, braid internals, dag.* |
| Periplasm (routing) | Ionic, Metallic | Classified requests, telemetry, relay | Raw covalent RPC, FAMILY_SEED |
| Plasma membrane (gate) | Covalent, Metallic | Full capability, braid, workloads | Nothing blocked within family |
| Cytoplasm (NUCLEUS) | Covalent only | UDS IPC, shared memory | (everything stays) |

This directly maps to the existing `organo_metal_salt.toml` bonding
policy and the ionic `capability_deny = ["storage.*", "dag.*",
"braid.*", "crypto.*"]` rule — braid stays inside the plasma membrane;
ionic partners in the periplasm get compute results but not attribution
internals.

---

## Section 5: Terminology Reconciliation

Explicit table mapping old doc terms to new canonical terms:

| Old term (in doc X) | New canonical term | Why |
|---------------------|-------------------|-----|
| "inner membrane" (SOVEREIGN_HPC) | Plasma membrane | Consistent with biology: always the gate boundary |
| "outer membrane" (SOVEREIGN_HPC) | Outer membrane | Correct — VPS channels facing internet |
| "inner membrane" (FIELDMOUSE) | Periplasm + outer membrane | Was using "inner" relative to GitHub; resolved by absolute naming |
| "gram-negative" | Diderm | Describes structure, not staining artifact |
| "gram-positive" | Monoderm | Same |
| "cell wall" | (no equivalent) | Substrate provider; not a membrane layer |

These old documents are not modified — they become fossil record. The
K-Derm spec is canonical; old terms are referenced with reconciliation.

---

## Section 6: K-Derm Extensions Beyond Biology

These are the computational extensions that go beyond the biological
model — the "extend" part of "model from nature and extend."

### 6a: Recursive Nesting (Organelle Membranes)

Eukaryotic cells don't just have a plasma membrane — mitochondria have
their own double membrane, the nucleus has its own envelope, the ER has
its own membrane system. K-Derm allows the same: every administrative
domain (lab, department, campus, consortium) is its own K-Derm system,
and they nest recursively.

```
Consortium (outer membrane)
  → consortium periplasm (federated routing)
    → University (outer membrane)
      → campus periplasm (campus routing, bonding classification)
        → Lab (plasma membrane)
          → lab cytoplasm (covalent HPC mesh)
            → HPC organelle (own double membrane: scheduler + compute pool)
```

Each level is a self-contained envelope. The bonding model at each
boundary is independently configured. A lab can be covalent internally
and ionic to campus, while campus is ionic internally and weak to the
consortium.

### 6b: Endosymbiosis (Sovereignty Escalation)

Mitochondria were free-living bacteria that became permanent
intracellular residents over evolutionary time. K-Derm models the same
trajectory for infrastructure:

- **Phase 1 (Weak/External)**: Cloud GPU cluster appears as an external
  resource, weak bond, passive API
- **Phase 2 (Ionic/Contract)**: Formal bonding contract, BTSP scoped
  tokens, capability masks
- **Phase 3 (Metallic/Fleet)**: GPU cluster joins the delocalized
  compute pool, specialized but coordinated
- **Phase 4 (Covalent/Internalized)**: Cluster's membrane becomes part
  of the host periplasm — fully trusted, shared family seed, free
  workload routing

This maps directly to the existing bonding escalation path, but K-Derm
gives it topological semantics: endosymbiosis means the external
system's outer membrane *becomes* a layer in the host's envelope. The
boundary doesn't disappear — it transforms from a trust barrier into a
functional compartment.

### 6c: Vesicle Transport (Braid as Membrane Coat)

Cells move cargo between compartments in membrane-bound vesicles — lipid
bubbles that bud off one organelle and fuse with another. The membrane
coat determines which target accepts the vesicle (SNARE proteins in
biology).

In K-Derm, **braid is the vesicle coat**. A workload wrapped in a
sweetGrass braid carries provenance attribution that acts as the
targeting signal:

1. **Budding**: Student submits workload from lab. sweetGrass creates
   braid wrapping the DAG session + data references + attribution chain.
2. **Periplasm transit**: Braid-wrapped workload traverses campus
   periplasm. Routing logic reads the braid metadata (not the content)
   to classify bonding type and destination.
3. **Fusion**: HPC intake membrane accepts the vesicle because the braid
   proves data alignment — the DAG references are already verified via
   rhizoCrypt, the attribution chain is intact, the ionic contract
   authorizes compute on this data.
4. **Content release**: Inside the HPC organelle, the braid is verified
   and the workload executes in the metallic compute pool.

The key insight: if the student's data is already braided from prior
covalent sessions (e.g., lab work that produced the dataset), the
ionic-to-metallic handoff doesn't require re-verification of data
lineage. The braid carries the proof across bond-type boundaries.

### 6d: Channel Proteins (Bonding at Boundaries)

`SOVEREIGN_TRANSACTION_MEMBRANE.md` already maps bond types to channel
proteins:

- **Covalent = aquaporin** — always open, shared family seed,
  free-flowing
- **Ionic = gated ion channel** — BTSP scoped token opens the gate,
  method-level filtering
- **Ceremony = voltage-gated** — time-bound decay (covalent → ionic →
  weak over time)
- **Weak = passive diffusion** — read-only, no active transport

K-Derm formalizes this at every envelope layer, not just the single
membrane. Each periplasmic space has its own set of channel proteins
(bonding policies). The `[graph.bonding_policy]` section in deploy
graphs is exactly this: `tower_internal = "covalent"`, `cross_family =
"ionic"`, `public_edge = "weak"` — channel protein specificity per
boundary.

### 6e: Membrane Potential (Resource Pressure Gradients)

Cells maintain electrochemical gradients across membranes. The
computational analogue: resource load creates "osmotic pressure" across
membrane boundaries. A heavily loaded HPC has high internal pressure
that naturally routes overflow through the periplasm toward less-loaded
systems.

Braid alignment reduces the activation energy of crossing a boundary —
a pre-braided workload crosses faster because the membrane doesn't need
to verify provenance from scratch. This is the K-Derm equivalent of
facilitated diffusion.

---

## Typed Interface

The K-Derm model is encoded in `cellmembrane-types` via the `envelope`
module:

| Type | Module | Encodes |
|------|--------|---------|
| `EnvelopeTopology` | `envelope.rs` | Monoderm / Diderm |
| `EnvelopeLayer` | `envelope.rs` | Cytoplasm / Plasma / Periplasm / Outer / Extracellular |
| `ChannelProtein` | `envelope.rs` | Aquaporin / GatedIon / VoltageGated / PassiveDiffusion |
| `BondType` | `envelope.rs` | Covalent / Metallic / Ionic / Ceremony / Weak |
| `BraidPolicy` | `envelope.rs` | PassThrough / Verify / Block |
| `BoundaryPolicy` | `envelope.rs` | Per-layer bond + channel protein + braid rules |
| `MembraneConfig.topology` | `config.rs` | Configuration field selecting the envelope topology |

---

## What This Spec Does NOT Cover

- Runtime periplasm routing logic (biomeOS work)
- Full K-Derm recursive nesting implementation (model established,
  implementation follows in later cycles)
- sunCloud metabolic economics in the periplasm (separate track)
- Actual resource-pressure-based routing (membrane potential is
  conceptual in this cycle)

---

## Cross-References

- K-NOME methodology: `infra/whitePaper/gen3/about/K_NOME_PROGRAMMING.md`
- K-NOME gen4 extensions: `infra/whitePaper/gen4/knome/README.md`
- Organo-metallo-salt bonding: `primals/biomeOS/specs/NUCLEUS_BONDING_MODEL.md`
- Channel proteins: `infra/whitePaper/gen4/architecture/SOVEREIGN_TRANSACTION_MEMBRANE.md`
- Composition model: `specs/MEMBRANE_COMPOSITION_MODEL.md`
- cellMembrane architecture: `specs/CELLMEMBRANE_ARCHITECTURE.md`
- NUCLEUS atomics: `infra/whitePaper/technical/NUCLEUS_ARCHITECTURE.md`
