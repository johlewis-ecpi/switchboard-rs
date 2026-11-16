use crate::{
    model::*,
    query::fold,
    store::{atomic_write, read_bounded},
    transfer::{clean_html, entities},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(target_os = "macos")]
pub fn bookmark(path: &str) -> Option<String> {
    use base64::Engine;
    use objc2_foundation::{NSString, NSURL, NSURLBookmarkCreationOptions};
    let u = NSURL::fileURLWithPath(&NSString::from_str(path));
    u.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
        NSURLBookmarkCreationOptions::empty(),
        None,
        None,
    )
    .ok()
    .map(|d| base64::engine::general_purpose::STANDARD.encode(d.to_vec()))
}
#[cfg(not(target_os = "macos"))]
pub fn bookmark(_: &str) -> Option<String> {
    None
}
#[cfg(target_os = "macos")]
pub fn resolve(e: &Entry) -> String {
    use base64::Engine;
    use objc2_foundation::{NSData, NSURL, NSURLBookmarkResolutionOptions};
    let Some(data) = e
        .bookmark
        .as_ref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
    else {
        return e.location.clone();
    };
    let data = NSData::with_bytes(&data);
    let result = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithoutUI
                | NSURLBookmarkResolutionOptions::WithoutMounting,
            None,
            std::ptr::null_mut(),
        )
    };
    result
        .ok()
        .and_then(|u| u.path())
        .map(|s| s.to_string())
        .unwrap_or_else(|| e.location.clone())
}
#[cfg(not(target_os = "macos"))]
pub fn resolve(e: &Entry) -> String {
    e.location.clone()
}
pub fn local_entry(path: &Path, collection: Option<uuid::Uuid>) -> Result<Entry> {
    let path = if path.is_absolute() {
        standard_path(path)
    } else {
        standard_path(
            &std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path),
        )
    };
    let kind = if path.is_dir() {
        Kind::Folder
    } else {
        Kind::File
    };
    let mut e = Entry::new(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        path.to_string_lossy().into_owned(),
        kind,
    );
    e.collection_id = collection;
    e.bookmark = bookmark(&e.location);
    Ok(e)
}
pub fn open(e: &Entry, reveal: bool) -> Result<()> {
    let mut location = if e.kind == Kind::Url {
        e.location.clone()
    } else {
        resolve(e)
    };
    let mut cmd = std::process::Command::new("/usr/bin/open");
    if reveal {
        if e.kind == Kind::Url {
            return Err("Web links cannot be revealed in Finder".into());
        }
        if fs::symlink_metadata(&location)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            // Missing references can still reveal their nearest existing folder.
            if let Some(parent) = Path::new(&location)
                .ancestors()
                .skip(1)
                .find(|p| p.is_dir())
            {
                location = parent.to_string_lossy().into_owned();
            } else {
                return Err("No containing folder is available to reveal".into());
            }
        } else {
            cmd.arg("-R");
        }
    }
    let status = cmd
        .arg("--")
        .arg(location)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The system could not open this reference".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentResult {
    pub id: uuid::Uuid,
    pub state: String,
    pub detail: String,
    pub snippet: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Cache {
    signature: String,
    text: String,
}
static CACHE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn clear_cache() -> Result<()> {
    let _guard = CACHE_LOCK.lock().map_err(|_| "Cache lock unavailable")?;
    CACHE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = cache_dir();
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| e.to_string())?
    }
    Ok(())
}
pub fn prune_cache(entries: &[Entry]) {
    let _guard = CACHE_LOCK.lock().ok();
    if let Ok(files) = fs::read_dir(cache_dir()) {
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_some_and(|s| s == "json")
                && !entries.iter().any(|e| {
                    path.file_stem()
                        .is_some_and(|s| s == e.id.to_string().as_str())
                })
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}
pub fn cache_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Library/Caches/Switchboard/ContentSearch")
}
#[cfg(target_os = "macos")]
pub(crate) fn local_safe(path: &Path) -> Result<()> {
    use std::{ffi::CString, os::macos::fs::MetadataExt};
    let m = fs::metadata(path).map_err(|e| e.to_string())?;
    if m.st_flags() & 0x40000000 != 0 {
        return Err("Cloud placeholder; download skipped".into());
    }
    let p = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(p.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err("Volume status unavailable".into());
    }
    if unsafe { st.assume_init() }.f_flags & libc::MNT_LOCAL as u32 == 0 {
        return Err("Remote volume skipped".into());
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn local_safe(_: &Path) -> Result<()> {
    Err("Non-downloading file access is currently supported on macOS only".into())
}
#[cfg(target_os = "macos")]
fn no_download() -> Result<()> {
    unsafe extern "C" {
        fn setiopolicy_np(kind: i32, scope: i32, policy: i32) -> i32;
    }
    if unsafe { setiopolicy_np(3, 1, 1) } < 0 {
        return Err("Could not disable cloud materialization".into());
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn no_download() -> Result<()> {
    Err("Non-downloading access unavailable".into())
}
pub fn decode_text(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let le = bytes[0] == 0xff;
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err("Truncated UTF-16".into());
        }
        let chars: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if le {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16(&chars).map_err(|_| "Invalid UTF-16".into())
    } else {
        if bytes.contains(&0) {
            return Err("Binary input".into());
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| "Not UTF-8 text".into())
    }
}
#[cfg(target_os = "macos")]
fn pdf_text(path: &Path) -> Result<String> {
    use objc2::{msg_send, rc::Retained, runtime::AnyClass, runtime::AnyObject};
    use objc2_foundation::{NSData, NSString};
    #[link(name = "PDFKit", kind = "framework")]
    unsafe extern "C" {}
    let class = AnyClass::get(c"PDFDocument").ok_or("PDFKit unavailable")?;
    let data = NSData::with_bytes(&read_bounded(path, 8 * 1024 * 1024)?);
    let doc: Option<Retained<AnyObject>> = unsafe {
        let doc: objc2::rc::Allocated<AnyObject> = msg_send![class, alloc];
        msg_send![doc,initWithData:&*data]
    };
    let doc = doc.ok_or("Invalid PDF")?;
    let locked: bool = unsafe { msg_send![&*doc, isLocked] };
    let pages: usize = unsafe { msg_send![&*doc, pageCount] };
    if locked || pages > 200 {
        return Err("Locked PDF or exceeds 200 pages".into());
    }
    let text: Option<Retained<NSString>> = unsafe { msg_send![&*doc, string] };
    text.map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .ok_or("PDF has no extractable text".into())
}
#[cfg(not(target_os = "macos"))]
fn pdf_text(_: &Path) -> Result<String> {
    Err("PDF extraction requires macOS".into())
}
pub fn extract(e: &Entry, dir: &Path) -> Result<String> {
    let epoch = CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    use unicode_segmentation::UnicodeSegmentation;
    if e.kind != Kind::File {
        return Err("Only regular local files are searchable".into());
    }
    no_download()?;
    let p = PathBuf::from(resolve(e));
    local_safe(&p)?;
    let m = fs::metadata(&p).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("Not a regular file".into());
    }
    let ext = p
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let pdf = ext == "pdf";
    if !pdf
        && !matches!(
            ext.as_str(),
            "txt"
                | "md"
                | "markdown"
                | "log"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "tsv"
                | "swift"
                | "rs"
                | "js"
                | "ts"
                | "html"
                | "css"
                | "xml"
        )
    {
        return Err("Unsupported file type".into());
    }
    let limit = if pdf { 8 * 1024 * 1024 } else { 1024 * 1024 };
    if m.len() > limit {
        return Err("File exceeds extraction size limit".into());
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", m.dev(), m.ino())
    };
    #[cfg(not(unix))]
    let identity = String::new();
    let sig = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}:{:?}:{identity}",
            p.display(),
            m.len(),
            m.modified()
        ))
    );
    let cp = dir.join(format!("{}.json", e.id));
    if let Ok(bytes) = read_bounded(&cp, 4 * 1024 * 1024)
        && let Ok(c) = serde_json::from_slice::<Cache>(&bytes)
        && c.signature == sig
    {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(&cp) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Ok(c.text);
    }
    let text = if pdf {
        pdf_text(&p)?
    } else {
        decode_text(&read_bounded(&p, limit)?)?
    };
    let text: String = text.graphemes(true).take(500000).collect();
    let c = Cache {
        signature: sig,
        text: text.clone(),
    };
    if let Ok(b) = serde_json::to_vec(&c) {
        let _guard = CACHE_LOCK.lock().map_err(|_| "Cache lock unavailable")?;
        if CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) == epoch {
            let _ = atomic_write(&cp, &b);
            evict_cache(dir, 20 * 1024 * 1024);
        }
    }
    Ok(text)
}
pub fn evict_cache(dir: &Path, budget: u64) {
    let mut files: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let p = e.ok()?.path();
            let m = fs::metadata(&p).ok()?;
            Some((p, m.len(), m.modified().ok()))
        })
        .collect();
    files.sort_by_key(|x| x.2);
    let mut total: u64 = files.iter().map(|x| x.1).sum();
    for (p, n, _) in files {
        if total <= budget {
            break;
        }
        if fs::remove_file(p).is_ok() {
            total -= n
        }
    }
}
pub fn content_snippet(text: &str, query: &str) -> Option<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let graphemes: Vec<_> = text.graphemes(true).collect();
    let mut normalized = String::new();
    let mut spans = Vec::with_capacity(graphemes.len());
    for g in &graphemes {
        let start = normalized.len();
        normalized.push_str(&fold(g));
        spans.push((start, normalized.len()));
    }
    let query = fold(query);
    let pos = normalized.find(&query)?;
    let end = pos + query.len();
    let first = spans.iter().position(|(_, b)| *b > pos).unwrap_or(0);
    let last = spans.iter().rposition(|(a, _)| *a < end).unwrap_or(first);
    Some(
        graphemes[first.saturating_sub(70)..(last + 101).min(graphemes.len())]
            .concat()
            .replace(['\n', '\r'], " "),
    )
}
pub fn content(e: &Entry, q: &str) -> ContentResult {
    let result = extract(e, &cache_dir());
    match result {
        Ok(text) => {
            if let Some(snippet) = content_snippet(&text, q) {
                ContentResult {
                    id: e.id,
                    state: "matched".into(),
                    detail: String::new(),
                    snippet: Some(snippet),
                }
            } else {
                ContentResult {
                    id: e.id,
                    state: "noMatch".into(),
                    detail: String::new(),
                    snippet: None,
                }
            }
        }
        Err(detail) => ContentResult {
            id: e.id,
            state: "skipped".into(),
            detail,
            snippet: None,
        },
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Availability {
    pub status: String,
    pub detail: String,
    pub checked: f64,
}

pub fn availability(e: &Entry) -> Availability {
    let (status, detail) = if e.kind == Kind::Url {
        (
            "web",
            "Web link; availability is checked only in Library Health".to_owned(),
        )
    } else {
        let path = PathBuf::from(resolve(e));
        match fs::metadata(&path) {
            Err(error) => (
                if error.kind() == std::io::ErrorKind::NotFound {
                    "missing"
                } else {
                    "inaccessible"
                },
                error.to_string(),
            ),
            Ok(metadata) => {
                #[cfg(target_os = "macos")]
                let online_only = {
                    use std::os::macos::fs::MetadataExt;
                    metadata.st_flags() & 0x40000000 != 0
                };
                #[cfg(not(target_os = "macos"))]
                let online_only = false;
                let _ = &metadata;
                if online_only {
                    (
                        "onlineOnly",
                        "File contents are managed by a cloud provider and are not currently local"
                            .into(),
                    )
                } else {
                    #[cfg(unix)]
                    let readable = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                        .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::R_OK) } == 0);
                    #[cfg(not(unix))]
                    let readable = true;
                    if readable {
                        ("available", "Available locally".into())
                    } else {
                        ("inaccessible", "Read access is denied".into())
                    }
                }
            }
        }
    };
    Availability {
        status: status.into(),
        detail,
        checked: now(),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub entry: Entry,
    pub status: String,
    pub detail: String,
    pub duplicates: Vec<uuid::Uuid>,
    pub checked: f64,
}
pub fn health(e: &Entry, entries: &[Entry], web: bool) -> Finding {
    let key = address_key(e);
    let duplicates = entries
        .iter()
        .filter(|x| x.id != e.id && address_key(x) == key)
        .map(|x| x.id)
        .collect();
    let (status, detail) = if e.kind == Kind::Url {
        if web {
            web_probe(&e.location)
        } else {
            (
                "notVerified".into(),
                "Web checks require an explicit command".into(),
            )
        }
    } else {
        let p = PathBuf::from(resolve(e));
        if p.starts_with("/Volumes")
            && p.components()
                .nth(2)
                .is_some_and(|n| !Path::new("/Volumes").join(n).exists())
        {
            (
                "volumeUnavailable".into(),
                "The referenced volume is not mounted".into(),
            )
        } else {
            match local_safe(&p) {
                Ok(()) => (
                    "reachable".into(),
                    if p.to_string_lossy() != e.location {
                        format!("Bookmark resolves to {}", p.display())
                    } else {
                        "Local reference is available".into()
                    },
                ),
                Err(detail) => {
                    let status = match fs::symlink_metadata(&p) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
                        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                            "permissionDenied"
                        }
                        _ => "notVerified",
                    };
                    (status.into(), detail)
                }
            }
        }
    };
    Finding {
        entry: e.clone(),
        status,
        detail,
        duplicates,
        checked: now(),
    }
}
fn web_probe(raw: &str) -> (String, String) {
    let run = (|| -> Result<(u16, String)> {
        let mut u = url::Url::parse(raw).map_err(|e| e.to_string())?;
        let _ = u.set_username("");
        let _ = u.set_password(None);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Switchboard link check")
            .build()
            .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut fallback = false;
        let mut redirects = 0;
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or("Web check timed out")?;
            let mut request = if fallback {
                client.get(u.clone()).header("Range", "bytes=0-4095")
            } else {
                client.head(u.clone())
            };
            request = request
                .header("Accept", "*/*")
                .timeout(remaining.min(Duration::from_secs(10)));
            let response = request.send().map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if redirects >= 5 {
                    return Err("Redirect limit exceeded".into());
                }
                redirects += 1;
                let next = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or("Redirect without location")?;
                u = u.join(next).map_err(|e| e.to_string())?;
                if !matches!(u.scheme(), "http" | "https") {
                    return Err("Redirect to unsupported scheme".into());
                }
                let _ = u.set_username("");
                let _ = u.set_password(None);
                continue;
            }
            if matches!(status, 405 | 501) && !fallback {
                fallback = true;
                continue;
            }
            if fallback {
                let mut bytes = vec![];
                response
                    .take(8192)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
            }
            return Ok((status, format!("HTTP {status}")));
        }
    })();
    match run {
        Ok((code, detail)) => {
            let status = match code {
                200..=299 => "reachable",
                404 | 410 => "missing",
                401 | 403 => "authenticationRequired",
                429 => "rateLimited",
                500..=599 => "temporaryFailure",
                _ => "notVerified",
            };
            (status.into(), detail)
        }
        Err(e) => ("temporaryFailure".into(), e),
    }
}
pub fn clipboard_url(s: &str) -> Option<String> {
    let s = s.trim();
    if s.chars().any(char::is_whitespace) {
        return None;
    }
    let u = web_url(s).ok()?;
    let parsed = url::Url::parse(&u).ok()?;
    let h = parsed.host_str()?;
    if s.contains("://") || h.contains('.') || h == "localhost" || h.contains(':') {
        Some(u)
    } else {
        None
    }
}
pub fn page_title(raw: &str) -> Result<String> {
    let raw = web_url(raw)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .get(raw)
        .header("Accept", "text/html, application/xhtml+xml")
        .send()
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("Page did not return success".into());
    }
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if !ct.is_empty()
        && !ct.to_lowercase().contains("text/html")
        && !ct.to_lowercase().contains("application/xhtml+xml")
    {
        return Err("Page is not HTML".into());
    }
    let mut b = vec![];
    r.take(1048577)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > 1048576 {
        return Err("Page exceeds title lookup limit".into());
    }
    let charset = regex::Regex::new(r#"(?i)charset\s*=\s*["']?([\w-]+)"#).unwrap();
    let early = String::from_utf8_lossy(&b[..b.len().min(4096)]);
    let enc = charset
        .captures(&ct)
        .or_else(|| charset.captures(&early))
        .and_then(|c| encoding_rs::Encoding::for_label(c[1].as_bytes()));
    let text = if let Some(enc) = enc {
        enc.decode(&b).0.into_owned()
    } else {
        String::from_utf8(b.clone())
            .unwrap_or_else(|_| encoding_rs::WINDOWS_1252.decode(&b).0.into_owned())
    };
    let cleaned = clean_html(&text);
    let re = regex::Regex::new(r"(?is)<title\b[^>]*>(.*?)</title\s*>").unwrap();
    let title = re
        .captures(&cleaned)
        .map(|c| {
            entities(&c[1])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(500)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .ok_or("No page title")?;
    Ok(title)
}

#[cfg(target_os = "macos")]
pub fn native_icon(e: &Entry) -> Option<String> {
    let path = PathBuf::from(resolve(e));
    crate::preview_cache::get_or_render(&path, "icon", || Preview {
        image: native_icon_for_path(&path),
        ..Preview::default()
    })
    .image
}
#[cfg(target_os = "macos")]
fn native_icon_for_path(path: &Path) -> Option<String> {
    use base64::Engine;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};
    let image =
        NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    let data = image.TIFFRepresentation()?;
    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data)?;
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png.to_vec())
    ))
}
#[cfg(not(target_os = "macos"))]
pub fn native_icon(_: &Entry) -> Option<String> {
    None
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Preview {
    pub text: Option<String>,
    pub image: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub viewer: Option<String>,
}
static PREVIEW_SLOTS: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());
struct PreviewPermit;
impl PreviewPermit {
    fn acquire() -> Self {
        let mut count = PREVIEW_SLOTS.0.lock().unwrap_or_else(|e| e.into_inner());
        while *count >= 4 {
            count = PREVIEW_SLOTS
                .1
                .wait(count)
                .unwrap_or_else(|e| e.into_inner())
        }
        *count += 1;
        Self
    }
}
impl Drop for PreviewPermit {
    fn drop(&mut self) {
        let mut count = PREVIEW_SLOTS.0.lock().unwrap_or_else(|e| e.into_inner());
        *count -= 1;
        PREVIEW_SLOTS.1.notify_one();
    }
}
pub fn preview(e: &Entry) -> Preview {
    if e.kind != Kind::File {
        return Preview::default();
    }
    let path = PathBuf::from(resolve(e));
    if let Some(preview) =
        registered_reader_preview(&path, &crate::viewer_registry::Registry::default())
    {
        return preview;
    }
    crate::preview_cache::get_or_render(&path, "document", || uncached_preview(&path))
}

