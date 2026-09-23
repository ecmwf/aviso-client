// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Bounds on how much a single line or a single event may hold.
//!
//! The byte stream comes from the server, and the parser has to hold
//! bytes until a line terminator or a blank line arrives. Without a
//! bound, a server that never sends one makes the parser grow without
//! limit. The bounds here sit above the largest message the server's
//! store will pass on (a NATS `JetStream` message is 1 MiB by default and
//! 64 MiB at most), so a notification the server could deliver, a large
//! polygon included, is never refused, and a stream that does exceed
//! them is reported as an [`Overflow`] rather than kept, from `feed` or,
//! for a line that a held CR completes at end-of-stream, from `end`. A
//! bound is checked after every 64 KiB copied in, so the parser holds at
//! most a bound plus 64 KiB before it reports.

use core::fmt;

/// One mebibyte.
const MIB: usize = 1024 * 1024;

/// Bounds applied by a [`crate::Parser`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Longest line, in bytes, that may be buffered while waiting for
    /// its terminator. A notification arrives as one `data:` line, so
    /// this bounds the notification itself. Default 16 MiB.
    pub max_line_bytes: usize,
    /// Most `data:` bytes one event may accumulate before the blank line
    /// that dispatches it. Default 32 MiB.
    pub max_event_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_line_bytes: 16 * MIB,
            max_event_bytes: 32 * MIB,
        }
    }
}

impl Limits {
    /// Sets the longest line that may be buffered.
    #[must_use]
    pub fn with_max_line_bytes(mut self, bytes: usize) -> Self {
        self.max_line_bytes = bytes;
        self
    }

    /// Sets the most `data:` bytes one event may accumulate.
    #[must_use]
    pub fn with_max_event_bytes(mut self, bytes: usize) -> Self {
        self.max_event_bytes = bytes;
        self
    }
}

/// A bound in [`Limits`] was exceeded. The parser has stopped and
/// dropped what it held; the stream cannot be continued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Overflow {
    /// A line, terminated or not, is longer than `max` bytes.
    Line {
        /// The bound that was exceeded.
        max: usize,
    },
    /// One event accumulated more than `max` bytes of `data:` without a
    /// blank line to dispatch it.
    Event {
        /// The bound that was exceeded.
        max: usize,
    },
}

impl fmt::Display for Overflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Line { max } => write!(f, "SSE line exceeds {max} bytes"),
            Self::Event { max } => write!(
                f,
                "SSE event exceeds {max} bytes of data without a dispatching blank line"
            ),
        }
    }
}

impl std::error::Error for Overflow {}
