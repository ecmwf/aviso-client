//! Admin endpoints: wipe a stream, wipe all streams, delete a single notification.
//!
//! All three sit under `/api/v1/admin/` on `aviso-server` and require operator-level
//! authentication. The client provides them as plain methods on [`AvisoClient`]; CLI tooling is
//! expected to add its own confirmation prompt before invocation.

use crate::AvisoClient;
use crate::client::parse_json_response_optional;

impl AvisoClient {
    /// Wipes all notifications for the given `stream_name` via `DELETE /api/v1/admin/wipe/stream`.
    ///
    /// The server expects the stream name in the request body as `{ "stream_name": "<name>" }`.
    ///
    /// # Errors
    ///
    /// Returns the same error shape as [`AvisoClient::notify`]: [`crate::ClientError::Transport`] on
    /// network failure, [`crate::ClientError::Http`] on non-success status (with verbatim body and
    /// `X-Request-ID`), [`crate::ClientError::Auth`] on auth-provider failure.
    pub async fn wipe_stream(&self, stream_name: &str) -> crate::Result<()> {
        let url = self.endpoint("api/v1/admin/wipe/stream")?;
        let body = serde_json::json!({ "stream_name": stream_name });
        let response = self
            .send_with_refresh(|http| http.delete(url.clone()).json(&body))
            .await?;
        parse_json_response_optional(response).await
    }

    /// Wipes every stream via `DELETE /api/v1/admin/wipe/all`. Operator-only.
    ///
    /// # Errors
    ///
    /// Same shape as [`AvisoClient::wipe_stream`].
    pub async fn wipe_all(&self) -> crate::Result<()> {
        let url = self.endpoint("api/v1/admin/wipe/all")?;
        let response = self
            .send_with_refresh(|http| http.delete(url.clone()))
            .await?;
        parse_json_response_optional(response).await
    }

    /// Deletes a single notification via `DELETE /api/v1/admin/notification/{notification_id}`.
    ///
    /// `notification_id` is the full `<event_type>@<sequence>` id the server emits in
    /// `CloudEvents` (per D9). The `@` character is preserved verbatim in the URL because the
    /// server-side route matches on the literal `@`-joined id. Identifiers containing `/` are
    /// invalid (the server never emits them) and will be rejected by the server with `400` or
    /// `404`.
    ///
    /// # Errors
    ///
    /// Same shape as [`AvisoClient::wipe_stream`]. A missing id returns
    /// [`crate::ClientError::Http`] with `status = 404`.
    pub async fn delete_notification(&self, notification_id: &str) -> crate::Result<()> {
        crate::client::validate_path_segment(notification_id)?;
        let url = self.endpoint(&format!("api/v1/admin/notification/{notification_id}"))?;
        let response = self
            .send_with_refresh(|http| http.delete(url.clone()))
            .await?;
        parse_json_response_optional(response).await
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
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::auth::{AuthProvider, Bearer};
    use crate::{AvisoClient, ClientError};

    fn client_for(server: &MockServer, auth: Option<Arc<dyn AuthProvider>>) -> AvisoClient {
        let mut builder = AvisoClient::builder().base_url(server.uri());
        if let Some(a) = auth {
            builder = builder.auth(a);
        }
        builder.build().unwrap()
    }

    #[tokio::test]
    async fn wipe_stream_sends_stream_name_in_body() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/wipe/stream"))
            .and(body_json(json!({ "stream_name": "mars" })))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        client.wipe_stream("mars").await.unwrap();
    }

    #[tokio::test]
    async fn wipe_all_hits_correct_path_and_method() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/wipe/all"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        client.wipe_all().await.unwrap();
    }

    #[tokio::test]
    async fn delete_notification_rejects_path_traversal_attempt() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let err = client.delete_notification("../wipe/all").await.unwrap_err();
        assert!(
            matches!(err, ClientError::Config(_)),
            "expected Config validation error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn delete_notification_rejects_slash_in_id() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let err = client.delete_notification("mars/42").await.unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn delete_notification_rejects_empty_id() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let err = client.delete_notification("").await.unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn delete_notification_keeps_at_sign_in_path_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/notification/mars@42"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        client.delete_notification("mars@42").await.unwrap();
    }

    #[tokio::test]
    async fn delete_notification_surfaces_404() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/notification/missing@1"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client.delete_notification("missing@1").await.unwrap_err();
        match err {
            ClientError::Http { status, body, .. } => {
                assert_eq!(status, 404);
                assert!(body.contains("not found"), "body={body}");
            }
            other => panic!("expected Http(404), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn admin_refreshes_and_retries_once_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/wipe/all"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/wipe/all"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let auth: Arc<dyn AuthProvider> = Arc::new(Bearer::new("tok").unwrap());
        let client = client_for(&server, Some(auth));
        client.wipe_all().await.unwrap();
    }
}
