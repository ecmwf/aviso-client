// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `AvisoClient::notify` implementation.
//!
//! The notify path covers:
//!
//! - POST to `api/v1/notification` with a JSON-serialized [`NotificationRequest`].
//! - `Authorization` header sourced from the optional [`AuthProvider`].
//! - On `401 Unauthorized`: call [`AuthProvider::refresh`] once and retry the request once. Per
//!   D8 ("refresh on 401, retry once"). On a second `401`, surface the error verbatim; do not
//!   loop.
//! - On any other non-success status: surface [`ClientError::Http`] with the verbatim response
//!   body and the server's `X-Request-ID` header, per D7.
//! - On ambiguous transport failure (request body sent but no response received): do not retry
//!   per D16.

use futures_util::stream::{self, StreamExt};

use crate::client::parse_json_response;
use crate::{AvisoClient, NotificationRequest, NotifyResponse};

/// Default number of notifications [`AvisoClient::notify_many`] keeps in flight
/// at once when the caller passes `0` for the concurrency argument.
///
/// Chosen as a middle ground: high enough to collapse a model time step's worth
/// of polygons into roughly one round-trip over an HTTP/2 connection, low enough
/// not to open an unbounded number of in-flight requests against the server. A
/// caller that knows its own batch sizes and the server's capacity can pass an
/// explicit value instead.
pub const DEFAULT_NOTIFY_CONCURRENCY: usize = 16;

impl AvisoClient {
    /// Publishes a notification to `POST /api/v1/notification`.
    ///
    /// On a `401 Unauthorized` response, the configured [`crate::auth::AuthProvider`] (if any)
    /// is asked to refresh and the request is retried once. A second `401` is returned to the
    /// caller as [`crate::ClientError::Http`]; the client never loops on auth failures.
    ///
    /// # Errors
    ///
    /// - [`crate::ClientError::Transport`] for network-level failures before the response begins
    ///   (DNS, connect, TLS). Per D16, transport errors after the request body has been sent are
    ///   not retried because the server may have processed the publish.
    /// - [`crate::ClientError::Http`] for any non-success status, carrying the verbatim body and
    ///   `X-Request-ID` for support correlation.
    /// - [`crate::ClientError::Decode`] when the server returned `200`/`201` but the body did not
    ///   deserialize as [`NotifyResponse`].
    /// - [`crate::ClientError::Auth`] when the auth provider fails to produce a header or when
    ///   `AuthProvider::refresh` itself fails.
    pub async fn notify(&self, request: &NotificationRequest) -> crate::Result<NotifyResponse> {
        let url = self.endpoint("api/v1/notification")?;
        let response = self
            .send_with_refresh(|http| http.post(url.clone()).json(request))
            .await?;
        parse_json_response(response).await
    }

    /// Publishes many notifications concurrently and returns one result per
    /// request, in the same order as `requests`.
    ///
    /// At most `concurrency` requests are in flight at once; pass `0` to use
    /// [`DEFAULT_NOTIFY_CONCURRENCY`]. Over an HTTP/2 connection the in-flight
    /// requests share one connection, so a batch that would take N sequential
    /// round-trips finishes in roughly one. Each request is published with the
    /// same per-request contract as [`Self::notify`], including the
    /// `401 -> refresh -> retry once` behaviour.
    ///
    /// The batch is not atomic. Items succeed or fail independently, a failure
    /// does not stop the rest, and `result[i]` is the outcome of `requests[i]`.
    /// As with [`Self::notify`], an item that fails with an ambiguous transport
    /// error after its body was sent is not retried, because the server may have
    /// processed it. Dropping the returned future cancels in-flight requests
    /// without rolling back any the server already accepted.
    ///
    /// An empty `requests` slice returns an empty vector and sends nothing.
    pub async fn notify_many(
        &self,
        requests: &[NotificationRequest],
        concurrency: usize,
    ) -> Vec<crate::Result<NotifyResponse>> {
        let limit = if concurrency == 0 {
            DEFAULT_NOTIFY_CONCURRENCY
        } else {
            concurrency
        };
        let mut indexed: Vec<(usize, crate::Result<NotifyResponse>)> =
            stream::iter(0..requests.len())
                .map(|index| async move { (index, self.notify(&requests[index]).await) })
                .buffer_unordered(limit)
                .collect()
                .await;
        indexed.sort_by_key(|(index, _)| *index);
        indexed.into_iter().map(|(_, result)| result).collect()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on constructor success and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::auth::{AuthProvider, Bearer};
    use crate::{AvisoClient, ClientError, NotificationRequest};

    fn ok_body() -> serde_json::Value {
        json!({
            "status": "success",
            "request_id": "req-abc",
            "processed_at": "2026-05-17T12:34:56Z"
        })
    }

    fn client_for(server: &MockServer, auth: Option<Arc<dyn AuthProvider>>) -> AvisoClient {
        let mut builder = AvisoClient::builder().base_url(server.uri());
        if let Some(a) = auth {
            builder = builder.auth(a);
        }
        builder.build().unwrap()
    }

    #[tokio::test]
    async fn returns_notify_response_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let response = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap();

        assert_eq!(response.status, "success");
        assert_eq!(response.request_id, "req-abc");
        assert_eq!(response.processed_at, "2026-05-17T12:34:56Z");
    }

