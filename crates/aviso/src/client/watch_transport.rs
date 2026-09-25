// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The HTTP connections watches run on.
//!
//! A watch holds one HTTP request open for as long as it runs. Over HTTP/2
//! every request to a server shares one connection per `reqwest::Client`,
//! and servers and proxies cap how many requests a connection may carry at
//! once: nginx allows 128 by default, `HAProxy` 100. A watch beyond the cap
//! is not refused; its request waits inside the HTTP library for a free
//! slot, which never comes while the other watches stay open, so the watch
//! fails its opening deadline. A process with many watches on one client
//! would lose every watch past the cap.
//!
//! So watches do not share the client's own connection. They lease one of
//! a small set of `reqwest::Client`s, each with its own connection pool,
//! and each carrying at most [`WATCHES_PER_CONNECTION`] watches. When all
//! are full the next watch gets a new one; a lease is returned when its
//! watch ends. Clones of an `AvisoClient` share the set, so the cap holds
//! across them.
//!
//! These clients are built without the request timeout the caller may have
//! set. That timeout bounds a whole request, body included, and a watch's
//! body is meant to stay open: with it, every watch was cut and reconnected
//! each time the timeout elapsed. Watches have their own opening deadline
//! and heartbeat watchdog instead.

use std::sync::{Arc, Mutex, PoisonError};

use reqwest::Client as HttpClient;

use crate::ClientError;

/// Most watches one connection carries. Below the concurrent-request limits
/// of common servers and proxies (nginx 128, `HAProxy` 100), leaving room on
/// the same connection for the requests triggers make to that host, and for
/// servers that advertise a lower limit than those.
pub(crate) const WATCHES_PER_CONNECTION: usize = 64;

type Factory = dyn Fn() -> reqwest::Result<HttpClient> + Send + Sync;

/// The set of HTTP clients watches lease from.
pub(crate) struct WatchTransport {
    factory: Box<Factory>,
    per_connection: usize,
    slots: Mutex<Vec<Slot>>,
}

struct Slot {
    http: HttpClient,
    active: usize,
}

/// A watch's claim on one connection. Dropping it frees the slot.
pub(crate) struct Lease {
    transport: Arc<WatchTransport>,
    index: usize,
    http: HttpClient,
}

impl WatchTransport {
    /// Builds the first client now, so a configuration `reqwest` rejects
    /// fails at `build()` rather than at the first watch.
    pub(crate) fn new(
        factory: Box<Factory>,
        per_connection: usize,
    ) -> Result<Arc<Self>, ClientError> {
        let first = build(&*factory)?;
        Ok(Arc::new(Self {
            factory,
            per_connection: per_connection.max(1),
            slots: Mutex::new(vec![Slot {
                http: first,
                active: 0,
            }]),
        }))
    }

    /// Leases a slot on the first client with room, building a new client
    /// when every existing one is full.
    pub(crate) fn acquire(self: &Arc<Self>) -> Result<Lease, ClientError> {
        if let Some(lease) = self.lease_first_free(&mut self.lock()) {
            return Ok(lease);
        }
        // Building a client can read the system's certificate store, so it
        // happens without the lock held; watches ending meanwhile are not
        // kept waiting. Another watch may have freed or added room while
        // this one built, so look again before adding the new client.
        let built = build(&*self.factory)?;
        let mut slots = self.lock();
        if let Some(lease) = self.lease_first_free(&mut slots) {
            return Ok(lease);
        }
        let index = slots.len();
        slots.push(Slot {
            http: built.clone(),
            active: 1,
        });
        Ok(Lease {
            transport: Arc::clone(self),
            index,
            http: built,
        })
    }

