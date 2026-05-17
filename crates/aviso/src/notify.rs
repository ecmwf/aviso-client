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

use reqwest::StatusCode;
use reqwest::header::AUTHORIZATION;
use url::Url;

use crate::{AvisoClient, ClientError, NotificationRequest, NotifyResponse};

impl AvisoClient {
    /// Publishes a notification to `POST /api/v1/notification`.
    ///
    /// On a `401 Unauthorized` response, the configured [`AuthProvider`] (if any) is asked to
    /// refresh and the request is retried once. A second `401` is returned to the caller as
    /// [`ClientError::Http`]; the client never loops on auth failures.
    ///
    /// # Errors
    ///
    /// - [`ClientError::Transport`] for network-level failures before the response begins (DNS,
    ///   connect, TLS). Per D16, transport errors after the request body has been sent are not
    ///   retried because the server may have processed the publish.
    /// - [`ClientError::Http`] for any non-success status, carrying the verbatim body and
    ///   `X-Request-ID` for support correlation.
    /// - [`ClientError::Decode`] when the server returned `200`/`201` but the body did not
    ///   deserialize as [`NotifyResponse`].
    /// - [`ClientError::Auth`] when the auth provider fails to produce a header or when
    ///   [`AuthProvider::refresh`] itself fails.
    pub async fn notify(&self, request: &NotificationRequest) -> crate::Result<NotifyResponse> {
        let url = self.endpoint("api/v1/notification")?;
        let response = self.do_notify_request(&url, request).await?;

        if response.status() == StatusCode::UNAUTHORIZED {
            if let Some(auth) = self.auth() {
                drop(response);
                auth.refresh().await?;
                let retry = self.do_notify_request(&url, request).await?;
                return parse_notify_response(retry).await;
            }
        }

        parse_notify_response(response).await
    }

    async fn do_notify_request(
        &self,
        url: &Url,
        request: &NotificationRequest,
    ) -> crate::Result<reqwest::Response> {
        let mut http_request = self.http().post(url.clone()).json(request);
        if let Some(auth) = self.auth() {
            let value = auth.authorization_header().await?;
            http_request = http_request.header(AUTHORIZATION, value);
        }
        http_request.send().await.map_err(ClientError::from)
    }
}

async fn parse_notify_response(response: reqwest::Response) -> crate::Result<NotifyResponse> {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);
    let body = response.bytes().await?;
    if status.is_success() {
        let parsed: NotifyResponse = serde_json::from_slice(&body)?;
        Ok(parsed)
    } else {
        let body_str = String::from_utf8_lossy(&body).into_owned();
        Err(ClientError::Http {
            status: status.as_u16(),
            body: body_str,
            request_id,
        })
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

    use serde_json::json;
    use wiremock::matchers::{header, method, path};
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
}
