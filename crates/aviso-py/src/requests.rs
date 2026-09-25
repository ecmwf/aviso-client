// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Turning `listen()` arguments into a watch request.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::triggers::PyTrigger;
use crate::values::validate_identifier;
use crate::watch::{PyWatchRequest, parse_resume_start};

pub(crate) fn build_watch_request(
    event_type: Option<String>,
    filter: Option<&Bound<'_, PyDict>>,
    start_from: Option<&Bound<'_, PyAny>>,
    mode: Option<&str>,
    triggers: Option<&Bound<'_, PyAny>>,
    request: Option<PyRef<'_, PyWatchRequest>>,
) -> PyResult<aviso::watch::WatchRequest> {
    if let Some(req) = request {
        if event_type.is_some() || filter.is_some() || start_from.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "request is mutually exclusive with event_type, filter, and start_from",
            ));
        }
        if mode.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "request already carries a mode; do not pass mode= when using request=",
            ));
        }
        if triggers.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "triggers= cannot be combined with request=; add triggers to the WatchRequest instead",
            ));
        }
        return Ok(req.clone().into_inner());
    }
    let event = event_type.ok_or_else(|| {
        crate::error::AvisoError::new_err(
            "listen() requires either event_type=<str> or request=<WatchRequest>",
        )
    })?;
    let effective_mode = mode.unwrap_or("watch");
    let mut req = match (effective_mode, start_from) {
        ("watch", None) => aviso::watch::WatchRequest::watch(event),
        ("watch", Some(from_obj)) => {
            let resume = parse_resume_start(from_obj)?;
            aviso::watch::WatchRequest::watch_from(event, resume)
        }
        ("replay_only", Some(from_obj)) => {
            let resume = parse_resume_start(from_obj)?;
            aviso::watch::WatchRequest::replay_only(event, resume)
        }
        ("replay_only", None) => {
            return Err(crate::error::AvisoError::new_err(
                "replay_only mode requires start_from=<int sequence or date string>",
            ));
        }
        _ => {
            return Err(crate::error::AvisoError::new_err(format!(
                "unknown WatchMode {effective_mode:?}; expected 'watch' or 'replay_only'"
            )));
        }
    };
    if let Some(filter_dict) = filter {
        validate_identifier(filter_dict.as_any())?;
        let mut map = std::collections::BTreeMap::<String, serde_json::Value>::new();
        for (k, v) in filter_dict {
            let key: String = k.extract()?;
            let value: serde_json::Value = pythonize::depythonize(&v)?;
            map.insert(key, value);
        }
        req = req.with_filter(map);
    }
    if let Some(triggers_value) = triggers {
        let trigger_vec = extract_triggers(triggers_value)?;
        if !trigger_vec.is_empty() {
            req = req.with_triggers(trigger_vec);
        }
    }
    Ok(req)
}

fn extract_triggers(value: &Bound<'_, PyAny>) -> PyResult<Vec<aviso::watch::Trigger>> {
    if value.is_instance_of::<PyTrigger>() {
        return Err(crate::error::AvisoError::new_err(
            "triggers= must be a sequence of Trigger; pass [trigger] for a single trigger",
        ));
    }
    if value.is_instance_of::<pyo3::types::PyString>()
        || value.is_instance_of::<pyo3::types::PyBytes>()
    {
        return Err(crate::error::AvisoError::new_err(
            "triggers= must be a sequence of Trigger, not a string or bytes",
        ));
    }
    let iter = value.try_iter().map_err(|_| {
        crate::error::AvisoError::new_err(
            "triggers= must be an iterable of Trigger instances (list or tuple)",
        )
    })?;
    let mut out = Vec::new();
    for item in iter {
        let item = item?;
        let trigger: PyRef<'_, PyTrigger> = item.extract().map_err(|_| {
            crate::error::AvisoError::new_err(
                "triggers= entries must be Trigger instances; got something else",
            )
        })?;
        out.push(trigger.clone().into_inner());
    }
    Ok(out)
}