    fn lease_first_free(self: &Arc<Self>, slots: &mut [Slot]) -> Option<Lease> {
        let (index, slot) = slots
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.active < self.per_connection)?;
        slot.active += 1;
        Some(Lease {
            transport: Arc::clone(self),
            index,
            http: slot.http.clone(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn active_per_connection(&self) -> Vec<usize> {
        self.lock().iter().map(|s| s.active).collect()
    }
}

impl Lease {
    /// The client to send this watch's requests with, reconnects included.
    pub(crate) fn http(&self) -> &HttpClient {
        &self.http
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(slot) = self.transport.lock().get_mut(self.index) {
            slot.active = slot.active.saturating_sub(1);
        }
    }
}

/// Builds the client for ordinary requests and the transport for watches
/// from one set of settings. Only the ordinary client gets `timeout`.
pub(crate) fn http_clients(
    user_agent: String,
    extra_root_certs: Vec<reqwest::Certificate>,
    danger_accept_invalid_certs: bool,
    timeout: Option<std::time::Duration>,
) -> Result<(HttpClient, Arc<WatchTransport>), ClientError> {
    let base = move || {
        let mut builder = HttpClient::builder().user_agent(user_agent.clone());
        for cert in &extra_root_certs {
            builder = builder.add_root_certificate(cert.clone());
        }
        if danger_accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }
        builder
    };
    let mut ordinary = base();
    if let Some(timeout) = timeout {
        ordinary = ordinary.timeout(timeout);
    }
    let http = ordinary
        .build()
        .map_err(|e| ClientError::Config(format!("failed to build HTTP client: {e}")))?;
    let watches = WatchTransport::new(Box::new(move || base().build()), WATCHES_PER_CONNECTION)?;
    Ok((http, watches))
}

fn build(factory: &Factory) -> Result<HttpClient, ClientError> {
    factory().map_err(|e| ClientError::Config(format!("failed to build HTTP client: {e}")))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on known-good fixtures is the expected diagnostic"
)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn transport(per_connection: usize) -> (Arc<WatchTransport>, Arc<AtomicUsize>) {
        let built = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&built);
        let factory = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            HttpClient::builder().build()
        });
        (WatchTransport::new(factory, per_connection).unwrap(), built)
    }

    #[test]
    fn watches_share_a_connection_until_it_is_full() {
        let (t, built) = transport(2);
        let _a = t.acquire().unwrap();
        let _b = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![2]);
        assert_eq!(built.load(Ordering::SeqCst), 1);

        let _c = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![2, 1]);
        assert_eq!(built.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_finished_watch_frees_its_slot_for_the_next() {
        let (t, built) = transport(2);
        let a = t.acquire().unwrap();
        let _b = t.acquire().unwrap();
        let _c = t.acquire().unwrap();
        drop(a);
        assert_eq!(t.active_per_connection(), vec![1, 1]);

        // The freed slot on the first connection is reused; nothing new is built.
        let _d = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![2, 1]);
        assert_eq!(built.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn many_watches_spread_over_as_few_connections_as_the_cap_allows() {
        let (t, _) = transport(WATCHES_PER_CONNECTION);
        let leases: Vec<_> = (0..150).map(|_| t.acquire().unwrap()).collect();
        assert_eq!(t.active_per_connection(), vec![64, 64, 22]);
        drop(leases);
        assert_eq!(t.active_per_connection(), vec![0, 0, 0]);
    }

    /// Accepts connections and answers each with an opened, idle SSE stream.
    async fn holding_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    if socket.read(&mut buf).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let opening =
                        "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                         Transfer-Encoding: chunked\r\n\r\n{:X}\r\n{opening}\r\n",
                        opening.len()
                    );
                    if socket.write_all(response.as_bytes()).await.is_ok() {
                        while socket.read(&mut buf).await.unwrap_or(0) > 0 {}
                    }
                });
            }
        });
        url
    }

    // Multi-threaded, as in the Python and C/C++ bindings: there a supervisor
    // ends on one worker while the caller resumes on another.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watches_lease_connections_and_return_them_when_closed() {
        let url = holding_server().await;
        let client = crate::AvisoClient::builder()
            .base_url(&url)
            .build()
            .unwrap();
        let streams: Vec<_> = (0..WATCHES_PER_CONNECTION + 6)
            .map(|_| {
                client
                    .watch(crate::watch::WatchRequest::watch("mars"))
                    .unwrap()
            })
            .collect();
        assert_eq!(
            client.watch_transport.active_per_connection(),
            vec![WATCHES_PER_CONNECTION, 6]
        );

        for stream in streams {
            stream.close().await;
        }
        // Once close() has returned, the slot is already free.
        assert_eq!(client.watch_transport.active_per_connection(), vec![0, 0]);

        let again = client
            .watch(crate::watch::WatchRequest::watch("mars"))
            .unwrap();
        assert_eq!(client.watch_transport.active_per_connection(), vec![1, 0]);
        again.close().await;
    }

    #[test]
    fn a_cap_of_zero_is_treated_as_one() {
        let (t, _) = transport(0);
        let _a = t.acquire().unwrap();
        let _b = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![1, 1]);
    }
}
