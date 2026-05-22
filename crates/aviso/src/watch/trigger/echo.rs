//! Echo trigger dispatch.

use crate::Notification;

use super::TriggerError;

/// Echo dispatch: serialise the notification into a buffer ONCE (appending
/// the newline to the same buffer), then a single `write_all` against a
/// locked stdout handle. Buffer-then-write avoids any intra-trigger seam
/// between the JSON body and the line terminator.
pub(super) fn dispatch_echo(notification: &Notification) -> Result<(), TriggerError> {
    use std::io::Write as _;
    let mut buf = serde_json::to_vec(notification)?;
    buf.push(b'\n');
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(&buf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::dispatch_echo;
    use crate::Notification;

    fn make_notification() -> Notification {
        Notification {
            event_type: "mars".to_string(),
            sequence: 1,
            identifier: BTreeMap::new(),
            payload: serde_json::Value::Null,
        }
    }

    #[test]
    fn echo_trigger_succeeds_without_retry() {
        let result = dispatch_echo(&make_notification());
        assert!(matches!(result, Ok(())));
    }
}
