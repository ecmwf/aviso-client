// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `AvisoClient::watch_many`: several watches through one stream.

use std::collections::HashSet;

use super::AvisoClient;
use super::watch_spawn::compute_resume_key;
use crate::ClientError;
use crate::watch::{
    ErrorPolicy, MultiNotificationStream, NotificationStream, WatchRequest, WireWatchRequest,
};

impl AvisoClient {
    /// Opens one watch per named request and merges them into one stream.
    ///
    /// Each item is the watch's name with its notification, so the reader
    /// knows where it came from. Watches are read in turn; a busy one cannot
    /// starve a quiet one. The stream ends when every watch has ended, for
    /// example when each was opened in replay-only mode. `policy` decides
    /// what a failing watch does to the others; see [`ErrorPolicy`].
    ///
    /// Every request is checked before any watch is opened, so a mistake in
    /// one never leaves the others running. Each watch keeps its own resume
    /// position in the state store, the same one [`Self::watch`] would use
    /// for that request; the name is not part of it.
    ///
    /// ```no_run
    /// use aviso::AvisoClient;
    /// use aviso::watch::{ErrorPolicy, WatchRequest};
    /// use futures_util::StreamExt;
    /// use serde_json::json;
    ///
    /// # async fn run(client: AvisoClient) -> aviso::Result<()> {
    /// let surface = WatchRequest::watch("data")
    ///     .with_filter([("stream".to_string(), json!("oper"))].into());
    /// let wave = WatchRequest::watch("data")
    ///     .with_filter([("stream".to_string(), json!("wave"))].into());
    /// let mut stream =
    ///     client.watch_many([("surface", surface), ("wave", wave)], ErrorPolicy::Continue)?;
    /// while let Some(item) = stream.next().await {
    ///     match item {
    ///         Ok((name, notification)) => println!("{name}: {}", notification.sequence),
    ///         Err(e) => eprintln!("{e}"),
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when no Tokio runtime is entered, there
    /// are no requests, a name is empty or used twice, or a request is invalid
    /// (the message names it). Opening a watch can also fail as
    /// [`Self::watch`] can, for example if an HTTP client cannot be built; the
    /// watches opened before it are then cancelled.
    pub fn watch_many<I, N>(
        &self,
        requests: I,
        policy: ErrorPolicy,
    ) -> crate::Result<MultiNotificationStream>
    where
        I: IntoIterator<Item = (N, WatchRequest)>,
        N: Into<String>,
    {
        let requests: Vec<(String, WatchRequest)> = requests
            .into_iter()
            .map(|(name, request)| (name.into(), request))
            .collect();
        tokio::runtime::Handle::try_current().map_err(|_| {
            ClientError::Config("AvisoClient::watch_many requires a Tokio runtime".into())
        })?;
        if requests.is_empty() {
            return Err(ClientError::Config(
                "watch_many needs at least one request".into(),
            ));
        }
        let mut seen = HashSet::new();
        for (name, request) in &requests {
            if name.is_empty() {
                return Err(ClientError::Config(
                    "watch_many: a watch name must not be empty".into(),
                ));
            }
            if !seen.insert(name.as_str()) {
                return Err(ClientError::Config(format!(
                    "watch_many: the watch name '{name}' is used twice"
                )));
            }
            self.check_watch_request(request)
                .map_err(|e| named(name, e))?;
        }
        let mut streams: Vec<(String, NotificationStream)> = Vec::with_capacity(requests.len());
        for (name, request) in requests {
            // On failure the watches already opened are dropped with
            // `streams`, which cancels them.
            let stream = self.watch(request).map_err(|e| named(&name, e))?;
            streams.push((name, stream));
        }
        Ok(MultiNotificationStream::new(streams, policy))
    }

    /// Checks a watch request the way [`Self::watch`] and
    /// [`Self::watch_many`] do before opening it, without opening anything.
    /// Callers that build requests from their own input use it to report a
    /// mistake in their terms before any watch starts.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the request cannot be sent, for
    /// example a start position past the last possible sequence.
    pub fn check_watch_request(&self, request: &WatchRequest) -> crate::Result<()> {
        WireWatchRequest::from_public(request)?;
        compute_resume_key(&self.base_url, request)?;
        Ok(())
    }
}

/// Puts the watch's name into a configuration error's message.
fn named(name: &str, error: ClientError) -> ClientError {
    match error {
        ClientError::Config(message) => ClientError::Config(format!("watch '{name}': {message}")),
        other => other,
    }
}