// Reader API v1 provides an interactive view, not a thumbnail endpoint. Do not
// send registered formats through an unrelated Quick Look renderer or cache
// this hint: install/disable/remove must take effect on the next hover.
fn registered_reader_preview(
    path: &Path,
    registry: &crate::viewer_registry::Registry,
) -> Option<Preview> {
    if !path.is_file() {
        return None;
    }
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    match registry.resolve(&extension) {
        Ok(Some((manifest, _))) => Some(Preview {
            viewer: Some(manifest.name),
            ..Preview::default()
        }),
        Ok(None) => None,
        Err(error) => Some(Preview {
            error: Some(format!(
                "Viewer configuration: {error}. Check Settings → Viewers."
            )),
            ..Preview::default()
        }),
    }
}

fn uncached_preview(path: &Path) -> Preview {
    let _permit = PreviewPermit::acquire();
    let mut result = Preview::default();
    if let Ok(mut b) = read_bounded_preview(path) {
        let text = loop {
            match decode_text(&b) {
                Ok(s) => break Some(s),
                Err(_) if b.len() > 16380 => {
                    b.pop();
                }
                Err(_) => break None,
            }
        };
        if let Some(text) = text {
            result.text = Some(if text.is_empty() {
                "Empty file".into()
            } else {
                text.lines().take(14).collect::<Vec<_>>().join("\n")
            });
            return result;
        }
    }
    match render_preview(path) {
        Ok(image) => result.image = Some(image),
        Err(error) => result.error = Some(error),
    }
    result
}

