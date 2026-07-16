// SPDX-License-Identifier: AGPL-3.0-or-later

//! impulsePotential — inter-gate coordination via membrane action potentials.
//!
//! Impulses are TOML files in `infra/wateringHole/impulses/active/` that
//! ride alongside code pushes. Gates fire impulses with `impulse.post` (rP),
//! sense pending potential with `potential.sense` (qS), acknowledge with
//! `impulse.ack` (rP+wF), and archive with `impulse.archive` (wF).
//!
//! Triad mapping:
//!   - `impulse.post`    → rootPulse (ACTION) — fire action potential
//!   - `impulse.ack`     → rootPulse + waterFall (ACTION + SYNC)
//!   - `impulse.archive` → waterFall (SYNC) — discharge spent impulses
//!   - `potential.sense`  → quorumSignal (SENSE) — measure membrane potential
//!   - `potential.check`  → quorumSignal (SENSE) — gradient health

mod lifecycle;
mod parse;
mod policy;
mod primal;
mod sync;
mod types;

pub use types::{ImpulseType, PostArgs, Priority, SyncDivergeArgs};

pub use lifecycle::{ack, archive, check, post, sense};
pub use primal::discover_socket;
pub use sync::post_sync_diverge;
