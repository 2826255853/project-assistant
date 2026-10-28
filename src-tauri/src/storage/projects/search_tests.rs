use super::*;
use crate::ipc::correlation_id;
use crate::project_contract::ListRequest;
use std::fs;

fn request(query: &str, limit: u16, cursor: Option<String>) -> ListRequest {
    ListRequest {
        query: Some(query.into()),
        limit: Some(limit),
        cursor,
    }
}

#[test]
fn matching_examples_are_unicode_case_separator_and_relevance_aware() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    search_fixture::seed(&mut db.connection).unwrap();
    let mut search = |q: &str| db.projects().list(&request(q, 100, None)).unwrap().items;
    let matches = search("项目助手");
    assert_eq!(
        matches.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["项目助手", "工具项目助手扩展", "项目桌面助手"]
    );
    assert_eq!(
        search("中文工作台")[0].display_name.as_deref(),
        Some("中文工作台")
    );
    assert_eq!(search("sYnThEtIc-0099")[0].name, "Synthetic-0099");
    assert_eq!(
        search("C:/synthetic-private-path/SYNTHETIC-0099")[0].name,
        "Synthetic-0099"
    );
    assert_eq!(search("synth0099")[0].name, "Synthetic-0099");
    for q in ["助手项目", "不存在", "%_*", "   ", "' OR 1=1 --"] {
        assert!(search(q).is_empty());
    }
    assert_eq!(tests::count(&db, "audit_events"), 0);
    assert_eq!(tests::count(&db, "project_usage"), 334);
    f.unchanged();
}

#[test]
fn stable_recent_and_name_paging_visit_every_record_once() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    search_fixture::seed(&mut db.connection).unwrap();
    for sort in ["recent", "name"] {
        db.connection.execute("INSERT INTO settings(id,scope,key,value_text,revision,updated_at) VALUES('sort','global','project_sort',?1,1,1) ON CONFLICT(key) WHERE scope='global' DO UPDATE SET value_text=excluded.value_text", [sort]).unwrap();
        let mut views = Vec::new();
        let mut cursor = None;
        loop {
            let req = request("", 37, cursor);
            let page = db.projects().list(&req).unwrap();
            assert_eq!(page, db.projects().list(&req).unwrap());
            assert!(page.items.len() <= 37);
            views.extend(page.items);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(views.len(), 1000);
        assert_eq!(
            views
                .iter()
                .map(|v| &v.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            1000
        );
        for pair in views.windows(2) {
            if sort == "recent" {
                assert!(pair[0].last_used_at >= pair[1].last_used_at);
            }
            if sort == "name" || pair[0].last_used_at == pair[1].last_used_at {
                let a = pair[0]
                    .display_name
                    .as_ref()
                    .unwrap_or(&pair[0].name)
                    .to_lowercase();
                let b = pair[1]
                    .display_name
                    .as_ref()
                    .unwrap_or(&pair[1].name)
                    .to_lowercase();
                assert!((a, &pair[0].id) <= (b, &pair[1].id));
            }
        }
    }
}

#[test]
fn cursors_reject_changes_tampering_and_cross_query_use() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    search_fixture::seed(&mut db.connection).unwrap();
    let token = db
        .projects()
        .list(&request("", 1, None))
        .unwrap()
        .next_cursor
        .unwrap();
    assert_eq!(
        db.projects()
            .list(&request("synthetic", 1, Some(token.clone()))),
        Err(ErrorCode::ProjectConflictRevision)
    );
    assert_eq!(
        db.projects()
            .list(&request("", 1, Some("bad-cursor".into()))),
        Err(ErrorCode::InvalidRequest)
    );
    let changed = format!("v1.{}.{}", "0".repeat(64), token.split('.').nth(2).unwrap());
    assert_eq!(
        db.projects().list(&request("", 1, Some(changed))),
        Err(ErrorCode::ProjectConflictRevision)
    );
    db.connection
        .execute(
            "UPDATE projects SET display_name='renamed' WHERE name='Synthetic-0999'",
            [],
        )
        .unwrap();
    assert_eq!(
        db.projects().list(&request("", 1, Some(token))),
        Err(ErrorCode::ProjectConflictRevision)
    );
}

#[test]
fn stored_availability_visible_tombstones_hidden_and_read_has_no_writes() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    fs::rename(f.source(), f.root.join("moved")).unwrap();
    // Search uses the last authoritative observation; only detail/startup revalidate paths.
    assert_eq!(
        db.projects().list(&request("", 1, None)).unwrap().items[0].availability,
        Availability::Available
    );
    db.projects().get(&p.id, &correlation_id()).unwrap();
    let audit = tests::count(&db, "audit_events");
    assert_eq!(
        db.projects().list(&request("", 1, None)).unwrap().items[0].availability,
        Availability::Missing
    );
    assert_eq!(tests::count(&db, "audit_events"), audit);
    assert_eq!(tests::count(&db, "project_usage"), 0);
    db.projects().remove(&p.id, 2, &correlation_id()).unwrap();
    assert!(
        db.projects()
            .list(&request("", 1, None))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn request_limits_default_page_and_null_types_are_strict() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    search_fixture::seed(&mut db.connection).unwrap();
    let default: ListRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(db.projects().list(&default).unwrap().items.len(), 50);
    for raw in [
        r#"{"query":null}"#,
        r#"{"limit":null}"#,
        r#"{"cursor":null}"#,
        r#"{"sort":"name"}"#,
        r#"{"limit":1.5}"#,
    ] {
        assert!(serde_json::from_str::<ListRequest>(raw).is_err());
    }
    assert_eq!(
        db.projects().list(&request("", 0, None)),
        Err(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        db.projects().list(&request("", 101, None)),
        Err(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        db.projects().list(&request(&"中".repeat(86), 1, None)),
        Err(ErrorCode::InputTooLarge)
    );
    assert_eq!(
        db.projects().list(&request("bad\nquery", 1, None)),
        Err(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        db.projects().list(&request("", 1, Some("x".repeat(513)))),
        Err(ErrorCode::InputTooLarge)
    );
}

#[test]
fn equal_names_have_uuid_order_and_mutations_invalidate_pagination() {
    let f = tests::Fixture::new();
    let mut db = f.db();
    search_fixture::seed(&mut db.connection).unwrap();
    db.connection
        .execute(
            "UPDATE projects SET display_name='same' WHERE name LIKE 'Synthetic-%'",
            [],
        )
        .unwrap();
    let views = db
        .projects()
        .list(&request("same", 100, None))
        .unwrap()
        .items;
    for pair in views.windows(2) {
        if pair[0].last_used_at == pair[1].last_used_at {
            assert!(pair[0].id < pair[1].id);
        }
    }
    for mutation in [
        "UPDATE project_usage SET last_opened_at=last_opened_at+1",
        "UPDATE projects SET availability='inaccessible' WHERE name='Synthetic-0999'",
        "UPDATE projects SET authorization='revoked',removed_at=2 WHERE name='Synthetic-0999'",
    ] {
        let token = db
            .projects()
            .list(&request("", 1, None))
            .unwrap()
            .next_cursor;
        db.connection.execute(mutation, []).unwrap();
        assert_eq!(
            db.projects().list(&request("", 1, token)),
            Err(ErrorCode::ProjectConflictRevision)
        );
    }
    db.connection
        .execute_batch("ALTER TABLE project_usage RENAME TO unavailable_usage")
        .unwrap();
    assert_eq!(
        db.projects().list(&request("", 1, None)),
        Err(ErrorCode::SettingsStorageFailed)
    );
}
