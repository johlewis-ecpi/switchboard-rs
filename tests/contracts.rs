use std::fs;
use switchboard::{
    model::*,
    query::{self, WindowState},
    rules, services,
    store::{Backup, Store},
    transfer::{self, Portable},
};
use uuid::Uuid;

#[test]
fn opening_activity_persists_and_ranks_recent_and_frequent_independently() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.json");
    let a = entry("A");
    let b = entry("B");
    let unopened = entry("Never opened");
    let mut store = Store::load(path.clone());
    store.record_open(a.id).unwrap();
    store.record_open(a.id).unwrap();
    store.record_open(b.id).unwrap();
    let restored = Store::load(path);
    assert_eq!(restored.activity[&a.id].count, 2);
    assert_eq!(restored.activity[&b.id].count, 1);
    assert!(!directory.path().join("library.json").exists());
    let mut activity = restored.activity;
    activity.get_mut(&a.id).unwrap().last_opened = 10.;
    activity.get_mut(&b.id).unwrap().last_opened = 20.;
    let mut candidates = vec![&unopened, &a, &b];
    query::rank_activity(&mut candidates, "recent", &activity);
    assert_eq!(
        candidates.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![b.id, a.id]
    );
    query::rank_activity(&mut candidates, "frequent", &activity);
    assert_eq!(
        candidates.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![a.id, b.id]
    );
}

