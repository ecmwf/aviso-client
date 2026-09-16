// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Repeated identifiers and the shared inline listener arguments.

use std::collections::BTreeMap;

use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::{config::ListenerSpec, exit::usage_error, listener};

#[derive(Debug, Args)]
#[group(id = "identifier_source", required = false, multiple = false)]
struct IdentifierSource {
    /// Identifiers filter as a JSON object. Requires --event.
    #[arg(long, value_name = "JSON", requires = "event")]
    identifiers: Option<String>,

    /// One identifier: key=value (exact string) or key:=JSON (typed). Repeatable.
    /// Requires --event. Mutually exclusive with --identifiers.
    #[arg(long, value_name = "KEY=VALUE", action = clap::ArgAction::Append, requires = "event")]
    identifier: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct InlineListenerArgs {
    /// Inline event type. Requires --identifiers or repeated --identifier.
    /// Overrides YAML listeners and runs a default echo trigger.
    #[arg(long, value_name = "TYPE", requires = "identifier_source")]
    event: Option<String>,

    #[command(flatten)]
    source: IdentifierSource,
}

impl InlineListenerArgs {
    pub(crate) fn resolve(&self) -> Result<Option<ListenerSpec>> {
        let Some(event) = self.event.as_deref() else {
            return Ok(None);
        };
        if let Some(json) = self.source.identifiers.as_deref() {
            return listener::build_inline_listener_spec(event, json).map(Some);
        }
        let mut identifiers = BTreeMap::new();
        append(&mut identifiers, &self.source.identifier, false)?;
        Ok(Some(listener::inline_listener_spec(event, identifiers)))
    }
}

/// Append whole argv values without interpreting string contents.
/// `label=a,b:=c` is a valid string; `label:=oops` is invalid JSON.
pub(crate) fn append(
    identifiers: &mut BTreeMap<String, Value>,
    entries: &[String],
    notify: bool,
) -> Result<()> {
    for entry in entries {
        let (left, value) = entry
            .split_once('=')
            .ok_or_else(|| usage_error("--identifier requires key=value or key:=JSON"))?;
        let (key, typed) = match left.strip_suffix(':') {
            Some(key) => (key.trim(), true),
            None => (left.trim(), false),
        };
        if key.is_empty() {
            return Err(usage_error("--identifier key must not be empty"));
        }
        if notify && matches!(key, "event" | "data") {
            return Err(usage_error(format!(
                "--identifier {key} is reserved; use {key}= in the positional parameters"
            )));
        }
        if identifiers.contains_key(key) {
            return Err(usage_error(format!("duplicate identifier key `{key}`")));
        }
        let value = if typed {
            serde_json::from_str(value)
                .map_err(|e| usage_error(format!("parse --identifier `{key}` as JSON: {e}")))?
        } else {
            Value::String(value.to_owned())
        };
        identifiers.insert(key.to_owned(), value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(entries: &[&str]) -> Result<BTreeMap<String, Value>> {
        let mut map = BTreeMap::new();
        append(
            &mut map,
            &entries.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            false,
        )?;
        Ok(map)
    }

    #[test]
    fn strings_are_exact_after_first_equals() -> Result<()> {
        for value in [
            "",
            " a,b ",
            "\"quoted\"",
            "'quoted'",
            "a=b:=c",
            "[]",
            "{}",
            "12",
            "true",
            "null",
            r"\path/$HOME/$(false)",
        ] {
            assert_eq!(
                parse(&[&format!(" label ={value}")])?["label"],
                json!(value)
            );
        }
        assert_eq!(parse(&["a:b=x"])?["a:b"], json!("x"));
        Ok(())
    }

    #[test]
    fn explicit_json_supports_all_types() -> Result<()> {
        let map = parse(&[
            "int:=12",
            "float:=1.25",
            "bool:=true",
            "null:=null",
            "array:=[1,\"a\"]",
            "object:={\"gte\":5}",
            "string:=\"a=b:=c\"",
        ])?;
        assert_eq!(
            serde_json::to_value(map)?,
            json!({"int":12,"float":1.25,"bool":true,"null":null,"array":[1,"a"],"object":{"gte":5},"string":"a=b:=c"})
        );
        Ok(())
    }

    #[test]
    fn invalid_entries_and_duplicate_keys_are_usage_errors() {
        for entries in [
            vec!["missing"],
            vec!["=x"],
            vec![" :=1"],
            vec!["x:="],
            vec!["x:=NaN"],
            vec!["x:=01"],
            vec!["x:=true trailing"],
            vec!["x=a", " x :=1"],
            vec!["x:=1", "x=b"],
        ] {
            let result = parse(&entries);
            assert!(result.is_err(), "{entries:?}");
            if let Err(error) = result {
                assert_eq!(crate::exit::exit_code_for_anyhow(&error), 2);
            }
        }
    }
}
