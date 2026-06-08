// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Pointer newtypes that assert `Send` so a caller-owned `void* ctx` and an
//! owned outcome pointer can be moved into a spawned runtime task. Used by the
//! watch and async surfaces.

use std::ffi::c_void;

use crate::outcome::AvisoOutcome;

/// Moves a caller-supplied `void* ctx` into a runtime task. The consumer owns
/// `ctx` and guarantees it stays valid for the operation's lifetime and is safe
/// to touch from the runtime thread; the ABI documents that contract.
#[derive(Clone, Copy)]
pub(crate) struct SendPtr(pub(crate) *mut c_void);

// SAFETY: the pointer is opaque to this library; the consumer's documented
// contract is that it is safe to use from the runtime thread.
unsafe impl Send for SendPtr {}

/// Moves an owned outcome pointer into a runtime task (used for an error built
/// before the task is spawned, where the outcome's borrowed-pointer view makes
/// it otherwise not `Send`).
pub(crate) struct SendOutcome(pub(crate) *mut AvisoOutcome);

// SAFETY: the pointed-to outcome is uniquely owned and is only ever touched by
// the single task it is moved into, so there is no aliasing across threads.
unsafe impl Send for SendOutcome {}
