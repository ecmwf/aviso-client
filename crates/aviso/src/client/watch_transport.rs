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
//! and each carrying at most [`WATCHES_PER_CONNECTION`] watches. Here a
//! "connection" means one of those clients: over HTTP/2 it is one TCP
//! connection; over HTTP/1.1 each watch has its own TCP connection anyway,
//! and the cap only groups them. When all are full the next watch gets a
//! new client; a lease is returned when its watch ends, and clients left
//! idle at the end of the set are released. Clones of an `AvisoClient`
//! share the set, so the cap holds across them.
//!
//! A new client is built synchronously, on the thread that opens the watch,
//! the first time the existing ones are full.
//!
//! These clients are built without the request timeout the caller may have
//! set. That timeout bounds a whole request, body included, and a watch's
//! body is meant to stay open: with it, every watch would be cut and
//! reconnected each time the timeout elapsed. Watches have their own opening
//! deadline and heartbeat watchdog instead.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};

use reqwest::Client as HttpClient;

use crate::ClientError;

/// Most watches one connection carries. Below the concurrent-request limits
/// of common servers and proxies (nginx 128, `HAProxy` 100), leaving room on
/// the same connection for the requests triggers make to that host, and for
/// servers that advertise a lower limit than those. The value is 64 (one
/// plus 63, which a constant `NonZeroUsize` can express without a runtime
/// check).
pub(crate) const WATCHES_PER_CONNECTION: NonZeroUsize = NonZeroUsize::MIN.saturating_add(63);

type Factory = dyn Fn() -> reqwest::Result<HttpClient> + Send + Sync;

