// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Schema discovery: `GET /api/v1/schema` and `GET /api/v1/schema/{event_type}`.
//!
//! Per D7 the client does not validate notifications against schemas; this module is a pure
//! discovery pass-through for tooling such as the `aviso schema list/get` CLI subcommands.
//!
//! Schemas are deserialized into permissive maps backed by [`serde_json::Value`], so the client
//! survives the server adding new identifier-rule fields or payload-config fields without a
//! schema migration on this side.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AvisoClient;
use crate::client::parse_json_response;

/// Response body from `GET /api/v1/schema`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SchemaCatalog {
    /// Server-supplied status string (typically `"success"`).
    pub status: String,
    /// Map of event-type name to the corresponding [`StreamSchema`].
    pub schema: BTreeMap<String, StreamSchema>,
    /// List of event-type names. The server provides this alongside the map for convenience.
    pub event_types: Vec<String>,
    /// Total number of schemas served.
    pub total_schemas: u32,
}

/// Response body from `GET /api/v1/schema/{event_type}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SchemaResponse {
    /// Server-supplied status string.
    pub status: String,
    /// Event type the schema applies to (echoed by the server).
    pub event_type: String,
    /// The schema itself.
    pub schema: StreamSchema,
}

/// A single stream's schema as the server reports it.
///
/// Permissive by design per D7. Identifier rules and payload configuration are stored as
/// [`serde_json::Value`] so the client passes new server-side fields through without a code
/// change here. Use the keys you know about and ignore the rest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[non_exhaustive]
pub struct StreamSchema {
    /// Optional payload configuration; shape is server-defined.
    #[serde(default)]
    pub payload: Option<Value>,
    /// Identifier field configurations, keyed by identifier name.
    #[serde(default)]
    pub identifier: BTreeMap<String, Value>,
}

impl AvisoClient {
    /// Returns the full schema catalog from `GET /api/v1/schema`.
    ///
    /// # Errors
    ///
    /// Same shape as [`AvisoClient::notify`]: [`crate::ClientError::Transport`] on network failure,
    /// [`crate::ClientError::Http`] on non-success status, [`crate::ClientError::Decode`] if the server body
    /// drifts from [`SchemaCatalog`], [`crate::ClientError::Auth`] if the auth provider cannot produce
    /// a header.
    pub async fn schema(&self) -> crate::Result<SchemaCatalog> {
        let url = self.endpoint("api/v1/schema")?;
        let response = self.send_with_refresh(|http| http.get(url.clone())).await?;
        parse_json_response(response).await
    }

    /// Returns the schema for `event_type` from `GET /api/v1/schema/{event_type}`.
    ///
    /// # Errors
    ///
    /// Same shape as [`AvisoClient::schema`]; a missing event type returns
    /// [`crate::ClientError::Http`] with `status = 404` and the server-supplied body.
    pub async fn schema_for(&self, event_type: &str) -> crate::Result<SchemaResponse> {
        crate::client::validate_path_segment(event_type)?;
        let path = format!("api/v1/schema/{event_type}");
        let url = self.endpoint(&path)?;
        let response = self.send_with_refresh(|http| http.get(url.clone())).await?;
        parse_json_response(response).await
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::auth::{AuthProvider, Bearer};
    use crate::{AvisoClient, ClientError};

    fn catalog_body() -> serde_json::Value {
        json!({
            "status": "success",
            "schema": {
                "mars": {
                    "payload": { "type": "object" },
                    "identifier": {
                        "class": { "rule": "any" },
                        "stream": { "rule": "any" }
                    }
                },
                "polygon": {
                    "identifier": {
                        "region": { "rule": "any" }
                    }
                }
            },
            "event_types": ["mars", "polygon"],
            "total_schemas": 2
        })
    }

    fn single_body() -> serde_json::Value {
        json!({
            "status": "success",
            "event_type": "mars",
            "schema": {
                "payload": { "type": "object" },
                "identifier": {
                    "class": { "rule": "any" },
                    "point_cloud": {
                        "type": "PointCloudHandler",
                        "required": true
                    }
                }
            }
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
    async fn schema_returns_catalog_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema"))
            .respond_with(ResponseTemplate::new(200).set_body_json(catalog_body()))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let catalog = client.schema().await.unwrap();

        assert_eq!(catalog.status, "success");
        assert_eq!(catalog.total_schemas, 2);
        assert_eq!(catalog.event_types, vec!["mars", "polygon"]);
        assert!(catalog.schema.contains_key("mars"));
        assert!(catalog.schema.contains_key("polygon"));
    }

    #[tokio::test]
    async fn schema_for_returns_single_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema/mars"))
            .respond_with(ResponseTemplate::new(200).set_body_json(single_body()))
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let response = client.schema_for("mars").await.unwrap();

        assert_eq!(response.event_type, "mars");
        assert!(response.schema.identifier.contains_key("class"));
        assert_eq!(
            response.schema.identifier["point_cloud"],
            json!({"type": "PointCloudHandler", "required": true})
        );
    }

    #[tokio::test]
    async fn schema_for_rejects_path_traversal_attempt() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let err = client.schema_for("../admin/wipe").await.unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn schema_for_rejects_empty_event_type() {
        let server = MockServer::start().await;
        let client = client_for(&server, None);
        let err = client.schema_for("").await.unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn schema_for_surfaces_404_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema/unknown"))
            .respond_with(
                ResponseTemplate::new(404).set_body_string("event_type 'unknown' not configured"),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, None);
        let err = client.schema_for("unknown").await.unwrap_err();
        match err {
            ClientError::Http { status, body, .. } => {
                assert_eq!(status, 404);
                assert!(body.contains("not configured"), "body={body}");
            }
            other => panic!("expected Http(404), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn schema_refreshes_and_retries_once_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema"))
            .respond_with(ResponseTemplate::new(200).set_body_json(catalog_body()))
            .mount(&server)
            .await;

        let auth: Arc<dyn AuthProvider> = Arc::new(Bearer::new("tok").unwrap());
        let client = client_for(&server, Some(auth));
        let catalog = client.schema().await.unwrap();
        assert_eq!(catalog.total_schemas, 2);
    }
}
