// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deprecated: use [`crate::impulse`] instead.
//!
//! The `signal.*` command family was renamed to `impulse.*` / `potential.*`
//! in Wave 63 to align with the impulsePotential coordination standard.
//! This module exists only for backward compatibility; it will be removed
//! in Wave 66.

#[deprecated(since = "0.9.31", note = "use `impulse::ImpulseType` instead")]
pub type SignalType = crate::impulse::ImpulseType;

#[deprecated(since = "0.9.31", note = "use `impulse::ImpulseFile` instead")]
pub type SignalFile = crate::impulse::ImpulseFile;

#[deprecated(since = "0.9.31", note = "use `impulse::ImpulseFrom` instead")]
pub type SignalFrom = crate::impulse::ImpulseFrom;

#[deprecated(since = "0.9.31", note = "use `impulse::ImpulseTo` instead")]
pub type SignalTo = crate::impulse::ImpulseTo;
