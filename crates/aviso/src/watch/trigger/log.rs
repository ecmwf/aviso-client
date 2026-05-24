//! Log trigger dispatch.

use std::path::Path;

use crate::Notification;

use super::{TriggerError, TriggerState};

/// Log dispatch: lazy-open the file on first call, then buffer-then-write
/// the NDJSON line. No fsync per the at-least-once contract (a crashed
/// pre-commit log write replays on restart).
pub(super) async fn dispatch_log(
    path: &Path,
    state: &mut TriggerState,
    notification: &Notification,
) -> Result<(), TriggerError> {
    use tokio::io::AsyncWriteExt as _;
    if state.log_handle.is_none() {
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .await?;
        state.log_handle = Some(file);
    }
    let Some(handle) = state.log_handle.as_mut() else {
        return Err(TriggerError::Io(std::io::Error::other(
            "log handle missing immediately after init; bug in lazy-open invariant",
        )));
    };
    let mut buf = serde_json::to_vec(notification)?;
    buf.push(b'\n');
    handle.write_all(&buf).await?;
    // tokio::fs::File buffers writes internally; flush so the kernel
    // sees the bytes immediately. Tailers and downstream NDJSON readers
    // benefit from prompt visibility, and the test suite relies on this
    // to read the file back synchronously. This is NOT an fsync; OS
    // pagecache durability is still asynchronous (the at-least-once
    // invariant covers crash replay, so no fsync per Q8).
    handle.flush().await?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: unwrap/expect on temporary directory and panic on unexpected errors keep assertions direct"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::dispatch_log;
    use crate::Notification;
    use crate::watch::trigger::{Trigger, TriggerError, TriggerState};

    fn make_notification() -> Notification {
        Notification {
            event_type: "mars".to_string(),
            sequence: 1,
            identifier: BTreeMap::new(),
            payload: serde_json::Value::Null,
            cloudevent: None,
        }
    }

    #[tokio::test]
    async fn log_trigger_writes_ndjson_to_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notif.log");
        let mut state = TriggerState::new();
        let result = dispatch_log(&path, &mut state, &make_notification()).await;
        assert!(matches!(result, Ok(())));
        drop(state);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with('{'), "got: {contents}");
        assert!(
            contents.contains("\"event_type\":\"mars\""),
            "got: {contents}"
        );
        assert!(contents.ends_with('\n'), "got: {contents}");
    }

    #[tokio::test]
    async fn log_trigger_returns_io_error_when_parent_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("missing").join("x.log");
        let mut state = TriggerState::new();
        let result = dispatch_log(&bad_path, &mut state, &make_notification()).await;
        match result {
            Err(TriggerError::Io(_)) => {}
            Ok(()) => panic!("expected Io error, got Ok"),
            Err(other) => panic!("expected Io error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn log_trigger_lazy_opens_then_reuses_handle_across_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reuse.log");
        let trigger = Trigger::log(&path);
        let mut state = TriggerState::new();
        assert!(state.log_handle.is_none());
        let Trigger { kind, .. } = trigger;
        let crate::watch::trigger::kind::TriggerKind::Log { path } = kind else {
            unreachable!("Trigger::log must construct a Log kind");
        };
        dispatch_log(&path, &mut state, &make_notification())
            .await
            .expect("first dispatch_log must succeed");
        assert!(state.log_handle.is_some(), "handle should be open");
        let handle_ptr_before: *const _ = state.log_handle.as_ref().unwrap();
        dispatch_log(&path, &mut state, &make_notification())
            .await
            .expect("second dispatch_log must succeed");
        let handle_ptr_after: *const _ = state.log_handle.as_ref().unwrap();
        assert!(std::ptr::eq(handle_ptr_before, handle_ptr_after));
    }
}