#[test]
fn minimal_settings_support_old_preferences_and_validate_sizes() {
    let mut legacy = serde_json::to_value(WindowState::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("minimalIconSize");
    legacy.as_object_mut().unwrap().remove("minimalWrap");
    let mut state: WindowState = serde_json::from_value(legacy).unwrap();
    assert_eq!(state.minimal_icon_size, 80);
    assert!(state.minimal_wrap);
    state.minimal_icon_size = 999;
    state.scope = "recent".into();
    state.validate(&Library::default());
    assert_eq!(state.minimal_icon_size, 160);
    assert_eq!(state.scope, "recent");
    state.minimal_icon_size = 0;
    state.scope = "frequent".into();
    state.validate(&Library::default());
    assert_eq!(state.minimal_icon_size, 48);
    assert_eq!(state.scope, "frequent");
    let directory = tempfile::tempdir().unwrap();
    let store = Store::load(directory.path().join("library.json"));
    state.minimal_icon_size = 112;
    state.minimal_wrap = false;
    store.save_preferences(&state).unwrap();
    let restored = store.load_preferences();
    assert_eq!(restored.minimal_icon_size, 112);
    assert!(!restored.minimal_wrap);
}

#[test]
fn availability_distinguishes_local_missing_and_inaccessible_without_reading_contents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("document.docx");
    let mut e = entry("Document");
    e.location = path.to_string_lossy().into_owned();
    assert_eq!(services::availability(&e).status, "missing");
    fs::write(&path, "test document").unwrap();
    assert_eq!(services::availability(&e).status, "available");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(services::availability(&e).status, "inaccessible");
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    e.kind = Kind::Url;
    e.location = "https://example.invalid".into();
    assert_eq!(services::availability(&e).status, "web");
}
fn entry(name: &str) -> Entry {
    Entry::new(name.into(), format!("/tmp/{name}"), Kind::File)
}
fn collection(name: &str, parent: Option<Uuid>) -> Collection {
    Collection {
        id: Uuid::new_v4(),
        name: name.into(),
        parent_id: parent,
        icon: None,
        palette: None,
    }
}
fn tag(name: &str) -> Tag {
    Tag {
        id: Uuid::new_v4(),
        name: name.into(),
        icon: None,
        palette: None,
    }
}
fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::load(d.path().join("library.json"));
    (d, s)
}
fn rule(name: &str, actions: Vec<Action>) -> Rule {
    Rule {
        id: Uuid::new_v4(),
        name: name.into(),
        enabled: true,
        trigger: Trigger::AdditionsAndManual,
        predicate: Predicate::default(),
        name_contains: String::new(),
        location_contains: String::new(),
        url_host: String::new(),
        folder_path: String::new(),
        actions,
        stop_processing: false,
    }
}
#[test]
fn json_uses_apple_epoch_and_swift_actions() {
    let mut e = entry("A");
    e.added = 0.;
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["added"].as_f64(), Some(0.));
    assert!(v.get("bookmark").is_none());
    assert_eq!(
        serde_json::to_value(Action::Favorite { value: true }).unwrap(),
        serde_json::json!({"favorite":{"_0":true}})
    );
    assert_eq!(
        serde_json::to_value(Action::SetCollection { id: None }).unwrap(),
        serde_json::json!({"setCollection":{}})
    );
    let mut v = serde_json::to_value(Library::default()).unwrap();
    for key in ["tags", "savedFilters", "rules"] {
        v.as_object_mut().unwrap().remove(key);
    }
    assert!(serde_json::from_value::<Library>(v).is_ok());
    assert!(query::day_timestamp("2001-01-01").unwrap().abs() < 86400.);
}
#[test]
fn rejects_unknown_raw_enums_and_required_omissions() {
    let mut v = serde_json::to_value(Library::default()).unwrap();
    v["view"] = serde_json::json!("unknown");
    assert!(serde_json::from_value::<Library>(v).is_err());
    let mut v = serde_json::to_value(Library::default()).unwrap();
    v.as_object_mut().unwrap().remove("entries");
    assert!(serde_json::from_value::<Library>(v).is_err());
}
#[test]
fn url_validation_and_clipboard() {
    assert_eq!(web_url(" example.com ").unwrap(), "https://example.com/");
    assert!(web_url("ftp://example.com").is_err());
    assert!(web_url("https://bad host.com").is_err());
    assert!(web_url("http://localhost:8080/a").is_ok());
    assert!(services::clipboard_url("word").is_none());
    assert!(services::clipboard_url("example.com two").is_none());
    assert!(services::clipboard_url("http://private").is_some());
}
#[test]
fn normalization_preserves_tag_identity() {
    let mut l = Library::default();
    let t = tag("Work");
    l.tags.push(t.clone());
    let mut e = entry("A");
    e.tags = vec![" work ".into(), "WORK".into(), "".into(), "bad,tag".into()];
    l.entries.push(e);
    l.normalize_tags();
    assert_eq!(l.tags, vec![t]);
    assert_eq!(l.entries[0].tags, vec!["Work"]);
}
#[test]
fn integrity_rejects_cycles_dangling_duplicate_and_dates() {
    let mut l = Library::default();
    let mut c = collection("one", None);
    c.parent_id = Some(c.id);
    l.collections.push(c);
    assert!(l.validate().is_err());
    l.collections.clear();
    let mut e = entry("A");
    e.collection_id = Some(Uuid::new_v4());
    l.entries.push(e);
    assert!(l.validate().is_err());
    l.entries[0].collection_id = None;
    l.entries[0].added = f64::INFINITY;
    assert!(l.validate().is_err());
    l.entries[0].added = 0.;
    l.entries.push(l.entries[0].clone());
    assert!(l.validate().is_err());
}
#[test]
fn scopes_hierarchy_and_missing_ids_fail_closed() {
    let a = collection("A", None);
    let b = collection("B", Some(a.id));
    let mut e = entry("Report");
    e.collection_id = Some(b.id);
    let l = Library {
        collections: vec![a.clone(), b],
        entries: vec![e.clone()],
        ..Library::default()
    };
    let mut w = WindowState {
        scope: a.id.to_string(),
        ..WindowState::default()
    };
    assert_eq!(query::visible(&l, &w).len(), 1);
    w.hierarchical = true;
    assert!(query::visible(&l, &w).is_empty());
    w.query = "report".into();
    assert_eq!(query::visible(&l, &w).len(), 1);
    let p = Predicate {
        collection_id: Some(Uuid::new_v4()),
        ..Predicate::default()
    };
    assert!(!query::predicate(&e, &p, &l));
    w.scope = Uuid::new_v4().to_string();
    assert!(query::visible(&l, &w).is_empty());
}
#[test]
fn predicate_ands_queries_and_independent_tag_groups() {
    let a = tag("A");
    let b = tag("B");
    let mut e = entry("Résumé 2.md");
    e.notes = "planning".into();
    e.tags = vec!["A".into(), "B".into()];
    let l = Library {
        tags: vec![a.clone(), b.clone()],
        entries: vec![e.clone()],
        ..Library::default()
    };
    let mut p = Predicate {
        queries: vec!["resume".into(), "planning".into()],
        tag_ids: [a.id].into(),
        additional_tags: vec![TagConstraint {
            tag_ids: [b.id].into(),
            tag_match: TagMatch::All,
        }],
        file_extension: " .MD ".into(),
        ..Predicate::default()
    };
    assert!(query::predicate(&e, &p, &l));
    p.tag_ids.insert(Uuid::new_v4());
    assert!(!query::predicate(&e, &p, &l));
}
#[test]
fn relative_dates_use_calendar_days() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let mut e = entry("date");
    e.added = query::day_timestamp("2026-10-31").unwrap();
    let mut p = Predicate {
        added: Added::LastSevenDays,
        ..Predicate::default()
    };
    assert!(query::predicate_on(&e, &p, &Library::default(), today));
    e.added = query::day_timestamp("2026-10-30").unwrap();
    assert!(!query::predicate_on(&e, &p, &Library::default(), today));
    p.added = Added::Range;
    p.start_date = query::day_timestamp("2026-10-30").unwrap();
    p.end_date = p.start_date;
    assert!(query::predicate_on(&e, &p, &Library::default(), today));
}
#[test]
fn launcher_ranks_unicode_natural_and_caps_10000() {
    let mut l = Library::default();
    for i in 0..10000 {
        l.entries.push(entry(&format!("Other {i}")))
    }
    l.entries.push(entry("Résumé"));
    l.entries.push(entry("Resume later"));
    let mut e = entry("A resume here");
    e.notes = "notes".into();
    l.entries.push(e);
    let hits = query::launcher(&l, "resume");
    assert_eq!(hits[0].name, "Résumé");
    assert_eq!(hits[1].name, "Resume later");
    assert_eq!(hits[2].name, "A resume here");
    let hits = query::launcher(&l, "");
    assert_eq!(hits.len(), 50);
    assert_eq!(
        query::natural("Item 2", "Item 10"),
        std::cmp::Ordering::Less
    );
}
#[test]
fn additions_are_exact_duplicates_and_no_recursive_scan() {
    let (_d, mut s) = store();
    let e = entry("One");
    let ids = s.add(vec![e.clone(), e.clone()]).unwrap();
    assert_eq!(s.data.entries.len(), 1);
    assert_eq!(ids[0], ids[1]);
    assert!(s.undo.is_empty());
}
#[test]
fn failed_commit_preserves_catalog_and_history() {
    let (d, mut s) = store();
    s.add(vec![entry("A")]).unwrap();
    let before = s.data.clone();
    let e = before.entries[0].clone();
    let mut changed = e.clone();
    changed.favorite = true;
    s.update(&e, changed).unwrap();
    let history = s.undo.len();
    std::fs::remove_file(&s.path).unwrap();
    std::fs::create_dir(&s.path).unwrap();
    let mut next = s.data.clone();
    next.entries[0].notes = "must not publish".into();
    assert!(s.commit(next, "Fail", true).is_err());
    assert!(s.data.entries[0].notes.is_empty());
    assert_eq!(s.undo.len(), history);
    assert!(d.path().exists());
}
#[test]
fn unreadable_library_never_overwritten_and_restore_recovers() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("library.json");
    std::fs::write(&p, "broken").unwrap();
    let mut s = Store::load(p.clone());
    assert!(s.protected);
    assert!(s.add(vec![entry("A")]).is_err());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "broken");
    s.restore(&Backup {
        schema_version: 1,
        created: now(),
        library: Library::default(),
    })
    .unwrap();
    assert!(!s.protected);
    assert!(std::fs::read_dir(s.backup_dir()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("unreadable-")
    }));
}
#[test]
fn undo_preserves_unrelated_fields_and_conflicts_are_atomic() {
    let (_d, mut s) = store();
    s.add(vec![entry("A"), entry("B")]).unwrap();
    let ids: Vec<_> = s.data.entries.iter().map(|e| e.id).collect();
    s.bulk(&ids, "favorite", "true").unwrap();
    let mut next = s.data.clone();
    next.entries[0].notes = "new note".into();
    s.commit(next, "External note", false).unwrap();
    s.replay(false).unwrap();
    assert!(!s.data.entries[0].favorite);
    assert_eq!(s.data.entries[0].notes, "new note");
    s.replay(true).unwrap();
    let mut next = s.data.clone();
    next.entries[1].favorite = false;
    s.commit(next, "Conflict", false).unwrap();
    let before = s.data.clone();
    assert!(s.replay(false).is_err());
    assert_eq!(s.data, before);
    assert!(s.undo.is_empty());
    assert!(s.redo.is_empty());
}
#[test]
fn removal_undo_restores_order_identity_and_rejects_deleted_relations() {
    let (_d, mut s) = store();
    let t = tag("Tag");
    let mut l = s.data.clone();
    l.tags.push(t);
    s.commit(l, "Tag", false).unwrap();
    let mut a = entry("A");
    a.tags.push("Tag".into());
    s.add(vec![a, entry("B"), entry("C")]).unwrap();
    let original = s.data.entries.clone();
    s.bulk(&[original[0].id, original[2].id], "remove", "")
        .unwrap();
    s.replay(false).unwrap();
    assert_eq!(s.data.entries, original);
    s.replay(true).unwrap();
    let mut l = s.data.clone();
    l.tags.clear();
    s.commit(l, "Delete tag", false).unwrap();
    assert!(s.replay(false).is_err());
    assert_eq!(s.data.entries.len(), 1);
}
#[test]
fn rules_are_progressive_stop_and_addition_undo_keeps_entry() {
    let (_d, mut s) = store();
    let t = tag("Reviewed");
    let r1 = rule("Tag", vec![Action::AddTag { id: t.id }]);
    let mut r2 = rule("Favorite tagged", vec![Action::Favorite { value: true }]);
    r2.predicate.tag_ids.insert(t.id);
    let mut l = s.data.clone();
    l.tags.push(t);
    l.rules = vec![r1, r2];
    s.commit(l, "Setup", false).unwrap();
    let ids = s.add(vec![entry("A")]).unwrap();
    assert!(s.data.entries[0].favorite);
    assert_eq!(s.undo.len(), 1);
    s.replay(false).unwrap();
    assert_eq!(s.data.entries[0].id, ids[0]);
    assert!(!s.data.entries[0].favorite);
    assert!(s.data.entries[0].tags.is_empty());
}
#[test]
fn rules_host_and_path_boundaries() {
    let l = Library::default();
    let mut r = rule("R", vec![Action::Favorite { value: true }]);
    r.folder_path = "/work".into();
    let mut e = entry("A");
    e.location = "/work-other/a".into();
    assert!(!rules::matches(&e, &r, &l));
    e.location = "/work/a".into();
    assert!(rules::matches(&e, &r, &l));
    r.folder_path.clear();
    r.url_host = "example.com".into();
    e.kind = Kind::Url;
    e.location = "https://sub.example.com/".into();
    assert!(!rules::matches(&e, &r, &l));
    e.location = "https://example.com/".into();
    assert!(rules::matches(&e, &r, &l));
}
#[test]
fn portable_omits_bookmarks_preserves_dates_and_merge_remaps() {
    let a = collection("A", None);
    let t = tag("Work");
    let mut e = entry("File");
    e.bookmark = Some("YWJj".into());
    e.collection_id = Some(a.id);
    e.tags = vec![t.name.clone()];
    let source = Library {
        entries: vec![e],
        collections: vec![a],
        tags: vec![t],
        ..Library::default()
    };
    let p = Portable::from_library(&source);
    assert!(p.entries[0].bookmark.is_none());
    let imported = transfer::preview(&source, &p, true, false).unwrap();
    assert_eq!(imported.added, 1);
    assert_ne!(
        imported.candidate.entries[0].id,
        imported.candidate.entries[1].id
    );
    assert_eq!(imported.candidate.collections.len(), 2);
    assert_eq!(imported.candidate.tags.len(), 1);
    assert_eq!(
        imported.candidate.entries[1].collection_id,
        Some(imported.candidate.collections[1].id)
    );
    assert_eq!(
        transfer::preview(&source, &p, false, false)
            .unwrap()
            .skipped,
        1
    );
}
#[test]
fn stale_import_cannot_apply() {
    let (_d, mut s) = store();
    let p = Portable::from_library(&Library {
        entries: vec![entry("Import")],
        ..Library::default()
    });
    let preview = transfer::preview(&s.data, &p, false, false).unwrap();
    s.add(vec![entry("Other")]).unwrap();
    assert!(transfer::apply(&mut s, &preview).is_err());
    assert_eq!(s.data.entries.len(), 1);
}
#[test]
fn imported_missing_predicate_references_do_not_attach_to_existing_ids() {
    let t = tag("Existing");
    let mut filter = SavedFilter {
        id: Uuid::new_v4(),
        name: "Filter".into(),
        criteria: Predicate::default(),
        icon: None,
        palette: None,
        sort: None,
    };
    filter.criteria.tag_ids.insert(t.id);
    let src = Library {
        saved_filters: vec![filter],
        ..Library::default()
    };
    let dest = Library {
        tags: vec![t.clone()],
        ..Library::default()
    };
    let p = transfer::preview(&dest, &Portable::from_library(&src), false, false).unwrap();
    assert!(
        !p.candidate.saved_filters[0]
            .criteria
            .tag_ids
            .contains(&t.id)
    );
}
#[test]
fn conservative_duplicate_keys_keep_queries_fragments_and_trailing_slashes() {
    let mut a = Entry::new(
        "A".into(),
        "HTTPS://EXAMPLE.COM:443/a?x=1#y".into(),
        Kind::Url,
    );
    let mut b = a.clone();
    b.location = "https://example.com/a?x=1#y".into();
    assert_eq!(address_key(&a), address_key(&b));
    a.location = "https://example.com/a/".into();
    b.location = "https://example.com/a".into();
    assert_ne!(address_key(&a), address_key(&b));
}
#[test]
fn html_roundtrip_preserves_nesting_escaping_notes_tags_and_favorite() {
    let c = collection("A & B", None);
    let mut e = Entry::new(
        "<Example>".into(),
        "https://example.com/?a=1&b=2".into(),
        Kind::Url,
    );
    e.collection_id = Some(c.id);
    e.notes = "Notes & details".into();
    e.tags = vec!["Work".into()];
    e.favorite = true;
    let mut l = Library {
        entries: vec![e.clone()],
        collections: vec![c],
        ..Library::default()
    };
    l.normalize_tags();
    let out = transfer::html(&l);
    let p = transfer::parse_html(&out).unwrap();
    assert_eq!(p.entries[0].name, e.name);
    assert_eq!(p.entries[0].notes, e.notes);
    assert_eq!(p.entries[0].tags, e.tags);
    assert!(p.entries[0].favorite);
    assert_eq!(p.collections[0].name, "A & B");
    assert_eq!(p.entries[0].collection_id, Some(p.collections[0].id));
    assert!(transfer::parse_html("<script><A HREF='https://evil.test'>X</A></script>").is_err());
}
#[test]
fn csv_quotes_all_fields_and_crlf() {
    let mut e = entry("A, \"B\"");
    e.notes = "line\none".into();
    let s = transfer::csv(&Library {
        entries: vec![e],
        ..Library::default()
    });
    assert!(s.starts_with("\"Name\",\"Kind\""));
    assert!(s.contains("\"A, \"\"B\"\"\""));
    assert!(s.ends_with("\r\n"));
}
#[test]
fn filtered_reordering_preserves_hidden_slots() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();
    assert_eq!(
        query::reorder(&[a, b, c, d], &[a, c], a, c),
        vec![c, b, a, d]
    );
    assert_eq!(
        query::ordered(&[a, b, c], &[b, b, Uuid::new_v4()]),
        vec![b, a, c]
    );
}
#[test]
fn backups_preserve_prior_data_and_restore_full_display() {
    let (_d, mut s) = store();
    s.add(vec![entry("Before")]).unwrap();
    let p = s.backup_now().unwrap();
    let b = Store::restore_preview(&p).unwrap();
    let mut l = s.data.clone();
    l.entries.clear();
    l.view = View::Table;
    s.commit(l, "Change", false).unwrap();
    s.restore(&b).unwrap();
    assert_eq!(s.data.entries.len(), 1);
    assert_eq!(s.data.view, View::MediumCards);
    assert!(s.undo.is_empty());
    assert!(
        s.path
            .with_file_name("library.json.before-smart-collections")
            .exists()
    );
}
#[test]
fn bounded_text_decoding_and_extraction() {
    assert_eq!(services::decode_text(&[0xff, 0xfe, b'A', 0]).unwrap(), "A");
    assert!(services::decode_text(&[0, 1, 2]).is_err());
    assert!(services::decode_text(&[0xff, 0xfe, 1]).is_err());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("sample.txt");
    std::fs::write(&p, "Café planning\nDocument body").unwrap();
    let e = services::local_entry(&p, None).unwrap();
    let text = services::extract(&e, &d.path().join("cache")).unwrap();
    assert!(text.contains("Café"));
    std::fs::write(&p, "changed text").unwrap();
    assert_eq!(
        services::extract(&e, &d.path().join("cache")).unwrap(),
        "changed text"
    );
}
#[test]
fn local_health_does_not_request_web_links() {
    let e = Entry::new("Web".into(), "http://127.0.0.1:1".into(), Kind::Url);
    let r = services::health(&e, std::slice::from_ref(&e), false);
    assert_eq!(r.status, "notVerified");
}

