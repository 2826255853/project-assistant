//! Main-only, bounded in-memory task result reads. No transcript persistence.
use super::*;
use serde::Serialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContentRequest {
    pub task_id: String,
    pub channel: String,
    pub cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChangesRequest {
    pub task_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiffRequest {
    pub task_id: String,
    pub change_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Content {
    task_id: String,
    content_revision: i64,
    channel: String,
    text: String,
    next_cursor: Option<String>,
    truncated: bool,
    availability: &'static str,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Change {
    id: String,
    kind: String,
    relative_path: String,
    diff_available: bool,
}
#[derive(Clone)]
pub(super) struct StoredChange {
    view: Change,
    diff: String,
    identity: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Changes {
    task_id: String,
    items: Vec<Change>,
    next_cursor: Option<String>,
    content_available: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Diff {
    task_id: String,
    change_id: String,
    text: Option<String>,
    unavailable_reason: Option<&'static str>,
}

// Cursors bind task, channel and exact cached bytes. A live append requires a
// fresh snapshot; a cursor can never splice unrelated task/channel content.
fn page(
    text: &str,
    binding: &str,
    cursor: Option<&str>,
    size: usize,
) -> Result<(String, Option<String>), ErrorCode> {
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{binding}\0{text}").as_bytes())
    );
    let offset = match cursor {
        None => 0,
        Some(cursor) => {
            let (digest, offset) = cursor.split_once(':').ok_or(ErrorCode::InvalidRequest)?;
            if digest != hash {
                return Err(ErrorCode::TaskConflictRevision);
            }
            offset
                .parse::<usize>()
                .map_err(|_| ErrorCode::InvalidRequest)?
        }
    };
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut end = offset.saturating_add(size).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok((
        text[offset..end].into(),
        (end < text.len()).then(|| format!("{hash}:{end}")),
    ))
}

pub(crate) fn content(
    app: &tauri::AppHandle,
    request: ContentRequest,
) -> Result<Content, ErrorCode> {
    if !matches!(request.channel.as_str(), "reply" | "progress")
        || request.cursor.as_ref().is_some_and(|s| s.len() > 512)
    {
        return Err(ErrorCode::InvalidRequest);
    }
    let task = get(
        app,
        TaskGetRequest {
            task_id: request.task_id.clone(),
        },
    )?;
    let runtime = app.state::<TaskRuntimeState>();
    let outputs = runtime
        .outputs
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    let output = outputs.get(&request.task_id);
    let text = output
        .map(|o| {
            if request.channel == "reply" {
                &o.reply
            } else {
                &o.progress
            }
        })
        .map(String::as_str)
        .unwrap_or("");
    let (text, next_cursor) = page(
        text,
        &format!("{}:{}", request.task_id, request.channel),
        request.cursor.as_deref(),
        32 * 1024,
    )?;
    Ok(Content {
        task_id: request.task_id,
        content_revision: task.content_revision,
        channel: request.channel,
        text,
        next_cursor,
        truncated: false,
        availability: if output.is_some() {
            "memory"
        } else {
            "unavailable"
        },
    })
}

pub(crate) fn capture(
    app: &tauri::AppHandle,
    task_id: &str,
    changes: Vec<crate::owned_process::FileChange>,
) -> Result<(), ErrorCode> {
    #[cfg(windows)]
    {
        let verified = app
            .state::<StorageState>()
            .with_storage(|s| Ok(s.tasks().task_result_directory(task_id)))
            .map_err(map_storage)??;
        let mut captured = Vec::new();
        for change in changes {
            let identity = crate::project_paths::delivery::task_change_identity(
                &verified,
                &change.relative_path,
                change.kind == "deleted",
            )?;
            captured.push(StoredChange {
                view: Change {
                    id: crate::providers::uuid()?,
                    kind: change.kind,
                    relative_path: change.relative_path,
                    diff_available: true,
                },
                diff: change.diff,
                identity,
            });
        }
        let runtime = app.state::<TaskRuntimeState>();
        let mut outputs = runtime
            .outputs
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?;
        let output = outputs.entry(task_id.into()).or_default();
        // Replace an earlier version of the same task-local file atomically.
        let mut merged = output.changes.clone();
        for mut change in captured {
            if let Some(index) = merged
                .iter()
                .position(|c| c.view.relative_path == change.view.relative_path)
            {
                change.view.id = merged[index].view.id.clone();
                merged[index] = change;
            } else {
                merged.push(change);
            }
        }
        if merged.len() > 100
            || merged.iter().map(|c| c.diff.len()).sum::<usize>() > 2 * 1024 * 1024
        {
            return Err(ErrorCode::InputTooLarge);
        }
        output.changes = merged;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, task_id, changes);
        Err(ErrorCode::ProjectPathUnavailable)
    }
}

pub(crate) fn changes(
    app: &tauri::AppHandle,
    request: ChangesRequest,
) -> Result<Changes, ErrorCode> {
    let _task = get(
        app,
        TaskGetRequest {
            task_id: request.task_id.clone(),
        },
    )?;
    let limit = request.limit.unwrap_or(50) as usize;
    if !(1..=100).contains(&limit) || request.cursor.as_ref().is_some_and(|s| s.len() > 512) {
        return Err(ErrorCode::InvalidRequest);
    }
    let runtime = app.state::<TaskRuntimeState>();
    let outputs = runtime
        .outputs
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    let output = outputs.get(&request.task_id);
    let available = output.is_some();
    let mut stored = output.map(|o| o.changes.clone()).unwrap_or_default();
    drop(outputs);
    // Recheck locators before returning names, and again for the diff read.
    #[cfg(windows)]
    {
        let verified = app
            .state::<StorageState>()
            .with_storage(|s| Ok(s.tasks().task_result_directory(&request.task_id)))
            .map_err(map_storage)??;
        for change in &mut stored {
            change.view.diff_available = crate::project_paths::delivery::task_change_identity(
                &verified,
                &change.view.relative_path,
                change.view.kind == "deleted",
            )
            .is_ok_and(|id| id == change.identity);
        }
    }
    let binding = stored
        .iter()
        .map(|c| format!("{}:{}", c.view.id, c.identity))
        .collect::<Vec<_>>()
        .join("\n");
    // Page the stable ID list with the same binding checks as content.
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{binding}", request.task_id))
    );
    let offset = if let Some(cursor) = request.cursor {
        let (digest, offset) = cursor.split_once(':').ok_or(ErrorCode::InvalidRequest)?;
        if digest != hash {
            return Err(ErrorCode::TaskConflictRevision);
        }
        offset
            .parse::<usize>()
            .map_err(|_| ErrorCode::InvalidRequest)?
    } else {
        0
    };
    if offset > stored.len() {
        return Err(ErrorCode::InvalidRequest);
    }
    let end = offset.saturating_add(limit).min(stored.len());
    Ok(Changes {
        task_id: request.task_id,
        items: stored[offset..end].iter().map(|c| c.view.clone()).collect(),
        next_cursor: (end < stored.len()).then(|| format!("{hash}:{end}")),
        content_available: available,
    })
}

pub(crate) fn diff(app: &tauri::AppHandle, request: DiffRequest) -> Result<Diff, ErrorCode> {
    let _task = get(
        app,
        TaskGetRequest {
            task_id: request.task_id.clone(),
        },
    )?;
    let runtime = app.state::<TaskRuntimeState>();
    let outputs = runtime
        .outputs
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    let change = outputs
        .get(&request.task_id)
        .and_then(|o| o.changes.iter().find(|c| c.view.id == request.change_id))
        .cloned();
    let expired = !outputs.contains_key(&request.task_id);
    drop(outputs);
    if change.is_none() && !expired {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut result = Diff {
        task_id: request.task_id.clone(),
        change_id: request.change_id,
        text: None,
        unavailable_reason: Some("expired"),
    };
    if let Some(change) = change {
        #[cfg(windows)]
        {
            let verified = app
                .state::<StorageState>()
                .with_storage(|s| Ok(s.tasks().task_result_directory(&request.task_id)))
                .map_err(map_storage)??;
            if crate::project_paths::delivery::task_change_identity(
                &verified,
                &change.view.relative_path,
                change.view.kind == "deleted",
            )
            .is_ok_and(|id| id == change.identity)
            {
                result.text = Some(change.diff);
                result.unavailable_reason = None;
            } else {
                result.unavailable_reason = Some("unsafe_path");
            }
        }
    }
    Ok(result)
}

#[cfg(all(test, windows))]
pub(crate) fn desktop_fixture(app: &tauri::AppHandle, action: &str) -> Result<(), ErrorCode> {
    let task_id = app
        .state::<StorageState>()
        .with_storage(|s| {
            s.connection
                .query_row(
                    "SELECT id FROM tasks ORDER BY created_at DESC,id DESC LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| StorageError {
                    code: "STORAGE_SQL_FAILED",
                    backup_path: None,
                })
        })
        .map_err(map_storage)?;
    match action {
        "task_ui_recover" => {
            app.state::<StorageState>()
                .with_storage(|s| {
                    s.recover_on_startup(&crate::ipc::correlation_id())
                        .map(|_| ())
                })
                .map_err(map_storage)?;
        }
        "task_ui_gap" => {
            app.state::<StorageState>()
                .with_storage(|s| {
                    s.connection
                        .execute("DELETE FROM task_events WHERE task_id=?1", [&task_id])
                        .map(|_| ())
                        .map_err(|_| StorageError {
                            code: "STORAGE_SQL_FAILED",
                            backup_path: None,
                        })
                })
                .map_err(map_storage)?;
        }
        "task_ui_evict" => {
            app.state::<TaskRuntimeState>()
                .outputs
                .lock()
                .map_err(|_| ErrorCode::ProcessExitFailed)?
                .remove(&task_id);
        }
        "task_ui_change" | "task_ui_escape" => {
            let path = if action == "task_ui_escape" {
                "../outside.txt"
            } else {
                "selected-through-ui.txt"
            };
            capture(
                app,
                &task_id,
                vec![crate::owned_process::FileChange {
                    relative_path: path.into(),
                    kind: "modified".into(),
                    diff: "@@ -1 +1 @@\n-old\n+S3_DIFF_SENTINEL".into(),
                }],
            )?;
        }
        _ => return Err(ErrorCode::InvalidRequest),
    }
    app_state::publish(app, crate::ipc::correlation_id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_union_serializes_the_frontend_field_names() {
        assert_eq!(
            serde_json::to_value(crate::storage::tasks::EventData::ContentChanged {
                channel: "reply".into(),
                content_revision: 2
            })
            .unwrap(),
            serde_json::json!({"type":"content_changed","channel":"reply","contentRevision":2})
        );
        assert_eq!(
            serde_json::to_value(crate::storage::tasks::EventData::Approval {
                approval_id: "id".into()
            })
            .unwrap(),
            serde_json::json!({"type":"approval","approvalId":"id"})
        );
    }
    #[test]
    fn content_pages_bind_task_channel_and_bytes_and_preserve_utf8() {
        let text = "中文🙂".repeat(10000);
        let mut collected = String::new();
        let mut cursor = None;
        loop {
            let (part, next) = page(&text, "task:reply", cursor.as_deref(), 32768).unwrap();
            assert!(part.len() <= 32768);
            collected.push_str(&part);
            if next.is_none() {
                break;
            }
            cursor = next;
        }
        assert_eq!(collected, text);
        let (_, cursor) = page(&text, "task:reply", None, 32768).unwrap();
        for (text, binding) in [
            (&text[..], "other:reply"),
            (&text[..], "task:progress"),
            ("changed", "task:reply"),
        ] {
            assert!(page(text, binding, cursor.as_deref(), 32768).is_err());
        }
    }
}