#[cfg(target_os = "macos")]
fn render_preview(path: &Path) -> Result<String> {
    render_preview_with_fallback_mode(path, false)
}

#[cfg(target_os = "macos")]
fn render_preview_with_fallback_mode(path: &Path, fallback_only: bool) -> Result<String> {
    use base64::Engine;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = std::env::temp_dir().join(format!("Switchboard-preview-{}", uuid::Uuid::new_v4()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("Could not create preview workspace: {e}"))?;
    let result = (|| {
        let helper = dir.join("preview-helper");
        fs::write(
            &helper,
            include_bytes!(concat!(env!("OUT_DIR"), "/switchboard-preview")),
        )
        .map_err(|e| format!("Could not prepare preview helper: {e}"))?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let output = dir.join("preview.png");
        let errors = dir.join("error.txt");
        let mut command = std::process::Command::new(&helper);
        command.arg(path).arg(&output);
        if fallback_only {
            command.arg("--fallback-only");
        }
        let mut child = command
            .stdout(std::process::Stdio::null())
            .stderr(fs::File::create(&errors).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("Could not start preview helper: {e}"))?;
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let error = read_bounded(&errors, 16 * 1024)
                            .ok()
                            .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| {
                                format!("Native preview helper exited with {status}")
                            });
                        return Err(error);
                    }
                    break;
                }
                Ok(None) if start.elapsed() < Duration::from_secs(22) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                status => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(match status {
                        Err(e) => format!("Could not wait for preview: {e}"),
                        _ => "Native document preview timed out after 22 seconds".into(),
                    });
                }
            }
        }
        let bytes = read_bounded(&output, 8 * 1024 * 1024)
            .map_err(|e| format!("Could not read document preview: {e}"))?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("Native preview returned an invalid image".into());
        }
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    })();
    let _ = fs::remove_dir_all(dir);
    result
}

