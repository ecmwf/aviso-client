// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::ClientError;

pub(super) const OPENING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(super) fn protocol(message: &str) -> ClientError {
    ClientError::StreamProtocol {
        message: format!("{message}; check base_url points to an Aviso server"),
        request_id: None,
    }
}

pub(super) fn validate_content_type(
    headers: &reqwest::header::HeaderMap,
) -> Result<(), ClientError> {
    // Parameters are allowed: `Text/Event-Stream; charset=utf-8` is valid,
    // whereas `text/html` is not an SSE response.
    let valid = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"));
    if valid {
        Ok(())
    } else {
        Err(protocol(
            "HTTP 200 response requires Content-Type text/event-stream",
        ))
    }
}

/// Consume only opening frames, leaving all subsequent data for the normal
/// dispatcher. A heartbeat or unknown event cannot confirm an Aviso stream.
pub(super) fn confirmed(
    parser: &mut finesse::Parser,
    historical: bool,
) -> Result<bool, ClientError> {
    while let Some(frame) = parser.next_frame() {
        let finesse::Frame::Message(message) = frame else {
            continue;
        };
        let expected = if historical {
            "replay_started"
        } else {
            "connection_established"
        };
        let expected_event = if historical {
            "replay-control"
        } else {
            "live-notification"
        };
        match message.event.as_str() {
            "live-notification" | "replay" | "replay-control" => {
                let value: serde_json::Value = serde_json::from_str(&message.data)
                    .map_err(|_| protocol("invalid Aviso opening control event"))?;
                if message.event == expected_event
                    && value.get("type").and_then(serde_json::Value::as_str) == Some(expected)
                {
                    return Ok(true);
                }
                return Err(protocol(
                    "unexpected event before required Aviso opening control",
                ));
            }
            "error" => {
                let wire: crate::watch::wire::WireErrorEvent = serde_json::from_str(&message.data)?;
                return Err(ClientError::StreamProtocol {
                    message: wire.message.or(wire.error).unwrap_or_else(|| {
                        "server rejected stream before opening confirmation".into()
                    }),
                    request_id: wire.request_id,
                });
            }
            "connection-closing" => {
                return Err(protocol("stream closed before opening confirmation"));
            }
            _ => {}
        }
    }
    Ok(false)
}

pub(super) async fn with_startup_budget(
    run: impl std::future::Future<Output = ()>,
    timeout: Option<std::time::Duration>,
    mut ready: tokio::sync::watch::Receiver<bool>,
    errors: tokio::sync::mpsc::Sender<Result<crate::Notification, ClientError>>,
) {
    let Some(timeout) = timeout else {
        run.await;
        return;
    };
    tokio::pin!(run);
    tokio::select! {
        biased;
        () = &mut run => return,
        confirmed = async { ready.wait_for(|confirmed| *confirmed).await.is_ok() } => {
            if !confirmed {
                return;
            }
        }
        () = tokio::time::sleep(timeout) => {
            // No data can precede confirmation, so this channel has room.
            errors.send(Err(protocol("listener startup timeout exceeded"))).await.ok();
            return;
        }
    }
    run.await;
}