type MockResponse = (u16, Vec<(&'static str, String)>, Vec<u8>);
fn mock_http(responses: Vec<MockResponse>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let mut requests = vec![];
        let start = Instant::now();
        for (status, headers, body) in responses {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(s) => break s,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(10) =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("Mock HTTP accept failed: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = vec![];
            let mut b = [0; 1];
            while stream.read(&mut b).unwrap_or(0) > 0 {
                request.push(b[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let head = request.starts_with("HEAD ");
            requests.push(request);
            let h = headers
                .into_iter()
                .map(|(k, v)| format!("{k}: {v}\r\n"))
                .collect::<String>();
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{h}\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            if !head {
                let _ = stream.write_all(&body);
            }
        }
        requests
    });
    (format!("http://{addr}"), thread)
}
#[test]
fn web_health_uses_head_then_bounded_get_and_strips_credentials() {
    let (url, server) = mock_http(vec![
        (405, vec![], vec![]),
        (404, vec![], b"missing".to_vec()),
    ]);
    let e = Entry::new(
        "Web".into(),
        url.replace("http://", "http://user:secret@"),
        Kind::Url,
    );
    let finding = services::health(&e, &[], true);
    assert_eq!(finding.status, "missing");
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("HEAD /"));
    assert!(requests[1].starts_with("GET /"));
    assert!(requests[1].to_lowercase().contains("range: bytes=0-4095"));
    assert!(!requests.join("").to_lowercase().contains("authorization:"));
    assert!(!requests.join("").to_lowercase().contains("cookie:"));
}
#[test]
fn web_health_redirects_and_transient_responses_are_conservative() {
    for (code, status) in [
        (401, "authenticationRequired"),
        (403, "authenticationRequired"),
        (429, "rateLimited"),
        (503, "temporaryFailure"),
        (200, "reachable"),
    ] {
        let (url, server) = mock_http(vec![
            (302, vec![("Location", "/next".into())], vec![]),
            (code, vec![], vec![]),
        ]);
        let e = Entry::new("Web".into(), url, Kind::Url);
        assert_eq!(services::health(&e, &[], true).status, status);
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("HEAD /next"));
    }
}
#[test]
fn title_fetch_is_data_only_decodes_entities_and_honors_mime() {
    let body=b"<!-- <title>Wrong</title> --><script><title>Wrong</title></script><title> A &amp; B &#x1F680; </title>".to_vec();
    let (url, server) = mock_http(vec![(
        200,
        vec![("Content-Type", "text/html; charset=utf-8".into())],
        body,
    )]);
    assert_eq!(services::page_title(&url).unwrap(), "A & B 🚀");
    server.join().unwrap();
    let (url, server) = mock_http(vec![(
        200,
        vec![("Content-Type", "application/json".into())],
        b"<title>No</title>".to_vec(),
    )]);
    assert!(services::page_title(&url).is_err());
    server.join().unwrap();
}
#[test]
fn removal_undo_restores_multiple_middle_positions() {
    let (_d, mut s) = store();
    s.add(vec![
        entry("A"),
        entry("B"),
        entry("C"),
        entry("D"),
        entry("E"),
    ])
    .unwrap();
    let entries = s.data.entries.clone();
    s.bulk(&[entries[1].id, entries[3].id], "remove", "")
        .unwrap();
    s.replay(false).unwrap();
    assert_eq!(s.data.entries, entries);
}
#[test]
fn pdfkit_extracts_searchable_pdf_with_native_page_bounds() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("sample.pdf");
    let stream = "BT /F1 12 Tf 50 750 Td (Hello PDF library) Tj ET";
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_owned(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),format!("<< /Length {} >>\nstream\n{stream}\nendstream",stream.len())];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = vec![];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{obj}\nendobj\n", i + 1));
    }
    let start = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n"
    ));
    std::fs::write(&p, pdf).unwrap();
    let e = services::local_entry(&p, None).unwrap();
    assert!(
        services::extract(&e, &d.path().join("cache"))
            .unwrap()
            .contains("Hello PDF library")
    );
}

