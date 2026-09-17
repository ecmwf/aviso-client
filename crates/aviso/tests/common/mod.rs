// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

/// Add the opening control matching the request's resolved resume position.
pub fn opened_sse(request: &wiremock::Request, body: &str) -> wiremock::ResponseTemplate {
    let historical = request
        .body_json::<serde_json::Value>()
        .is_ok_and(|value| value.get("from_id").is_some() || value.get("from_date").is_some());
    let (event, tag) = if historical {
        ("replay-control", "replay_started")
    } else {
        ("live-notification", "connection_established")
    };
    wiremock::ResponseTemplate::new(200).set_body_raw(
        format!("event: {event}\ndata: {{\"type\":\"{tag}\"}}\n\n{body}"),
        "text/event-stream",
    )
}
