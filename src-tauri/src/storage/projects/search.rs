//! Metadata-only search. A read transaction binds settings and cursor to one
//! database snapshot. Never visits project directories or starts child processes.
use super::*;
use crate::project_contract::{ListRequest, ProjectListPage};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;

fn normalize(text: &str) -> String {
    text.replace('/', "\\").to_lowercase()
}

// Exact field, substring, then ordered Unicode scalar subsequence. No pinyin,
// token splitting or wildcard interpretation; whitespace is literal.
fn rank(view: &ProjectView, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    [
        &view.name,
        view.display_name.as_ref().unwrap_or(&view.name),
        &view.absolute_path,
    ]
    .into_iter()
    .filter_map(|field| {
        let field = normalize(field);
        if field == query {
            return Some(0);
        }
        if field.contains(query) {
            return Some(1);
        }
        let mut chars = field.chars();
        query
            .chars()
            .all(|ch| chars.by_ref().any(|candidate| candidate == ch))
            .then_some(2)
    })
    .min()
}

type Key = (u8, Reverse<Option<u64>>, String, String);
fn key(view: &ProjectView, query: &str, recent: bool) -> Option<Key> {
    Some((
        rank(view, query)?,
        Reverse(if recent { view.last_used_at } else { None }),
        normalize(view.display_name.as_deref().unwrap_or(&view.name)),
        view.id.clone(),
    ))
}

fn cursor(value: &str) -> Result<(&str, &str), ErrorCode> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts[0] != "v1"
        || parts[1].len() != 64
        || !parts[1]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || (crate::project_contract::ProjectIdRequest {
            project_id: parts[2].into(),
        })
        .validate()
        .is_err()
    {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok((parts[1], parts[2]))
}

impl ProjectRepository<'_> {
    pub fn list(&mut self, request: &ListRequest) -> Result<ProjectListPage, ErrorCode> {
        request.validate()?;
        let query = normalize(request.query.as_deref().unwrap_or(""));
        let after = request.cursor.as_deref().map(cursor).transpose()?;
        let limit = usize::from(request.limit.unwrap_or(50));
        let tx = self.connection.transaction().map_err(sql)?;
        let setting: Option<(Option<String>, Option<i64>)> = tx.query_row(
            "SELECT value_text,value_integer FROM settings WHERE scope='global' AND key='project_sort'",
            [], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
        let recent = match setting {
            None => true,
            Some((Some(value), None)) if value == "recent" => true,
            Some((Some(value), None)) if value == "name" => false,
            _ => return Err(ErrorCode::SettingsCorrupt),
        };
        let after_key = after
            .map(|(_, id)| {
                let view = record(&tx, id)
                    .map_err(|error| {
                        if error == ErrorCode::ProjectNotFound {
                            ErrorCode::ProjectConflictRevision
                        } else {
                            error
                        }
                    })?
                    .view;
                key(&view, &query, recent).ok_or(ErrorCode::ProjectConflictRevision)
            })
            .transpose()?;
        let mut digest = Sha256::new();
        digest.update(
            serde_json::to_vec(&(&query, recent)).map_err(|_| ErrorCode::InternalUnexpected)?,
        );
        // Stream all registered metadata, retaining at most limit+1 matches.
        // Memory is bounded independently of the number of registered projects.
        let mut best: Vec<(Key, ProjectView)> = Vec::with_capacity(limit + 2);
        {
            let mut statement = tx.prepare(&format!("SELECT {COLUMNS} FROM projects p LEFT JOIN project_usage u ON u.project_id=p.id WHERE p.removed_at IS NULL AND p.authorization!='revoked' ORDER BY p.id")).map_err(sql)?;
            let mut rows = statement.query([]).map_err(sql)?;
            while let Some(row) = rows.next().map_err(sql)? {
                let view = map_record(row).map_err(sql)?.view;
                view.validate_wire()?;
                digest
                    .update(serde_json::to_vec(&view).map_err(|_| ErrorCode::InternalUnexpected)?);
                if let Some(key) = key(&view, &query, recent) {
                    if after_key.as_ref().is_some_and(|last| &key <= last) {
                        continue;
                    }
                    let index = best
                        .binary_search_by(|(candidate, _)| candidate.cmp(&key))
                        .unwrap_or_else(|i| i);
                    if index <= limit {
                        best.insert(index, (key, view));
                        best.truncate(limit + 1);
                    }
                }
            }
        }
        let fingerprint = format!("{:x}", digest.finalize());
        if after.is_some_and(|(expected, _)| expected != fingerprint) {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let more = best.len() > limit;
        best.truncate(limit);
        let next_cursor = if more {
            best.last()
                .map(|(_, view)| format!("v1.{fingerprint}.{}", view.id))
        } else {
            None
        };
        let page = ProjectListPage {
            items: best.into_iter().map(|(_, view)| view).collect(),
            next_cursor,
        };
        page.validate_wire()?;
        tx.commit().map_err(sql)?;
        Ok(page)
    }
}
