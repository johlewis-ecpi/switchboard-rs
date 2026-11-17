mod hover_popup;
mod menu;
mod native_viewer;
mod platform;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use switchboard::{
    model::*,
    query::{self, WindowState},
    rules,
    services::{self, ContentResult, Finding},
    store::{self, Backup, Store},
    transfer::{self, ImportPreview, Portable},
    viewer_registry::{Registry as ViewerRegistry, TestResult as ViewerTestResult},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use uuid::Uuid;
use wry::{WebView, WebViewBuilder};
#[derive(Debug)]
enum Message {
    Opened(WindowId, u64, Vec<(Entry, Result<String>)>),
    Magnify(WindowId, f64, bool),
    DismissViewer(WindowId),
    HoverReady(WindowId),
    HoverSize(WindowId, u64, f64),
    Command(WindowId, Value),
    Drop(WindowId, Vec<PathBuf>),
    Content(WindowId, u64, Entry, ContentResult),
    Finding(WindowId, u64, Finding),
    Availability(WindowId, Entry, services::Availability),
    Title(WindowId, u64, String),
    Preview(WindowId, Entry, services::Preview),
    Viewer(WindowId, u64, Entry, Result<switchboard::viewer::Document>),
    ViewerRange(WindowId, u64, u64, Result<Vec<u8>>),
    Menu(String),
    Launch,
}
struct ViewerTest {
    key: String,
    result: ViewerTestResult,
}

struct Shell {
    hover_popup: Option<hover_popup::HoverPopup>,
    context_menu: Option<menu::EntryMenu>,
    viewer_test: Option<ViewerTest>,
    native_viewer: native_viewer::NativeViewer,
    viewer_request: u64,
    viewer_entry: Option<Entry>,
    magnify_enabled: Arc<AtomicBool>,
    _magnify_monitor: platform::MagnifyMonitor,
    window: Window,
    webview: WebView,
    state: WindowState,
    launcher: bool,
    preview_owner: Option<WindowId>,
    pending_preview: Option<Uuid>,
    previous_app: Option<i32>,
    generation: Arc<AtomicU64>,
    health_generation: Arc<AtomicU64>,
    content: HashMap<Uuid, ContentResult>,
    findings: Vec<Finding>,
    import: Option<ImportPreview>,
    restore: Option<Backup>,
    rule_preview: Option<(String, Library)>,
    title_generation: u64,
}
impl Shell {
    fn eval(&self, function: &str, v: &Value) {
        let _ = self
            .webview
            .evaluate_script(&format!("window.{function}({v})"));
    }
}
fn create(
    target: &EventLoopWindowTarget<Message>,
    proxy: EventLoopProxy<Message>,
    state: WindowState,
    launcher: bool,
    preview: Option<(WindowId, Uuid)>,
) -> Shell {
    let window = WindowBuilder::new()
        .with_title(if launcher {
            "Switchboard — Quick Launcher"
        } else {
            "Switchboard"
        })
        .with_inner_size(if launcher {
            LogicalSize::new(640., 460.)
        } else {
            LogicalSize::new(1250., 790.)
        })
        .with_min_inner_size(if launcher {
            LogicalSize::new(400., 200.)
        } else {
            LogicalSize::new(240., 120.)
        })
        .with_always_on_top(launcher)
        .with_visible(false)
        .build(target)
        .expect("create desktop window");
    if preview.is_some() {
        platform::place_preview(&window);
        window.set_title("Switchboard — Preview");
    } else if !launcher
        && !std::env::args()
            .any(|arg| matches!(arg.as_str(), "--smoke-test" | "--hover-smoke-test"))
    {
        platform::restore_library_window_frame(&window);
    }
    let id = window.id();
    let magnify_enabled = Arc::new(AtomicBool::new(state.minimal && !launcher));
    let magnify_proxy = proxy.clone();
    let magnify_monitor =
        platform::monitor_magnify(&window, magnify_enabled.clone(), move |delta, ended| {
            let _ = magnify_proxy.send_event(Message::Magnify(id, delta, ended));
        });
    let drop_proxy = proxy.clone();
    // The capability is injected into the main frame only. A reader cannot use
    // WebKit's raw native message handler, even if it navigates to about:blank.
    let bridge_token = Uuid::new_v4().to_string();
    let bridge_script = format!(
        "window.switchboardPostMessage = payload => {{ const value = JSON.parse(payload); value._bridgeToken = {token}; window.ipc.postMessage(JSON.stringify(value)); }};",
        token = json!(bridge_token)
    );
    let webview = WebViewBuilder::new()
        .with_initialization_script(format!(
            "window.switchboardNativeContextMenu = true; window.switchboardNativeHover = true; window.switchboardPreviewWindow = {};",
            preview.is_some()
        ))
        .with_initialization_script(&bridge_script)
        .with_initialization_script_for_main_only(include_str!("viewer_keys.js"), false)
        .with_html(
            include_str!("ui.html")
                .replace("/* VIEWER_MODULE */", include_str!("viewer.js"))
                .replace("/* VIEWER_MANAGER */", include_str!("viewer_manager.js")),
        )
        .with_navigation_handler(|url| {
            url == "about:blank"
                || url == "about:srcdoc"
                || url.starts_with("about:blank#")
                || url.starts_with("about:srcdoc#")
        })
        .with_ipc_handler(move |r| {
            // WebKit's native handler is visible to child frames as well.
            // Reader frames use about:srcdoc and cannot issue application IPC.
            if r.uri() != "about:blank" {
                return;
            }
            if let Ok(v) = serde_json::from_str::<Value>(r.body())
                && v["_bridgeToken"].as_str() == Some(&bridge_token)
            {
                let _ = proxy.send_event(Message::Command(id, v));
            }
        })
        .with_drag_drop_handler(move |event| {
            if let wry::DragDropEvent::Drop { paths, .. } = event {
                let _ = drop_proxy.send_event(Message::Drop(id, paths));
            }
            true
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .expect("create native webview");
    if !launcher && preview.is_none() {
        window.set_visible(true);
    }
    Shell {
        hover_popup: None,
        context_menu: None,
        viewer_test: None,
        native_viewer: native_viewer::NativeViewer::default(),
        viewer_request: 0,
        viewer_entry: None,
        magnify_enabled,
        _magnify_monitor: magnify_monitor,
        window,
        webview,
        state,
        launcher,
        preview_owner: preview.map(|(owner, _)| owner),
        pending_preview: preview.map(|(_, entry)| entry),
        previous_app: None,
        generation: Arc::new(AtomicU64::new(0)),
        health_generation: Arc::new(AtomicU64::new(0)),
        content: HashMap::new(),
        findings: vec![],
        import: None,
        restore: None,
        rule_preview: None,
        title_generation: 0,
    }
}
fn publish(store: &Store, s: &mut Shell) {
    s.magnify_enabled
        .store(s.state.minimal && !s.launcher, Ordering::Relaxed);
    s.state.validate(&store.data);
    let mut entries = if s.launcher {
        query::launcher(&store.data, &s.state.query)
    } else {
        query::candidates(&store.data, &s.state)
    };
    if !s.launcher {
        entries.retain(|e| {
            let meta = query::metadata(e, &s.state.query);
            let matched = s.content.get(&e.id).is_some_and(|r| r.state == "matched");
            match s.state.search_mode.as_str() {
                "contents" if !s.state.query.trim().is_empty() => matched,
                "both" if !s.state.query.trim().is_empty() => meta || matched,
                _ => meta,
            }
        })
    }
    if !s.launcher {
        query::rank_activity(&mut entries, &s.state.scope, &store.activity);
    }
    let ids: Vec<_> = entries.iter().map(|e| e.id).collect();
    let ids = if s.state.minimal && !matches!(s.state.scope.as_str(), "recent" | "frequent") {
        query::ordered(&ids, &s.state.order)
    } else {
        ids
    };
    s.eval("receive",&json!({"library":store.data,"state":s.state,"visible":ids,"content":s.content,"findings":s.findings,"activity":store.activity,"launcher":s.launcher,"protected":store.protected,"error":store.error,"undo":store.undo.last().map(|h|&h.label),"redo":store.redo.last().map(|h|&h.label)}));
}
fn search(s: &mut Shell, l: &Library, proxy: &EventLoopProxy<Message>) {
    let generation = s.generation.fetch_add(1, Ordering::SeqCst) + 1;
    s.content.clear();
    if s.launcher || s.state.search_mode == "metadata" || s.state.query.trim().is_empty() {
        return;
    }
    let entries: Vec<_> = query::candidates(l, &s.state)
        .into_iter()
        .cloned()
        .collect();
    let q = s.state.query.clone();
    let id = s.window.id();
    let token = s.generation.clone();
    let proxy = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(180));
        if token.load(Ordering::SeqCst) != generation {
            return;
        }
        let entries = Arc::new(entries);
        let index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let (entries, index, token, proxy, q) = (
                    entries.clone(),
                    index.clone(),
                    token.clone(),
                    proxy.clone(),
                    q.clone(),
                );
                scope.spawn(move || {
                    loop {
                        if token.load(Ordering::SeqCst) != generation {
                            break;
                        }
                        let i = index.fetch_add(1, Ordering::SeqCst);
                        let Some(e) = entries.get(i) else { break };
                        let r = services::content(e, &q);
                        let _ = proxy.send_event(Message::Content(id, generation, e.clone(), r));
                    }
                });
            }
        });
    });
}
fn scan(s: &mut Shell, l: &Library, web: bool, proxy: &EventLoopProxy<Message>) {
    let generation = s.health_generation.fetch_add(1, Ordering::SeqCst) + 1;
    s.findings.clear();
    let id = s.window.id();
    let entries = Arc::new(l.entries.clone());
    let index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..4 {
        let (entries, index, token, proxy) = (
            entries.clone(),
            index.clone(),
            s.health_generation.clone(),
            proxy.clone(),
        );
        std::thread::spawn(move || {
            loop {
                if token.load(Ordering::SeqCst) != generation {
                    break;
                }
                let i = index.fetch_add(1, Ordering::SeqCst);
                let Some(e) = entries.get(i) else { break };
                let r = services::health(e, &entries, web);
                let _ = proxy.send_event(Message::Finding(id, generation, r));
            }
        });
    }
}
fn strv<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T> {
    serde_json::from_value(v.clone()).map_err(|e| e.to_string())
}
fn selected(v: &Value) -> Result<Vec<Uuid>> {
    decode(&v["ids"])
}
fn publish_viewer_inventory(s: &Shell) -> Result<()> {
    let registry = ViewerRegistry::default();
    s.eval(
        "receiveViewerInventory",
        &json!({"readers":registry.inventory()?,"directory":registry.root()}),
    );
    Ok(())
}
fn publish_viewer_test(s: &Shell) -> Result<()> {
    if let Some(test) = &s.viewer_test {
        ViewerRegistry::default().record_test(&test.key, &test.result)?;
        s.eval(
            "receiveViewerTestStatus",
            &json!({"request":s.viewer_request,"result":test.result}),
        );
    }
    Ok(())
}
fn close_viewer_test(s: &mut Shell) -> Result<()> {
    if let Some(mut test) = s.viewer_test.take() {
        if test.result.startup == "waiting" {
            test.result.startup = "notCompleted".into();
            test.result.detail = "Closed before startup could be checked".into();
        }
        ViewerRegistry::default().record_test(&test.key, &test.result)?;
    }
    Ok(())
}
fn command(
    s: &mut Shell,
    store: &mut Store,
    v: &Value,
    proxy: &EventLoopProxy<Message>,
) -> Result<()> {
    let action = strv(v, "action");
    match action {
        "listViewers" => publish_viewer_inventory(s)?,
        "installViewer" => {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Install Viewer — choose a folder containing reader.json")
                .pick_folder()
            {
                ViewerRegistry::default().install(&path)?;
                s.eval(
                    "toast",
                    &json!("Viewer installed. Use Test with Sample to check it."),
                );
            }
            publish_viewer_inventory(s)?;
        }
        "setViewerEnabled" => {
            ViewerRegistry::default().set_enabled(strv(v, "key"), decode(&v["enabled"])?)?;
            publish_viewer_inventory(s)?;
        }
        "removeViewer" => {
            ViewerRegistry::default().remove(strv(v, "key"))?;
            publish_viewer_inventory(s)?;
        }
        "testViewer" => {
            let key = strv(v, "key");
            let registry = ViewerRegistry::default();
            let readers = registry.inventory()?;
            let info = readers
                .iter()
                .find(|r| r.key == key)
                .ok_or("Viewer is no longer installed")?;
            let mut picker =
                rfd::FileDialog::new().set_title("Choose a sample to test this viewer");
            if !info.extensions.is_empty() {
                picker = picker.add_filter(&info.name, &info.extensions);
            }
            if let Some(path) = picker.pick_file() {
                let (document, result) = registry.test_sample(key, &path)?;
                close_viewer_test(s)?;
                s.native_viewer.close();
                s.viewer_request = decode(&v["request"])?;
                s.viewer_entry = Some(Entry::new(
                    result.sample_name.clone(),
                    path.to_string_lossy().into(),
                    Kind::File,
                ));
                s.viewer_test = Some(ViewerTest {
                    key: key.into(),
                    result: result.clone(),
                });
                publish_viewer_test(s)?;
                s.eval("receiveViewerSample", &json!({"request":s.viewer_request,"document":document,"key":key,"name":result.sample_name,"result":result}));
            }
        }
        "viewerTestEvent" => {
            if v["request"].as_u64() == Some(s.viewer_request)
                && let Some(test) = &mut s.viewer_test
            {
                let result = &mut test.result;
                match strv(v, "event") {
                    "ready" if result.startup == "waiting" => {
                        result.startup = "passed".into();
                        if result.rendering == "unverified" {
                            result.detail =
                                "Reader started. Check the sample, then confirm rendering.".into();
                        }
                    }
                    "rendered"
                        if result.startup == "passed" && result.rendering == "unverified" =>
                    {
                        result.rendering = "reported".into();
                        result.detail = "Reader reports rendering completed; visual confirmation is still needed.".into();
                    }
                    "error" => {
                        if result.startup == "waiting" {
                            result.startup = "failed".into();
                        } else if matches!(result.startup.as_str(), "passed" | "opened") {
                            result.rendering = "failed".into();
                        }
                        result.detail = strv(v, "message").chars().take(2000).collect();
                    }
                    _ => return Ok(()),
                }
                publish_viewer_test(s)?;
            }
        }
        "viewerTestVerdict" => {
            if v["request"].as_u64() == Some(s.viewer_request)
                && let Some(test) = &mut s.viewer_test
            {
                let passed: bool = decode(&v["passed"])?;
                if passed && !matches!(test.result.startup.as_str(), "passed" | "opened") {
                    return Err("Wait for the reader to start before confirming rendering".into());
                }
                test.result.rendering = if passed { "confirmed" } else { "failed" }.into();
                test.result.detail = if passed {
                    "Rendering visually confirmed by user for this sample"
                } else {
                    "User reported a rendering problem with this sample"
                }
                .into();
                publish_viewer_test(s)?;
            }
        }
        "viewDocument" => {
            let id: Uuid = decode(&v["id"])?;
            let request: u64 = decode(&v["request"])?;
            let entry = store
                .data
                .entries
                .iter()
                .find(|e| e.id == id)
                .cloned()
                .ok_or("File no longer in library")?;
            s.native_viewer.close();
            s.viewer_request = request;
            s.viewer_entry = Some(entry.clone());
            let proxy = proxy.clone();
            let window = s.window.id();
            std::thread::spawn(move || {
                let document = switchboard::viewer::load(&entry);
                let _ = proxy.send_event(Message::Viewer(window, request, entry, document));
            });
        }
        "readViewerRange" => {
            // API v2: the reader asked (via the host page) for a byte range of the open document.
            let request: u64 = decode(&v["request"])?;
            let id: u64 = decode(&v["id"])?;
            if request != s.viewer_request {
                return Ok(());
            }
            let (offset, length): (u64, u64) = (decode(&v["offset"])?, decode(&v["length"])?);
            match s.viewer_entry.as_ref().map(switchboard::viewer::path) {
                Some(Ok(path)) => {
                    let proxy = proxy.clone();
                    let window = s.window.id();
                    std::thread::spawn(move || {
                        let bytes = switchboard::viewer::read_range(&path, offset, length);
                        let _ = proxy.send_event(Message::ViewerRange(window, request, id, bytes));
                    });
                }
                Some(Err(error)) => s.eval(
                    "receiveViewerRange",
                    &json!({"request":request,"id":id,"error":error}),
                ),
                None => s.eval(
                    "receiveViewerRange",
                    &json!({"request":request,"id":id,"error":"No document is open"}),
                ),
            }
        }
        "closeViewer" => {
            if v["request"]
                .as_u64()
                .is_some_and(|request| request != s.viewer_request)
            {
                return Ok(());
            }
            s.viewer_entry = None;
            s.native_viewer.close();
            close_viewer_test(s)?;
        }
        "layoutViewer" => {
            let request: u64 = decode(&v["request"])?;
            let rect: [f64; 4] = decode(&v["rect"])?;
            if request == s.viewer_request
                && let Some(entry) = &s.viewer_entry
            {
                let path = switchboard::viewer::path(entry)?;
                let close_proxy = proxy.clone();
                let window_id = s.window.id();
                if let Err(error) = s.native_viewer.show(&s.window, &path, rect, move || {
                    let _ = close_proxy.send_event(Message::DismissViewer(window_id));
                }) {
                    if let Some(test) = &mut s.viewer_test {
                        test.result.startup = "failed".into();
                        test.result.detail = error.clone();
                        publish_viewer_test(s)?;
                    }
                    return Err(error);
                }
                if let Some(test) = &mut s.viewer_test
                    && test.result.startup == "waiting"
                {
                    test.result.startup = "opened".into();
                    test.result.detail = "Native view opened; check the document and confirm rendering. Quick Look has no startup handshake.".into();
                    publish_viewer_test(s)?;
                }
            }
        }
        "minimalZoom" => {
            if s.state.minimal && !s.launcher {
                let size: u32 = decode(&v["size"])?;
                s.state.minimal_icon_size = size.clamp(48, 160);
                store.save_preferences(&s.state)?;
            }
        }
        "ready" => {}
        "predicateCount" => {
            let p: Predicate = decode(&v["predicate"])?;
            let count = store
                .data
                .entries
                .iter()
                .filter(|e| query::predicate(e, &p, &store.data))
                .count();
            s.eval("predicateCount", &json!({"token":v["token"],"count":count}));
        }
        "availability" => {
            let ids = selected(v)?;
            let entries: Vec<_> = store
                .data
                .entries
                .iter()
                .filter(|e| ids.contains(&e.id))
                .take(100)
                .cloned()
                .collect();
            let proxy = proxy.clone();
            let id = s.window.id();
            std::thread::spawn(move || {
                for entry in entries {
                    let availability = services::availability(&entry);
                    let _ = proxy.send_event(Message::Availability(id, entry, availability));
                }
            });
        }
        "icons" => {
            let ids = selected(v)?;
            let mut icons = serde_json::Map::new();
            for id in ids.into_iter().take(100) {
                if let Some(e) = store
                    .data
                    .entries
                    .iter()
                    .find(|e| e.id == id && e.kind != Kind::Url)
                    && let Some(icon) = services::native_icon(e)
                {
                    icons.insert(id.to_string(), json!(icon));
                }
            }
            s.eval("receiveIcons", &Value::Object(icons));
        }
        "preview" => {
            let id: Uuid = decode(&v["id"])?;
            if let Some(e) = store.data.entries.iter().find(|e| e.id == id).cloned() {
                let proxy = proxy.clone();
                let id = s.window.id();
                std::thread::spawn(move || {
                    let preview = services::preview(&e);
                    let _ = proxy.send_event(Message::Preview(id, e, preview));
                });
            }
        }
        "state" => {
            let old = s.state.clone();
            s.state = decode(&v["state"])?;
            s.state.validate(&store.data);
            if old.scope != s.state.scope
                || old.selected_tags != s.state.selected_tags
                || old.tag_match != s.state.tag_match
            {
                s.state.selected.clear()
            }
            if old.scope != s.state.scope
                && let Some(id) = s
                    .state
                    .scope
                    .strip_prefix("smart:")
                    .and_then(|s| Uuid::parse_str(s).ok())
                && let Some(sort) = store
                    .data
                    .saved_filters
                    .iter()
                    .find(|f| f.id == id)
                    .and_then(|f| f.sort)
            {
                let mut next = store.data.clone();
                next.sort = sort;
                store.commit(next, "Sort", false)?;
            }
            if old.query != s.state.query
                || old.scope != s.state.scope
                || old.selected_tags != s.state.selected_tags
                || old.search_mode != s.state.search_mode
                || old.tag_match != s.state.tag_match
            {
                search(s, &store.data, proxy)
            }
            if !s.launcher && s.preview_owner.is_none() {
                store.save_preferences(&s.state)?;
            }
        }
        "addFiles" | "addFolder" => {
            let paths = if action == "addFolder" {
                rfd::FileDialog::new().pick_folders()
            } else {
                rfd::FileDialog::new().pick_files()
            };
            if let Some(paths) = paths {
                add_paths(s, store, paths)?;
            }
        }
        "addUrl" => {
            let location = web_url(strv(v, "url"))?;
            let supplied = strv(v, "name").trim();
            let name = if supplied.is_empty() {
                url::Url::parse(&location)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| location.clone())
            } else {
                supplied.into()
            };
            let mut e = Entry::new(name, location, Kind::Url);
            e.collection_id = Uuid::parse_str(&s.state.scope).ok();
            let ids = store.add(vec![e])?;
            s.state.selected = ids.into_iter().collect();
            s.eval("closeDialog", &Value::Null);
        }
        "clipboard" => {
            s.title_generation += 1;
            let generation = s.title_generation;
            let raw = std::process::Command::new("/usr/bin/pbpaste")
                .output()
                .map_err(|e| e.to_string())?;
            if let Some(url) = services::clipboard_url(&String::from_utf8_lossy(&raw.stdout)) {
                s.eval("clipboardSuggestion", &json!({"url":url}));
                let proxy = proxy.clone();
                let id = s.window.id();
                std::thread::spawn(move || {
                    if let Ok(title) = services::page_title(&url) {
                        let _ = proxy.send_event(Message::Title(id, generation, title));
                    }
                });
            }
        }
        "update" => {
            let expected: Entry = decode(&v["expected"])?;
            let changed: Entry = decode(&v["entry"])?;
            store.update(&expected, changed)?;
        }
        "bulk" => {
            store.bulk(&selected(v)?, strv(v, "operation"), strv(v, "value"))?;
        }
        "open" => {
            let ids = selected(v)?;
            let entries: Vec<_> = store
                .data
                .entries
                .iter()
                .filter(|e| ids.contains(&e.id))
                .cloned()
                .collect();
            if entries.is_empty() {
                return Err("The reference is no longer in the library".into());
            }
            let request_id = v["requestId"].as_u64().unwrap_or(0);
            let proxy = proxy.clone();
            let window_id = s.window.id();
            // Launching an application or fetching a cloud reference can take
            // time. Keep the UI event loop available to paint loading feedback.
            std::thread::spawn(move || {
                let results = entries
                    .into_iter()
                    .map(|entry| {
                        let result =
                            services::open(&entry, false).map(|()| services::resolve(&entry));
                        (entry, result)
                    })
                    .collect();
                let _ = proxy.send_event(Message::Opened(window_id, request_id, results));
            });
        }
        "reveal" => {
            for id in selected(v)? {
                if let Some(e) = store.data.entries.iter().find(|e| e.id == id).cloned() {
                    services::open(&e, action == "reveal")?;
                    if e.kind != Kind::Url {
                        let resolved = services::resolve(&e);
                        if resolved != e.location {
                            let mut next = store.data.clone();
                            let x = next.entries.iter_mut().find(|x| x.id == id).unwrap();
                            x.location = resolved;
                            x.bookmark = services::bookmark(&x.location);
                            store.commit(next, "Refresh reference", false)?;
                        }
                    }
                }
            }
            if s.launcher {
                s.window.set_visible(false)
            }
        }
        "copy" => {
            use std::io::Write;
            let text = selected(v)?
                .iter()
                .filter_map(|id| store.data.entries.iter().find(|e| e.id == *id))
                .map(|e| e.location.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let mut child = std::process::Command::new("/usr/bin/pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            child
                .stdin
                .take()
                .ok_or("Clipboard unavailable")?
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
            if s.launcher {
                s.window.set_visible(false);
                platform::restore_app(s.previous_app);
            }
        }
        "undo" => store.replay(false)?,
        "redo" => store.replay(true)?,
        "display" => {
            let mut next = store.data.clone();
            if !v["view"].is_null() {
                next.view = decode(&v["view"])?
            }
            if !v["sort"].is_null() {
                next.sort = decode(&v["sort"])?
            }
            if !v["appearance"].is_null() {
                next.appearance = decode(&v["appearance"])?
            }
            if let Some(config) = v.get("configuration") {
                let key = serde_json::to_value(next.view)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned();
                next.configurations.insert(key, config.clone());
            }
            store.commit(next, "Display preferences", false)?;
        }
        "saveCollection" => {
            let c: Collection = decode(&v["collection"])?;
            if c.name.trim().is_empty() {
                return Err("Collection name must not be blank".into());
            }
            let mut next = store.data.clone();
            if let Some(old) = next.collections.iter_mut().find(|x| x.id == c.id) {
                *old = c
            } else {
                next.collections.push(c)
            }
            store.commit(next, "Collection", false)?;
        }
        "deleteCollection" => {
            let id: Uuid = decode(&v["id"])?;
            let mut next = store.data.clone();
            let c = next
                .collections
                .iter()
                .find(|c| c.id == id)
                .ok_or("Collection missing")?
                .clone();
            next.collections.retain(|c| c.id != id);
            for e in &mut next.entries {
                if e.collection_id == Some(id) {
                    e.collection_id = None
                }
            }
            for child in &mut next.collections {
                if child.parent_id == Some(id) {
                    child.parent_id = c.parent_id
                }
            }
            store.commit(next, "Delete collection", false)?;
        }
        "saveTag" => {
            let t: Tag = decode(&v["tag"])?;
            if !valid_tag(&t.name)
                || store
                    .data
                    .tags
                    .iter()
                    .any(|x| x.id != t.id && x.name.eq_ignore_ascii_case(&t.name))
            {
                return Err("Tags need a unique, nonblank name without commas".into());
            }
            let mut next = store.data.clone();
            if let Some(old) = next.tags.iter_mut().find(|x| x.id == t.id) {
                let old_name = old.name.clone();
                *old = t.clone();
                for e in &mut next.entries {
                    for name in &mut e.tags {
                        if *name == old_name {
                            *name = t.name.clone()
                        }
                    }
                }
            } else {
                next.tags.push(t)
            }
            store.commit(next, "Tag", false)?;
        }
        "deleteTag" => {
            let id: Uuid = decode(&v["id"])?;
            let mut next = store.data.clone();
            let name = next
                .tags
                .iter()
                .find(|t| t.id == id)
                .ok_or("Tag missing")?
                .name
                .clone();
            next.tags.retain(|t| t.id != id);
            for e in &mut next.entries {
                e.tags.retain(|t| *t != name)
            }
            store.commit(next, "Delete tag", false)?;
        }
        "saveSmart" => {
            let f: SavedFilter = decode(&v["filter"])?;
            if f.name.trim().is_empty()
                || store
                    .data
                    .saved_filters
                    .iter()
                    .any(|x| x.id != f.id && x.name.eq_ignore_ascii_case(&f.name))
            {
                return Err("Smart collections need a unique nonblank name".into());
            }
            let mut next = store.data.clone();
            if let Some(x) = next.saved_filters.iter_mut().find(|x| x.id == f.id) {
                *x = f
            } else {
                next.saved_filters.push(f)
            }
            store.commit(next, "Smart collection", false)?;
        }
        "deleteSmart" => {
            let mut next = store.data.clone();
            next.saved_filters
                .retain(|f| Some(f.id.to_string().as_str()) != v["id"].as_str());
            store.commit(next, "Delete smart collection", false)?;
        }
        "saveRule" => {
            let r: Rule = decode(&v["rule"])?;
            if r.name.trim().is_empty()
                || store
                    .data
                    .rules
                    .iter()
                    .any(|x| x.id != r.id && x.name.eq_ignore_ascii_case(&r.name))
            {
                return Err("Rules need a unique nonblank name".into());
            }
            if r.enabled && !rules::valid_rule(&r, &store.data) {
                return Err("Rule has invalid or unavailable conditions/actions".into());
            }
            let mut next = store.data.clone();
            if let Some(x) = next.rules.iter_mut().find(|x| x.id == r.id) {
                *x = r
            } else {
                next.rules.push(r)
            }
            store.commit(next, "Rule", false)?;
        }
        "deleteRule" => {
            let mut next = store.data.clone();
            next.rules
                .retain(|r| Some(r.id.to_string().as_str()) != v["id"].as_str());
            store.commit(next, "Delete rule", false)?;
        }
        "moveRule" => {
            let mut next = store.data.clone();
            if let Some(i) = next
                .rules
                .iter()
                .position(|r| r.id.to_string() == strv(v, "id"))
            {
                let j = (i as i64 + v["direction"].as_i64().unwrap_or(0))
                    .clamp(0, next.rules.len() as i64 - 1) as usize;
                next.rules.swap(i, j);
            }
            store.commit(next, "Reorder rules", false)?;
        }
        "previewRules" => {
            let mut next = store.data.clone();
            let ids = if v["all"].as_bool() == Some(true) {
                next.entries.iter().map(|e| e.id).collect()
            } else {
                selected(v)?
            };
            let changes = rules::apply(&mut next, &ids, false);
            s.rule_preview = Some((
                format!(
                    "{}:{}",
                    transfer::fingerprint(&store.data),
                    chrono::Local::now().date_naive()
                ),
                next,
            ));
            s.eval("rulesPreview", &json!(changes));
        }
        "applyRules" => {
            let (fp, next) = s.rule_preview.take().ok_or("Preview rules first")?;
            if fp
                != format!(
                    "{}:{}",
                    transfer::fingerprint(&store.data),
                    chrono::Local::now().date_naive()
                )
            {
                return Err("Rule preview is stale. Preview again".into());
            }
            store.commit(next, "Apply rules", true)?;
            s.eval("closeDialog", &Value::Null);
        }
        "import" => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Library or bookmarks", &["json", "html", "htm"])
                .pick_file()
            {
                let p = Portable::read(&path)?;
                let preview = transfer::preview(
                    &store.data,
                    &p,
                    v["keepBoth"].as_bool().unwrap_or(false),
                    v["processRules"].as_bool().unwrap_or(false),
                )?;
                s.eval("importPreview",&json!({"added":preview.added,"skipped":preview.skipped,"remapped":preview.remapped,"retainedAppearance":preview.retained_appearance,"entries":preview.candidate.entries.iter().filter(|e|!store.data.entries.iter().any(|x|x.id==e.id)).take(50).collect::<Vec<_>>()}));
                s.import = Some(preview);
            }
        }
        "applyImport" => {
            let p = s.import.take().ok_or("Preview an import first")?;
            transfer::apply(store, &p)?;
            s.eval("closeDialog", &Value::Null);
        }
        "export" => {
            let format = strv(v, "format");
            if let Some(p) = rfd::FileDialog::new()
                .set_file_name(format!("Switchboard.{format}"))
                .save_file()
            {
                transfer::export(&p, &store.data, format)?;
                s.eval("toast", &json!("Export saved"));
            }
        }
        "backup" => {
            let p = store.backup_now()?;
            s.eval("toast", &json!(format!("Backup saved: {}", p.display())));
        }
        "showBackups" => {
            std::fs::create_dir_all(store.backup_dir()).map_err(|e| e.to_string())?;
            let status = std::process::Command::new("/usr/bin/open")
                .arg(store.backup_dir())
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("Could not open Backups folder".into());
            }
        }
        "restore" => {
            if let Some(p) = rfd::FileDialog::new()
                .set_directory(store.backup_dir())
                .add_filter("Backup", &["json"])
                .pick_file()
            {
                let b = Store::restore_preview(&p)?;
                s.eval("restorePreview",&json!({"created":b.created,"entries":b.library.entries.len(),"collections":b.library.collections.len()}));
                s.restore = Some(b);
            }
        }
        "applyRestore" => {
            let b = s.restore.take().ok_or("Preview a restore first")?;
            store.restore(&b)?;
            services::clear_cache()?;
            s.eval("closeDialog", &Value::Null);
        }
        "health" => scan(s, &store.data, v["web"].as_bool().unwrap_or(false), proxy),
        "cancelHealth" => {
            s.health_generation.fetch_add(1, Ordering::SeqCst);
        }
        "repair" => {
            let expected: Entry = decode(&v["entry"])?;
            if expected.kind == Kind::Url {
                return Err("Edit web addresses in the inspector".into());
            }
            let p = if expected.kind == Kind::Folder {
                rfd::FileDialog::new().pick_folder()
            } else {
                rfd::FileDialog::new().pick_file()
            };
            if let Some(p) = p {
                let replacement = services::local_entry(&p, None)?;
                let mut next = expected.clone();
                next.location = replacement.location;
                next.bookmark = replacement.bookmark;
                store.update(&expected, next)?;
            }
        }
        "removeFindings" => {
            let entries: Vec<Entry> = decode(&v["entries"])?;
            if entries.iter().any(|e| !store.data.entries.contains(e)) {
                return Err("Health findings changed. Scan again before removing".into());
            }
            store.bulk(
                &entries.iter().map(|e| e.id).collect::<Vec<_>>(),
                "remove",
                "",
            )?;
        }
        "refresh" => search(s, &store.data, proxy),
        "clearCache" => {
            s.generation.fetch_add(1, Ordering::SeqCst);
            services::clear_cache()?;
            search(s, &store.data, proxy)
        }
        "reorder" => {
            let all: Vec<_> = query::candidates(
                &store.data,
                &WindowState {
                    scope: "all".into(),
                    query: String::new(),
                    selected_tags: BTreeSet::new(),
                    hierarchical: false,
                    ..s.state.clone()
                },
            )
            .iter()
            .map(|e| e.id)
            .collect();
            let all = query::ordered(&all, &s.state.order);
            let visible: Vec<Uuid> = decode(&v["visible"])?;
            let moving: Uuid = decode(&v["moving"])?;
            let target: Uuid = decode(&v["target"])?;
            s.state.order = query::reorder(&all, &visible, moving, target);
            store.save_preferences(&s.state)?;
        }
        "dismiss" => {
            s.window.set_visible(false);
            if s.launcher {
                platform::restore_app(s.previous_app);
            }
        }
        "clearError" => store.error = None,
        _ => return Err(format!("Unknown command: {action}")),
    };
    Ok(())
}
fn add_paths(s: &mut Shell, store: &mut Store, paths: Vec<PathBuf>) -> Result<()> {
    let entries = paths
        .iter()
        .map(|p| services::local_entry(p, Uuid::parse_str(&s.state.scope).ok()))
        .collect::<Result<Vec<_>>>()?;
    s.state.selected = store.add(entries)?.into_iter().collect();
    Ok(())
}
fn register_hotkey(
    manager: &GlobalHotKeyManager,
    state: &WindowState,
    old: &mut Option<HotKey>,
) -> Result<()> {
    if let Some(key) = old.take() {
        manager.unregister(key).map_err(|e| e.to_string())?
    }
    if !state.launcher_enabled {
        return Ok(());
    }
    let key: HotKey = state
        .launcher_key
        .parse()
        .map_err(|e| format!("Invalid shortcut: {e}"))?;
    let name = format!("{:?}", key.key);
    if name != "Space" && !(name.starts_with("Key") && name.len() == 4) {
        return Err("Choose Space or an A–Z key".into());
    }
    if !key.mods.intersects(Modifiers::CONTROL | Modifiers::SUPER) {
        return Err("Shortcut requires Control or Command".into());
    }
    manager.register(key).map_err(|e|format!("Global shortcut registration failed: {e}. Quick Launcher remains available in the app menu."))?;
    *old = Some(key);
    Ok(())
}
fn main() {
    if std::env::args().any(|a| a == "--check") {
        let s = Store::load(store::default_path());
        if let Some(e) = s.error {
            eprintln!("{e}");
            std::process::exit(1)
        }
        if let Err(e) = s.data.validate() {
            eprintln!("{e}");
            std::process::exit(1)
        }
        println!("Library valid: {} entries", s.data.entries.len());
        return;
    }
    let mut store = Store::load(store::default_path());
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let hotkey_proxy = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = hotkey_proxy.send_event(Message::Launch);
        }
    }));
    let native_menu = menu::build();
    let menu_proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = menu_proxy.send_event(Message::Menu(event.id.0));
    }));
    let manager = GlobalHotKeyManager::new().expect("global shortcut manager");
    let mut hotkey = None;
    let mut preferences = store.load_preferences();
    if let Err(e) = register_hotkey(&manager, &preferences, &mut hotkey) {
        store.error = Some(e)
    }
    let shell = create(&event_loop, proxy.clone(), preferences.clone(), false, None);
    let mut windows = HashMap::from([(shell.window.id(), shell)]);
    let launcher = create(
        &event_loop,
        proxy.clone(),
        WindowState::default(),
        true,
        None,
    );
    let launcher_id = launcher.window.id();
    windows.insert(launcher_id, launcher);
    let mut hover_smoke_complete = false;
    event_loop.run(move |event, target, flow| {
        if hover_smoke_complete {
            *flow = ControlFlow::Exit;
            return;
        }
        let _ = &native_menu.menu;
        let focused = windows
            .values()
            .any(|s| !s.launcher && s.preview_owner.is_none() && s.window.is_visible() && s.window.is_focused());
        for item in &native_menu.focused_items {
            item.set_enabled(focused);
        }
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Message::HoverReady(id)) => {
                if let Some(s) = windows.get_mut(&id) && let Some(popup) = &mut s.hover_popup {
                    popup.ready();
                }
            }
            Event::UserEvent(Message::HoverSize(id, revision, height)) => {
                if let Some(s) = windows.get(&id)
                    && s.window.is_visible() && s.window.is_focused()
                    && let Some(popup) = &s.hover_popup
                {
                    popup.resize(&s.window, revision, height);
                }
                if std::env::args().any(|a| a == "--hover-smoke-test")
                    && let Some(s) = windows.get_mut(&id)
                    && let Some(popup) = &mut s.hover_popup
                {
                    match popup.verify_smoke_test(&s.window) {
                        Ok(()) => println!("Native hover smoke test: screen-sized popup exceeds small library bounds, retains library focus, and ignores measurements after dismissal"),
                        Err(error) => { eprintln!("Native hover smoke test failed: {error}"); std::process::exit(1); }
                    }
                    hover_smoke_complete = true;
                    *flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(Message::DismissViewer(id)) => {
                if let Some(shell) = windows.get(&id) && shell.viewer_entry.is_some() {
                    shell.eval("closeDialog", &Value::Null);
                }
            }
            Event::UserEvent(Message::Magnify(id, delta, ended)) => {
                if delta.is_finite()
                    && let Some(s) = windows.get(&id)
                    && s.state.minimal
                    && !s.launcher
                {
                    s.eval("minimalMagnify", &json!({"delta":delta,"ended":ended}));
                }
            }
            Event::UserEvent(Message::Menu(action)) => {
                let context_action = windows.iter().find_map(|(id, s)| {
                    s.context_menu.as_ref()?.action(&action).map(|action| (*id, action.to_owned()))
                });
                if let Some((id, action)) = context_action {
                    let s = windows.get_mut(&id).unwrap();
                    s.context_menu = None;
                    s.eval("invokeContextAction", &json!(action));
                    return;
                }

                if action == "quit" {
                    *flow = ControlFlow::Exit;
                    return;
                }
                if action == "launcher" {
                    let _ = proxy.send_event(Message::Launch);
                    return;
                }
                if action == "newWindow" {
                    let shell = create(target, proxy.clone(), preferences.clone(), false, None);
                    windows.insert(shell.window.id(), shell);
                    return;
                }
                let id = windows
                    .iter()
                    .find(|(_, s)| !s.launcher && s.preview_owner.is_none() && s.window.is_focused())
                    .map(|(id, _)| *id)
                    .or_else(|| windows.iter().find(|(_, s)| !s.launcher && s.preview_owner.is_none()).map(|(id, _)| *id));
                if let Some(id) = id {
                    if let Some(ui) = action.strip_prefix("ui:") {
                        let shell = windows.get(&id).unwrap();
                        shell.window.set_visible(true);
                        shell.window.set_focus();
                        shell.eval("invokeMenu", &json!(ui));
                    } else {
                        let _ = proxy.send_event(Message::Command(id, json!({"action":action})));
                    }
                }
            }
            Event::Reopen { .. } => {
                if let Some(s) = windows.values_mut().find(|s| !s.launcher && s.preview_owner.is_none()) {
                    s.window.set_visible(true);
                    s.window.set_minimized(false);
                    s.window.set_focus();
                    publish(&store, s);
                }
            }
            Event::UserEvent(Message::Command(id, v)) => {
                let action = strv(&v, "action");
                if action == "smokeResult" && std::env::args().any(|a| a == "--smoke-test") {
                    println!("Native UI smoke test: {}", v["result"]);
                    if v["ok"] != true {
                        eprintln!("Native UI smoke failed");
                        std::process::exit(1)
                    }
                    *flow = ControlFlow::Exit;
                    return;
                }
                if action == "quit" {
                    *flow = ControlFlow::Exit;
                    return;
                }
                if action == "newWindow" {
                    let shell = create(target, proxy.clone(), preferences.clone(), false, None);
                    windows.insert(shell.window.id(), shell);
                    return;
                }
                if action == "launcher" {
                    let _ = proxy.send_event(Message::Launch);
                    return;
                }
                if action == "showInLibrary" {
                    let eid = Uuid::parse_str(strv(&v, "id")).ok();
                    if let Some(e) = eid
                        .and_then(|id| store.data.entries.iter().find(|e| e.id == id))
                        .cloned()
                    {
                        let dest = windows
                            .iter()
                            .find(|(_, s)| !s.launcher && s.preview_owner.is_none() && s.window.is_visible())
                            .map(|(id, _)| *id)
                            .or_else(|| {
                                windows.iter().find(|(_, s)| !s.launcher && s.preview_owner.is_none()).map(|(id, _)| *id)
                            });
                        if let Some(id) = dest {
                            let s = windows.get_mut(&id).unwrap();
                            s.state.scope = e
                                .collection_id
                                .map(|id| id.to_string())
                                .unwrap_or("all".into());
                            s.state.query.clear();
                            s.state.selected_tags.clear();
                            s.state.selected = BTreeSet::from([e.id]);
                            s.state.inspector = true;
                            s.window.set_visible(true);
                            s.window.set_minimized(false);
                            s.window.set_focus();
                            publish(&store, s);
                        }
                        windows.get(&launcher_id).unwrap().window.set_visible(false);
                    }
                    return;
                }
                if action == "contextMenu" {
                    if let Some(s) = windows.get_mut(&id)
                        && s.preview_owner.is_none()
                        && let Ok(entry_id) = Uuid::parse_str(strv(&v, "id"))
                        && let Some(entry) = store.data.entries.iter().find(|e| e.id == entry_id)
                        && let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64())
                        && x.is_finite() && y.is_finite()
                    {
                        if let Some(popup) = &mut s.hover_popup { popup.hide(); }
                        let reorder = s.state.minimal && !matches!(s.state.scope.as_str(), "recent" | "frequent");
                        match menu::EntryMenu::new(entry, reorder) {
                            Ok(menu) => {
                                s.context_menu = Some(menu);
                                s.context_menu.as_ref().unwrap().show(&s.window, x, y);
                            }
                            Err(error) => s.eval("toast", &json!(error)),
                        }
                    }
                    return;
                }
                if action == "showHover" || action == "hideHover" {
                    if let Some(s) = windows.get_mut(&id) {
                        if action == "hideHover" {
                            if let Some(popup) = &mut s.hover_popup { popup.hide(); }
                        } else if !s.launcher && s.preview_owner.is_none()
                            && s.window.is_visible() && s.window.is_focused()
                            && let (Some(x), Some(y), Some(html)) = (v["x"].as_f64(), v["y"].as_f64(), v["html"].as_str())
                            && x.is_finite() && y.is_finite()
                        {
                            if s.hover_popup.is_none() {
                                match hover_popup::HoverPopup::new(target, proxy.clone(), id) {
                                    Ok(popup) => s.hover_popup = Some(popup),
                                    Err(error) => s.eval("toast", &json!(error)),
                                }
                            }
                            if let Some(popup) = &mut s.hover_popup {
                                popup.show(html, &store.data.appearance, [x, y]);
                            }
                        }
                    }
                    return;
                }
                if action == "openViewerWindow" {
                    if let Some(owner) = windows.get(&id)
                        && owner.preview_owner.is_none()
                        && let Ok(entry) = Uuid::parse_str(strv(&v, "id"))
                        && store.data.entries.iter().any(|e| e.id == entry && e.kind == Kind::File)
                    {
                        let mut state = owner.state.clone();
                        state.minimal = false;
                        let shell = create(target, proxy.clone(), state, false, Some((id, entry)));
                        windows.insert(shell.window.id(), shell);
                    }
                    return;
                }
                if action == "closeViewer"
                    && let Some(shell) = windows.get(&id)
                    && let Some(owner) = shell.preview_owner
                    && v["request"].as_u64() == Some(shell.viewer_request)
                {
                    windows.remove(&id);
                    if let Some(shell) = windows.get(&owner) {
                        shell.window.set_focus();
                    }
                    return;
                }
                let before = store.data.clone();
                if let Some(mut s) = windows.remove(&id) {
                    match command(&mut s, &mut store, &v, &proxy) {
                        Err(e) if action == "layoutViewer" => s.eval("readerFailure", &json!(e)),
                        Err(e) if action == "open" => s.eval("openFinished", &json!({"requestId":v["requestId"],"error":e})),
                        Err(e) if matches!(action, "listViewers" | "installViewer" | "setViewerEnabled" | "removeViewer" | "testViewer") => {
                            if action != "listViewers" { let _ = publish_viewer_inventory(&s); }
                            s.eval("viewerManagerError", &json!(e));
                        },
                        Err(e) => s.eval("toast", &json!(e)),
                        Ok(()) if action != "open" => s.eval("operationSucceeded", &json!(action)),
                        Ok(()) => {},
                    }
                    if action == "viewDocument" && s.preview_owner.is_some() {
                        s.window.set_visible(true);
                        s.window.set_focus();
                    }
                    if !s.launcher && s.preview_owner.is_none() {
                        let changed = preferences.launcher_enabled != s.state.launcher_enabled
                            || preferences.launcher_key != s.state.launcher_key;
                        preferences = s.state.clone();
                        if changed
                            && let Err(e) = register_hotkey(&manager, &preferences, &mut hotkey)
                        {
                            s.eval("toast", &json!(e));
                        }
                    }
                    if !matches!(
                        action,
                        "icons" | "preview" | "availability" | "predicateCount" | "viewDocument" | "readViewerRange" | "closeViewer" | "layoutViewer" | "listViewers" | "installViewer" | "setViewerEnabled" | "removeViewer" | "testViewer" | "viewerTestEvent" | "viewerTestVerdict"
                    ) {
                        publish(&store, &mut s);
                    }
                    if action == "ready" && let Some(entry) = s.pending_preview.take() {
                        s.eval("viewDocument", &json!(entry));
                    }
                    if action == "ready" && !s.launcher && s.preview_owner.is_none()
                        && std::env::args().any(|a| a == "--hover-smoke-test")
                    {
                        s.state.minimal = true;
                        s.window.set_inner_size(LogicalSize::new(240., 120.));
                        publish(&store, &mut s);
                        // Let native resizing finish before simulating a delayed icon hover.
                        let _ = s.webview.evaluate_script("requestAnimationFrame(()=>requestAnimationFrame(()=>{const hover=document.querySelector('#hover');hover.innerHTML='<strong>Hover smoke fixture</strong><div style=\"height:480px\">Document preview</div>';hover.dataset.x=50;hover.dataset.y=50;hover.classList.remove('hidden');positionHover(50,50)}))");
                    }
                    if action == "ready"
                        && s.preview_owner.is_none()
                        && !s.launcher
                        && std::env::args().any(|a| a == "--smoke-test")
                    {
                        let sample = std::env::temp_dir().join(format!("Switchboard-viewer-smoke-{}.txt", Uuid::new_v4()));
                        let document = (|| -> Result<_> {
                            std::fs::write(&sample, "Viewer sample: <tag> & Unicode ✓\nSecond line").map_err(|e| e.to_string())?;
                            Ok(ViewerRegistry::default().test_sample("builtin:text", &sample)?.0)
                        })();
                        let _ = std::fs::remove_file(&sample);
                        match document {
                            Ok(document) => s.eval("runSmokeTest", &json!({"viewerDocument":document})),
                            Err(error) => { eprintln!("Viewer smoke fixture failed: {error}"); *flow = ControlFlow::Exit; }
                        }
                    }
                    windows.insert(id, s);
                }
                if store.data != before {
                    services::prune_cache(&store.data.entries);
                    for s in windows.values_mut() {
                        s.findings.retain(|f| store.data.entries.contains(&f.entry));
                        search(s, &store.data, &proxy);
                        publish(&store, s)
                    }
                } else if action == "open" {
                    for s in windows.values_mut() {
                        publish(&store, s);
                    }
                }
            }
            Event::UserEvent(Message::Drop(id, paths)) => {
                if let Some(s) = windows.get_mut(&id)
                    && let Err(e) = add_paths(s, &mut store, paths)
                {
                    s.eval("toast", &json!(e))
                }
                for s in windows.values_mut() {
                    search(s, &store.data, &proxy);
                    publish(&store, s)
                }
            }
            Event::UserEvent(Message::Content(id, generation, expected, r)) => {
                if let Some(s) = windows.get_mut(&id)
                    && s.generation.load(Ordering::SeqCst) == generation
                    && store.data.entries.contains(&expected)
                {
                    s.content.insert(r.id, r);
                    publish(&store, s)
                }
            }
            Event::UserEvent(Message::Finding(id, generation, r)) => {
                if let Some(s) = windows.get_mut(&id)
                    && s.health_generation.load(Ordering::SeqCst) == generation
                    && store.data.entries.contains(&r.entry)
                {
                    s.findings.push(r);
                    publish(&store, s)
                }
            }
            Event::UserEvent(Message::Opened(id, request_id, results)) => {
                let mut errors = vec![];
                let mut opened = false;
                let mut changed = false;
                for (expected, result) in results {
                    match result {
                        Err(error) => errors.push(format!("{}: {error}", expected.name)),
                        Ok(resolved) => {
                            opened = true;
                            if store.data.entries.iter().any(|e| e.id == expected.id && e.location == expected.location && e.kind == expected.kind)
                                && let Err(error) = store.record_open(expected.id)
                            { errors.push(format!("Could not save opening activity: {error}")); }
                            if expected.kind != Kind::Url && resolved != expected.location && store.data.entries.contains(&expected) {
                                let mut next = store.data.clone();
                                let entry = next.entries.iter_mut().find(|e| e.id == expected.id).unwrap();
                                entry.location = resolved;
                                entry.bookmark = services::bookmark(&entry.location);
                                match store.commit(next, "Refresh reference", false) {
                                    Ok(_) => changed = true,
                                    Err(error) => errors.push(format!("Could not refresh reference: {error}")),
                                }
                            }
                        }
                    }
                }
                if let Some(s) = windows.get(&id) {
                    s.eval("openFinished", &json!({"requestId":request_id,"error":if errors.is_empty(){None}else{Some(errors.join("\n"))}}));
                    if s.launcher && opened { s.window.set_visible(false); }
                }
                if changed { services::prune_cache(&store.data.entries); }
                for s in windows.values_mut() {
                    if changed { search(s, &store.data, &proxy); }
                    publish(&store, s);
                }
            }
            Event::UserEvent(Message::Availability(id, expected, availability)) => {
                if store.data.entries.contains(&expected)
                    && let Some(s) = windows.get(&id)
                {
                    s.eval(
                        "receiveAvailability",
                        &json!({"id":expected.id,"availability":availability}),
                    );
                }
            }
            Event::UserEvent(Message::Viewer(id, request, expected, document)) => {
                if let Some(s) = windows.get(&id)
                    && s.viewer_request == request && s.viewer_entry.as_ref() == Some(&expected)
                {
                    let result = if store.data.entries.contains(&expected) { document } else { Err("Reference changed; reopen the viewer".into()) };
                    s.eval("receiveDocument", &json!({"request":request,"document":result.as_ref().ok(),"error":result.as_ref().err()}));
                }
            }
            Event::UserEvent(Message::ViewerRange(id, request, range, bytes)) => {
                if let Some(s) = windows.get(&id)
                    && s.viewer_request == request
                    && s.viewer_entry.is_some()
                {
                    use base64::Engine;
                    let payload = match bytes {
                        Ok(bytes) => json!({"request":request,"id":range,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                        Err(error) => json!({"request":request,"id":range,"error":error}),
                    };
                    s.eval("receiveViewerRange", &payload);
                }
            }
            Event::UserEvent(Message::Preview(id, expected, preview)) => {
                if store.data.entries.contains(&expected)
                    && let Some(s) = windows.get(&id)
                {
                    s.eval(
                        "receivePreview",
                        &json!({"id":expected.id,"preview":preview}),
                    );
                }
            }
            Event::UserEvent(Message::Title(id, generation, title)) => {
                if let Some(s) = windows.get(&id)
                    && generation == s.title_generation
                {
                    s.eval("clipboardSuggestion", &json!({"title":title}));
                }
            }
            Event::UserEvent(Message::Launch) => {
                let s = windows.get_mut(&launcher_id).unwrap();
                s.previous_app = platform::previous_app();
                platform::place_launcher(&s.window);
                s.state.query.clear();
                s.state.selected.clear();
                if let Some(e) = query::launcher(&store.data, "").first() {
                    s.state.selected.insert(e.id);
                }
                s.window.set_visible(true);
                s.window.set_focus();
                publish(&store, s);
                s.eval("focusSearch", &Value::Null);
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
                window_id,
                ..
            } => {
                if let Some(s) = windows.get_mut(&window_id) {
                    if let Some(popup) = &mut s.hover_popup { popup.hide(); }
                    if !s.launcher && s.preview_owner.is_none() {
                        platform::save_library_window_frame(&s.window);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                if let Some(owner) = windows.get(&window_id).and_then(|s| s.preview_owner) {
                    windows.remove(&window_id);
                    if let Some(s) = windows.get(&owner) {
                        s.window.set_focus();
                    }
                } else if let Some(s) = windows.get_mut(&window_id) {
                    if let Some(popup) = &mut s.hover_popup { popup.hide(); }
                    if !s.launcher {
                        platform::save_library_window_frame(&s.window);
                    }
                    s.window.set_visible(false);
                    windows.retain(|_, s| s.preview_owner != Some(window_id));
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Focused(focused),
                window_id,
                ..
            } => {
                if let Some(s) = windows.get_mut(&window_id) {
                    if !focused && let Some(popup) = &mut s.hover_popup { popup.hide(); }
                    if s.launcher && !focused {
                        s.window.set_visible(false)
                    } else if focused {
                        if let Err(e) = store.daily_backup() {
                            s.eval("toast", &json!(e))
                        }
                        search(s, &store.data, &proxy);
                        publish(&store, s);
                    }
                }
            }
            _ => {}
        }
    });
}