    #[tokio::test]
    async fn surfaces_http_4xx_with_body_and_request_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("x-request-id", "req-bad")
                    .set_body_string("identifier field 'class' is required"),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        match err {
            ClientError::Http {
                status,
                body,
                request_id,
            } => {
                assert_eq!(status, 400);
                assert!(body.contains("identifier field"), "body={body}");
                assert_eq!(request_id.as_deref(), Some("req-bad"));
            }
            other => panic!("expected Http(400), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn surfaces_http_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::Http { status: 500, .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn refreshes_and_retries_once_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .mount(&server)
            .await;

        let auth: Arc<dyn AuthProvider> = Arc::new(Bearer::new("tok").unwrap());
        let client = client_for(&server, Some(auth));

        let response = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap();
        assert_eq!(response.status, "success");
    }

    #[tokio::test]
    async fn second_401_is_surfaced_without_a_second_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(401).insert_header("x-request-id", "req-still-bad"))
            .expect(2)
            .mount(&server)
            .await;

        let auth: Arc<dyn AuthProvider> = Arc::new(Bearer::new("tok").unwrap());
        let client = client_for(&server, Some(auth));

        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::Http { status: 401, .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn no_retry_on_401_when_no_auth_provider_configured() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::Http { status: 401, .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn sends_authorization_header_when_auth_is_configured() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .expect(1)
            .mount(&server)
            .await;

        let auth: Arc<dyn AuthProvider> = Arc::new(Bearer::new("tok").unwrap());
        let client = client_for(&server, Some(auth));

        client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn transport_error_surfaces_as_transport_variant() {
        // Port 1 is unbindable on Unix-like systems; connecting fails before any HTTP exchange.
        let client = AvisoClient::builder()
            .base_url("http://127.0.0.1:1")
            .build()
            .unwrap();
        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn malformed_success_body_surfaces_as_decode_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json {"))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client
            .notify(&NotificationRequest::new("mars"))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::Decode(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn notify_many_preserves_input_order() {
        let server = MockServer::start().await;
        for i in 0..3 {
            Mock::given(method("POST"))
                .and(path("/api/v1/notification"))
                .and(body_partial_json(json!({ "event_type": format!("e{i}") })))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "status": "success",
                    "request_id": format!("r{i}"),
                    "processed_at": "2026-05-17T12:34:56Z",
                })))
                .mount(&server)
                .await;
        }

        let client = client_for(&server, None);
        let requests: Vec<NotificationRequest> = (0..3)
            .map(|i| NotificationRequest::new(format!("e{i}")))
            .collect();
        let results = client.notify_many(&requests, 0).await;

        assert_eq!(results.len(), 3);
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.as_ref().unwrap().request_id, format!("r{i}"));
        }
    }

    #[tokio::test]
    async fn notify_many_preserves_order_when_completion_is_out_of_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .and(body_partial_json(json!({ "event_type": "slow" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(200))
                    .set_body_json(json!({
                        "status": "success",
                        "request_id": "slow",
                        "processed_at": "2026-05-17T12:34:56Z",
                    })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .and(body_partial_json(json!({ "event_type": "fast" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(10))
                    .set_body_json(json!({
                        "status": "success",
                        "request_id": "fast",
                        "processed_at": "2026-05-17T12:34:56Z",
                    })),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let requests = vec![
            NotificationRequest::new("slow"),
            NotificationRequest::new("fast"),
        ];
        let results = client.notify_many(&requests, 0).await;

        assert_eq!(results[0].as_ref().unwrap().request_id, "slow");
        assert_eq!(results[1].as_ref().unwrap().request_id, "fast");
    }

    #[tokio::test]
    async fn notify_many_reports_per_item_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .and(body_partial_json(json!({ "event_type": "ok" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .and(body_partial_json(json!({ "event_type": "bad" })))
            .respond_with(ResponseTemplate::new(400).set_body_string("rejected"))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let requests = vec![
            NotificationRequest::new("ok"),
            NotificationRequest::new("bad"),
        ];
        let results = client.notify_many(&requests, 0).await;

        assert!(results[0].is_ok());
        assert!(
            matches!(results[1], Err(ClientError::Http { status: 400, .. })),
            "got {:?}",
            results[1]
        );
    }

    #[tokio::test]
    async fn notify_many_empty_input_returns_empty() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let results = client.notify_many(&[], 0).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn notify_many_runs_requests_concurrently() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notification"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(150))
                    .set_body_json(ok_body()),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let requests: Vec<NotificationRequest> =
            (0..8).map(|_| NotificationRequest::new("mars")).collect();

        let start = std::time::Instant::now();
        let results = client.notify_many(&requests, 8).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 8);
        assert!(results.iter().all(Result::is_ok));
        assert!(
            elapsed < Duration::from_millis(700),
            "8 requests delayed 150ms each with concurrency 8 should overlap into roughly one round-trip; took {elapsed:?}"
        );
    }
}