/// The set of HTTP clients watches lease from.
pub(crate) struct WatchTransport {
    factory: Box<Factory>,
    per_connection: NonZeroUsize,
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
        per_connection: NonZeroUsize,
    ) -> Result<Arc<Self>, ClientError> {
        let first = build(&*factory)?;
        Ok(Arc::new(Self {
            factory,
            per_connection,
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
            .find(|(_, s)| s.active < self.per_connection.get())?;
        slot.active += 1;
        Some(Lease {
            transport: Arc::clone(self),
            index,
            http: slot.http.clone(),
        })
    }

    /// The slots. A panic elsewhere while the lock was held cannot leave
    /// them half-updated: every change under it is one push or one counter
    /// step. So a poisoned lock is still consistent and is used as is,
    /// rather than failing every later watch of the client.
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
        let mut slots = self.transport.lock();
        if let Some(slot) = slots.get_mut(self.index) {
            slot.active = slot.active.saturating_sub(1);
        }
        // Release idle clients at the end of the set, keeping the first.
        // Only trailing slots go, and only idle ones, so no live lease's
        // index ever points past the end.
        while slots.len() > 1 && slots.last().is_some_and(|s| s.active == 0) {
            slots.pop();
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
    factory().map_err(|e| {
        ClientError::Config(format!("failed to build the HTTP client for a watch: {e}"))
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on known-good fixtures are the expected diagnostics"
)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    fn cap(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn transport(per_connection: usize) -> (Arc<WatchTransport>, Arc<AtomicUsize>) {
        let built = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&built);
        let factory = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            HttpClient::builder().build()
        });
        (
            WatchTransport::new(factory, cap(per_connection)).unwrap(),
            built,
        )
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
        let (t, _) = transport(WATCHES_PER_CONNECTION.get());
        let leases: Vec<_> = (0..150).map(|_| t.acquire().unwrap()).collect();
        assert_eq!(t.active_per_connection(), vec![64, 64, 22]);
        drop(leases);
        // Idle clients past the first are released.
        assert_eq!(t.active_per_connection(), vec![0]);
    }

    #[test]
    fn only_idle_clients_at_the_end_are_released() {
        let (t, built) = transport(2);
        let a = t.acquire().unwrap();
        let b = t.acquire().unwrap();
        let c = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![2, 1]);
        // The first client goes idle, but a later one is still in use: the
        // set keeps both, so the busy one's position is unchanged.
        drop(a);
        drop(b);
        assert_eq!(t.active_per_connection(), vec![0, 1]);
        drop(c);
        assert_eq!(t.active_per_connection(), vec![0]);
        // The next watch reuses the first client; nothing new is built.
        let _d = t.acquire().unwrap();
        assert_eq!(t.active_per_connection(), vec![1]);
        assert_eq!(built.load(Ordering::SeqCst), 2);
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
        let streams: Vec<_> = (0..WATCHES_PER_CONNECTION.get() + 6)
            .map(|_| {
                client
                    .watch(crate::watch::WatchRequest::watch("mars"))
                    .unwrap()
            })
            .collect();
        assert_eq!(
            client.watch_transport.active_per_connection(),
            vec![WATCHES_PER_CONNECTION.get(), 6]
        );

        for stream in streams {
            stream.close().await;
        }
        // Once close() has returned, the slot is already free, and the idle
        // second client has been released.
        assert_eq!(client.watch_transport.active_per_connection(), vec![0]);

        let again = client
            .watch(crate::watch::WatchRequest::watch("mars"))
            .unwrap();
        assert_eq!(client.watch_transport.active_per_connection(), vec![1]);
        again.close().await;
    }

    /// An HTTP/2 server (cleartext) that allows `streams` concurrent requests
    /// per connection, like a proxy's stream limit. It answers every request
    /// with an opened SSE stream that stays open, and counts connections.
    async fn stream_limited_server(streams: u32) -> (String, Arc<AtomicUsize>) {
        use futures_util::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;
        use hyper_util::rt::{TokioExecutor, TokioIo};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(|_request| async {
                        let opening = bytes::Bytes::from_static(
                            b"event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n",
                        );
                        let frames =
                            futures_util::stream::iter([Ok::<_, std::convert::Infallible>(
                                Frame::data(opening),
                            )])
                            .chain(futures_util::stream::pending());
                        hyper::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(StreamBody::new(frames))
                    });
                    // reason: the connection ends when the client goes away,
                    // which is how every test here finishes.
                    hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .max_concurrent_streams(streams)
                        .serve_connection(TokioIo::new(socket), service)
                        .await
                        .ok();
                });
            }
        });
        (url, connections)
    }

    /// Opens `watches` watches, one at a time, against a server allowing 2
    /// streams per connection, with at most `per_connection` watches on each
    /// of the client's connections. The first `expected_open` must confirm;
    /// each is awaited before the next opens, so every client's connection
    /// exists before it is reused and the connection count is exact. The
    /// rest must not confirm. Returns the connections the server saw.
    async fn open_against_a_stream_limit(
        watches: usize,
        per_connection: usize,
        expected_open: usize,
    ) -> usize {
        let (url, connections) = stream_limited_server(2).await;
        let mut client = crate::AvisoClient::builder()
            .base_url(&url)
            .build()
            .unwrap();
        client.watch_transport = WatchTransport::new(
            Box::new(|| HttpClient::builder().http2_prior_knowledge().build()),
            cap(per_connection),
        )
        .unwrap();
        let mut streams = Vec::with_capacity(watches);
        for index in 0..watches {
            let stream = client
                .watch(crate::watch::WatchRequest::watch("mars"))
                .unwrap();
            if index < expected_open {
                // An upper bound only: it is reached only if the watch never
                // confirms, which is a failure whatever the machine's speed.
                let mut ready = stream.subscribe_ready();
                tokio::time::timeout(Duration::from_secs(60), ready.wait_for(|r| *r))
                    .await
                    .expect("the watch should confirm")
                    .expect("the watch should confirm");
            }
            streams.push(stream);
        }
        // The watches past the limit must not confirm. A short wait can only
        // make this check miss a regression, never fail a correct build.
        tokio::time::sleep(Duration::from_millis(500)).await;
        for stream in &streams[expected_open..] {
            assert!(
                !*stream.subscribe_ready().borrow(),
                "a watch past the stream limit confirmed"
            );
        }
        for stream in streams {
            stream.close().await;
        }
        connections.load(Ordering::SeqCst)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watches_sharing_one_connection_stop_at_the_server_stream_limit() {
        // Five watches on one connection, which allows two: the other three
        // wait for a stream that never frees. This is the failure the
        // per-connection cap prevents.
        assert_eq!(open_against_a_stream_limit(5, 5, 2).await, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn capping_watches_per_connection_opens_them_all() {
        assert_eq!(open_against_a_stream_limit(5, 2, 5).await, 3);
    }
}
