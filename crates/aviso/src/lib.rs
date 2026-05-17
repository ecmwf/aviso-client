//! Core client library for [`aviso-server`], ECMWF's notification service for
//! data-driven workflows.
//!
//! The public surface grows as features land; the design rationale for each
//! choice lives in `docs/src/internals/decisions.md` and is referenced by
//! stable ADR id.
//!
//! [`aviso-server`]: https://github.com/ecmwf/aviso-server

#![forbid(unsafe_code)]

mod admin;
pub mod auth;
mod client;
mod error;
pub mod notification;
mod notify;
pub mod schema;

pub use client::{AvisoClient, AvisoClientBuilder};
pub use error::{ClientError, Result};
pub use notification::{Notification, NotificationRequest, NotifyResponse, parse_cloudevent_id};
pub use schema::{SchemaCatalog, SchemaResponse, StreamSchema};

/// Version string of the `aviso` crate, sourced from Cargo metadata.
///
/// Exposed so the CLI and the Python extension can render a single, consistent
/// version line without re-reading `CARGO_PKG_VERSION` themselves.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