#[cfg(not(target_os = "macos"))]
fn render_preview(_: &Path) -> Result<String> {
    Err("Document previews require macOS".into())
}

fn read_bounded_preview(path: &Path) -> Result<Vec<u8>> {
    // Render HTML and office documents with Quick Look instead of showing source
    // or extracted text. Its document thumbnail represents the first page/slide.
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if !matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "log"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "csv"
            | "tsv"
            | "swift"
            | "rs"
            | "js"
            | "ts"
            | "css"
            | "xml"
    ) {
        return Err("No text preview".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(16384)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn installed_readers_bypass_native_thumbnails_and_follow_registry_changes() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("reader.json"), r#"{"apiVersion":1,"id":"example.gan","name":"GanttProject Viewer","extensions":["gan"],"entry":"index.html"}"#).unwrap();
        fs::write(package.join("index.html"), "<html>Reader</html>").unwrap();
        let registry = crate::viewer_registry::Registry::new(
            temp.path().join("installed"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("readers"),
        );
        let key = registry.install(&package).unwrap();
        let source = temp.path().join("plan.GAN");
        // The hint checks routing, not document parsing or rendering success.
        fs::write(&source, "Not valid GAN XML").unwrap();
        let preview = registered_reader_preview(&source, &registry).unwrap();
        assert_eq!(preview.viewer.as_deref(), Some("GanttProject Viewer"));
        assert!(preview.error.is_none() && preview.image.is_none() && preview.text.is_none());
        assert!(registered_reader_preview(&temp.path().join("missing.gan"), &registry).is_none());
        registry.set_enabled(&key, false).unwrap();
        assert!(registered_reader_preview(&source, &registry).is_none());
        registry.set_enabled(&key, true).unwrap();
        assert!(registered_reader_preview(&source, &registry).is_some());
        registry.remove(&key).unwrap();
        assert!(registered_reader_preview(&source, &registry).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires native WebKit document rendering"]
    fn fallback_renders_html_and_office_saved_images_without_quick_look() {
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("page.html");
        let source = "<!doctype html><html><body style='background:navy;color:white'><h1>Browser fallback</h1></body></html>";
        fs::write(&html, source).unwrap();
        let image = render_preview_with_fallback_mode(&html, true).unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(image.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 1000);
        assert_eq!(fs::read_to_string(&html).unwrap(), source);

        // Minimal packages isolate the embedded-preview path: with Quick Look
        // disabled, neither a renderer nor a file-type icon can satisfy this.
        let props = dir.path().join("docProps");
        fs::create_dir(&props).unwrap();
        fs::write(props.join("thumbnail.png"), png).unwrap();
        for extension in ["docx", "pptx"] {
            let archive = dir.path().join(format!("saved-preview.{extension}"));
            assert!(
                std::process::Command::new("/usr/bin/zip")
                    .current_dir(dir.path())
                    .args(["-q"])
                    .arg(&archive)
                    .arg("docProps/thumbnail.png")
                    .status()
                    .unwrap()
                    .success()
            );
            let original = fs::read(&archive).unwrap();
            let result = render_preview_with_fallback_mode(&archive, true).unwrap();
            assert!(result.starts_with("data:image/png;base64,iVBOR"));
            assert!(result.len() > 1000);
            assert_eq!(fs::read(&archive).unwrap(), original);
        }
        // Word documents without saved thumbnails use the native importer.
        for directory in ["_rels", "word"] {
            fs::create_dir(dir.path().join(directory)).unwrap();
        }
        fs::write(dir.path().join("[Content_Types].xml"), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        fs::write(dir.path().join("_rels/.rels"), r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#).unwrap();
        fs::write(dir.path().join("word/document.xml"), r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Independent Word preview</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        let word = dir.path().join("no-thumbnail.docx");
        assert!(
            std::process::Command::new("/usr/bin/zip")
                .current_dir(dir.path())
                .arg("-q")
                .arg(&word)
                .args(["[Content_Types].xml", "_rels/.rels", "word/document.xml"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            render_preview_with_fallback_mode(&word, true)
                .unwrap()
                .starts_with("data:image/png;base64,iVBOR")
        );
        // Unsupported content must report the failed fallback explicitly.
        let unsupported = dir.path().join("unsupported.pptx");
        fs::write(&unsupported, "not an Office archive").unwrap();
        assert!(
            render_preview_with_fallback_mode(&unsupported, true)
                .unwrap_err()
                .contains("no usable saved preview")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires access to the macOS Quick Look service"]
    fn native_preview_renders_local_html() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.html");
        fs::write(&path, "<!doctype html><html><body style='background:navy;color:white'><h1>Rendered preview</h1></body></html>").unwrap();
        let entry = Entry::new(
            "HTML".into(),
            path.to_string_lossy().into_owned(),
            Kind::File,
        );
        let result = uncached_preview(Path::new(&entry.location));
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.text.is_none());
        let image = result.image.unwrap();
        assert!(image.starts_with("data:image/png;base64,iVBOR"));
        assert!(image.len() > 1000);
        fs::remove_file(&path).unwrap();
        let missing = uncached_preview(Path::new(&entry.location));
        assert!(missing.image.is_none());
        assert!(missing.error.unwrap().contains("File not found"));
    }

    #[test]
    fn rich_documents_bypass_text_preview() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["html", "htm", "HTML", "doc", "docx", "ppt", "pptx"] {
            let path = dir.path().join(format!("example.{extension}"));
            fs::write(&path, "This should be rendered, not shown as text").unwrap();
            assert!(read_bounded_preview(&path).is_err(), "{extension}");
        }
        let path = dir.path().join("example.txt");
        fs::write(&path, "Plain text preview").unwrap();
        assert_eq!(read_bounded_preview(&path).unwrap(), b"Plain text preview");
    }
}
