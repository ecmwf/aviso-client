//! WHATWG Server-Sent Events parser. Sync, push-based, no I/O.
//!
//! `finesse` implements the parsing algorithm from the HTML Living
//! Standard, sections 9.2.5 (parsing an event stream) and 9.2.6
//! (interpreting an event stream). It takes raw bytes from any
//! source and yields typed frames; it owns no HTTP transport, no
//! reconnect logic, and no aviso-specific semantics. Those concerns
//! belong to the consumer.
//!
//! The crate is reusable in principle but, while it remains a private
//! workspace member of `aviso-client`, it is not published to
//! crates.io. The first external consumer makes that decision worth
//! revisiting.

#![forbid(unsafe_code)]