#[test]
fn content_snippets_preserve_original_unicode_and_grapheme_context() {
    let text = format!("{} Café Planning 🚀\n{}", "x".repeat(100), "y".repeat(200));
    let snippet = services::content_snippet(&text, "cafe planning").unwrap();
    assert!(snippet.contains("Café Planning 🚀"));
    assert!(!snippet.contains('\n'));
    assert!(snippet.chars().count() < 200);
    assert!(services::content_snippet(&text, "missing").is_none());
}

#[test]
fn frontend_and_swift_uuid_acronyms_decode_exactly() {
    let c = collection("Parent", None);
    let mut e = entry("A");
    e.collection_id = Some(c.id);
    let value = serde_json::to_value(&e).unwrap();
    assert_eq!(value["collectionID"], c.id.to_string());
    assert!(value.get("collectionId").is_none());
    let child = collection("Child", Some(c.id));
    assert_eq!(
        serde_json::to_value(child).unwrap()["parentID"],
        c.id.to_string()
    );
    let external = serde_json::json!({"queries":[""],"kind":null,"favorite":null,"collectionID":c.id,"fileExtension":"","tagIDs":[],"tagMatch":"any","additionalTags":[{"tagIDs":[],"tagMatch":"all"}],"added":"anytime","startDate":0,"endDate":0});
    let p: Predicate = serde_json::from_value(external.clone()).unwrap();
    assert_eq!(p.collection_id, Some(c.id));
    assert_eq!(
        serde_json::to_value(p).unwrap()["tagIDs"],
        external["tagIDs"]
    );
}
