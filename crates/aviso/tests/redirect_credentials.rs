// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The `Authorization` header must not follow a redirect that leaves the
//! origin it was meant for.
//!
//! The plaintext-address rule in `aviso::auth` checks the address a client is
//! built with. A server could still answer an `https` request with a redirect
//! to plain `http`, so the guarantee also depends on the HTTP layer dropping
//! the credential when a redirect changes host, port or scheme. That is
//! `reqwest`'s behaviour today. These tests pin it, so an upgrade that changed
//! it would fail here rather than quietly widen where a credential can travel.
//!
//! A scheme change cannot be staged without TLS, so the tests exercise a port
//! change on the same host, which `reqwest` handles in the same branch.

use std::sync::Arc;

use aviso::AvisoClient;
use aviso::auth::Bearer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn authorization_seen(requests: &[Request]) -> Vec<Option<String>> {
    requests
        .iter()
        .map(|r| {
            r.headers
                .get("authorization")
                .map(|v| v.to_str().unwrap_or("<not utf-8>").to_owned())
        })
        .collect()
}

#[tokio::test]
async fn a_redirect_to_another_origin_drops_the_authorization_header() -> TestResult {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("{}/api/v1/schema", second.uri()).as_str(),
        ))
        .mount(&first)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success", "schema": {}, "event_types": [], "total_schemas": 0
        })))
        .mount(&second)
        .await;

    let client = AvisoClient::builder()
        .base_url(first.uri())
        .auth(Arc::new(Bearer::new("must-not-leave-first")?))
        .build()?;
    client.schema().await?;

    let at_first = authorization_seen(&first.received_requests().await.unwrap_or_default());
    let at_second = authorization_seen(&second.received_requests().await.unwrap_or_default());
    assert_eq!(
        at_first,
        vec![Some("Bearer must-not-leave-first".to_owned())],
        "the origin the client was built for receives the credential"
    );
    assert_eq!(
        at_second,
        vec![None],
        "the redirect target, a different origin, must not receive it"
    );
    Ok(())
}

#[tokio::test]
async fn a_redirect_within_the_same_origin_keeps_the_authorization_header() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/moved/schema"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/moved/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success", "schema": {}, "event_types": [], "total_schemas": 0
        })))
        .mount(&server)
        .await;

    let client = AvisoClient::builder()
        .base_url(server.uri())
        .auth(Arc::new(Bearer::new("stays-on-origin")?))
        .build()?;
    client.schema().await?;

    let seen = authorization_seen(&server.received_requests().await.unwrap_or_default());
    assert_eq!(
        seen,
        vec![
            Some("Bearer stays-on-origin".to_owned()),
            Some("Bearer stays-on-origin".to_owned()),
        ],
        "a same-origin redirect is an ordinary part of talking to that server"
    );
    Ok(())
}
