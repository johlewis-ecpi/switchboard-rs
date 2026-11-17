//! Format-independent reader registry and bounded, read-only document access.
//! Reader packages are runtime resources; the shell never imports their source.
use crate::{
    model::{Entry, Kind, Result},
    services,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_READER_BYTES: u64 = 16 * 1024 * 1024;
/// Largest byte range a reader may request in one API v2 round trip.
pub const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub renderer: &'static str,
    /// 1: `text` carries the decoded document. 2: no contents; the reader requests byte ranges of `size`.
    pub api_version: u32,
    pub size: Option<u64>,
    pub extension: String,
    pub text: Option<String>,
    pub reader_html: Option<String>,
    pub reader_name: String,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

pub fn reader_dir() -> PathBuf {
    std::env::var_os("SWITCHBOARD_READERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("Library/Application Support/Switchboard/Readers")
        })
}

pub(crate) fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if !path.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Not a regular file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "File exceeds {} MiB viewer limit",
            limit / 1024 / 1024
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_package(directory: &Path) -> Result<(Manifest, String)> {
    if fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Reader directory must not be a symlink".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bounded(&directory.join("reader.json"), 64 * 1024)?)
            .map_err(|e| format!("Invalid reader manifest: {e}"))?;
    if !matches!(manifest.api_version, 1 | 2) {
        return Err(format!(
            "Unsupported reader API version for {}",
            manifest.name
        ));
    }
    if manifest.id.is_empty() || manifest.name.is_empty() {
        return Err("Reader ID and name are required".into());
    }
    if manifest.extensions.iter().any(|ext| {
        ext.is_empty()
            || !ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }) {
        return Err(
            "Extensions must omit dots and contain only letters, digits, hyphens or underscores"
                .into(),
        );
    }
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let entry = root
        .join(&manifest.entry)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !entry.starts_with(&root) {
        return Err("Reader entry must stay inside its package".into());
    }
    let html = String::from_utf8(bounded(&entry, MAX_READER_BYTES)?).map_err(|e| e.to_string())?;
    Ok((manifest, html))
}

#[cfg(test)]
fn lookup(root: &Path, extension: &str) -> Result<Option<(Manifest, String)>> {
    crate::viewer_registry::Registry::new(root.to_path_buf(), bundled_readers()).resolve(extension)
}

pub fn path(entry: &Entry) -> Result<PathBuf> {
    if entry.kind != Kind::File {
        return Err("Select a local file to view".into());
    }
    let path = PathBuf::from(services::resolve(entry));
    if !path.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Only regular files can be viewed".into());
    }
    // Explicit viewing may download a cloud file on a local File Provider volume.
    // Keep this metadata-only: layout and range dispatch also resolve this path
    // on the UI thread. Materialization belongs to the background read below.
    services::local_volume(&path)?;
    Ok(path)
}

#[cfg(target_os = "macos")]
fn with_download<T>(read: impl FnOnce() -> Result<T>) -> Result<T> {
    unsafe extern "C" {
        fn getiopolicy_np(kind: i32, scope: i32) -> i32;
        fn setiopolicy_np(kind: i32, scope: i32, policy: i32) -> i32;
    }
    // sys/resource.h: VFS_MATERIALIZE_DATALESS_FILES, SCOPE_THREAD, and ON.
    // Change only the worker thread, then restore even on an error or unwind.
    struct Restore(i32);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe { setiopolicy_np(3, 1, self.0) };
        }
    }
    let previous = unsafe { getiopolicy_np(3, 1) };
    if previous < 0 || unsafe { setiopolicy_np(3, 1, 2) } < 0 {
        return Err("Could not enable cloud file downloads for this preview".into());
    }
    let _restore = Restore(previous);
    read()
}

#[cfg(not(target_os = "macos"))]
fn with_download<T>(read: impl FnOnce() -> Result<T>) -> Result<T> {
    read()
}

/// Read exactly `length` bytes at `offset` from a regular file, for API v2 readers.
/// Bounds are checked against the file's current length, so a shrinking file is an error, not a short read.
pub fn read_range(path: &Path, offset: u64, length: u64) -> Result<Vec<u8>> {
    // A provider can evict the file after initial load. Range reads may fetch it
    // again, but must never enable downloading on the UI or search threads.
    with_download(|| read_range_from(path, offset, length))
}

fn read_range_from(path: &Path, offset: u64, length: u64) -> Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    if length > MAX_RANGE_BYTES {
        return Err(format!(
            "Range exceeds {} MiB",
            MAX_RANGE_BYTES / 1024 / 1024
        ));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Only regular files can be viewed".into());
    }
    if offset
        .checked_add(length)
        .is_none_or(|end| end > metadata.len())
    {
        return Err("Range is outside the file".into());
    }
    let mut bytes = vec![0; length as usize];
    file.read_exact_at(&mut bytes, offset)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

pub fn load(entry: &Entry) -> Result<Document> {
    let path = path(entry)?;
    load_from(&path, &reader_dir())
}

fn load_from(path: &Path, readers: &Path) -> Result<Document> {
    with_download(|| load_downloadable(path, readers))
}

fn load_downloadable(path: &Path, readers: &Path) -> Result<Document> {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let registry = crate::viewer_registry::Registry::new(readers.to_path_buf(), bundled_readers());
    let reader = registry.resolve(&extension)?;
    let builtin_html = reader.is_none()
        && matches!(extension.as_str(), "html" | "htm" | "xhtml")
        && registry.is_enabled("builtin:html")?;
    let native = reader.is_none()
        && matches!(
            extension.as_str(),
            "doc" | "docx" | "docm" | "rtf" | "ppt" | "pptx" | "pptm" | "pdf"
        );
    if native && !registry.is_enabled("builtin:native")? {
        return Err("The native document viewer is disabled in Settings → Viewers".into());
    }
    if native {
        // Quick Look is attached on the UI thread. Touch the contents here first
        // so cloud retrieval happens while the preview shows its loading state.
        let _ = read_range_from(
            path,
            0,
            path.metadata().map_err(|e| e.to_string())?.len().min(1),
        )
        .map_err(|error| format!("Could not download or read the file for preview: {error}"))?;
    }
    // API v2 readers pull byte ranges on demand, so no size cap or decoding applies.
    let ranged = reader
        .as_ref()
        .is_some_and(|(manifest, _)| manifest.api_version == 2);
    let size = if ranged {
        Some(path.metadata().map_err(|e| e.to_string())?.len())
    } else {
        None
    };
    let text = if native || ranged {
        None
    } else {
        Some(services::decode_text(&bounded(path, MAX_TEXT_BYTES)?)?)
    };
    let specialized = matches!(
        extension.as_str(),
        "md" | "markdown" | "mm" | "gan" | "drawio" | "html" | "htm" | "xhtml"
    );
    let notice = (specialized && reader.is_none() && !builtin_html).then(|| format!("No enabled, valid .{extension} reader is available. Showing source. Manage readers in Settings → Viewers."));
    let (reader_name, reader_html) = if let Some((manifest, html)) = reader {
        (manifest.name, Some(html))
    } else if native {
        ("macOS Quick Look".into(), None)
    } else if builtin_html {
        let (manifest, html) = read_package(&bundled_readers().join("html"))?;
        (manifest.name, Some(html))
    } else {
        if !registry.is_enabled("builtin:text")? {
            return Err("The text viewer is disabled in Settings → Viewers".into());
        }
        let (manifest, html) = read_package(&bundled_readers().join("text"))?;
        (manifest.name, Some(html))
    };
    Ok(Document {
        renderer: if native {
            "native"
        } else if builtin_html {
            "html"
        } else {
            "package"
        },
        api_version: if ranged { 2 } else { 1 },
        size,
        extension,
        text,
        reader_html,
        reader_name,
        notice,
    })
}

pub(crate) fn bundled_readers() -> PathBuf {
    if let Some(path) = std::env::var_os("SWITCHBOARD_BUNDLED_READERS") {
        return path.into();
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let resources = parent.join("../Resources/Readers");
        if resources.is_dir() {
            return resources;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("readers")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn viewer_reads_restore_the_threads_non_downloading_policy() {
        // Use a dedicated thread, as content search does. A viewer read must
        // override its policy temporarily without weakening subsequent search.
        std::thread::spawn(|| {
            unsafe extern "C" {
                fn getiopolicy_np(kind: i32, scope: i32) -> i32;
                fn setiopolicy_np(kind: i32, scope: i32, policy: i32) -> i32;
            }
            assert_eq!(unsafe { setiopolicy_np(3, 1, 1) }, 0);
            let temp = tempfile::tempdir().unwrap();
            let file = temp.path().join("document.txt");
            fs::write(&file, "preview").unwrap();
            let result = with_download(|| {
                assert_eq!(unsafe { getiopolicy_np(3, 1) }, 2);
                read_range(&file, 0, 7)
            });
            assert_eq!(result.unwrap(), b"preview");
            assert_eq!(unsafe { getiopolicy_np(3, 1) }, 1);
            assert!(read_range(&file, 0, 8).is_err());
            assert_eq!(unsafe { getiopolicy_np(3, 1) }, 1);
            assert!(load_from(&file, &temp.path().join("readers")).is_ok());
            assert_eq!(unsafe { getiopolicy_np(3, 1) }, 1);
        })
        .join()
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn viewer_paths_check_file_kind_and_volume_without_reading_contents() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("document.txt");
        fs::write(&file, "preview").unwrap();
        let mut entry = services::local_entry(&file, None).unwrap();
        assert_eq!(path(&entry).unwrap(), file.canonicalize().unwrap());
        entry.kind = Kind::Url;
        assert!(path(&entry).unwrap_err().contains("local file"));
        entry.kind = Kind::File;
        entry.location = temp.path().to_string_lossy().into_owned();
        entry.bookmark = None;
        assert!(path(&entry).unwrap_err().contains("regular files"));
        entry.location = temp.path().join("missing").to_string_lossy().into_owned();
        assert!(path(&entry).is_err());
    }

    #[test]
    fn html_uses_bundled_page_reader_with_disabled_fallback_and_installed_override() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("readers");
        let path = temp.path().join("page.HTML");
        fs::write(&path, "<h1>Visual HTML</h1>").unwrap();
        let document = load_from(&path, &root).unwrap();
        assert_eq!(document.renderer, "html");
        assert_eq!(document.reader_name, "HTML Viewer");
        assert!(document.notice.is_none());
        let registry = crate::viewer_registry::Registry::new(root.clone(), bundled_readers());
        assert!(registry.inventory().unwrap().iter().any(
            |reader| reader.key == "builtin:html" && reader.extensions.contains(&"html".into())
        ));
        registry.set_enabled("builtin:html", false).unwrap();
        let fallback = load_from(&path, &root).unwrap();
        assert_eq!(fallback.reader_name, "Text / Code");
        assert_eq!(fallback.renderer, "package");
        assert!(fallback.notice.is_some());
        registry.set_enabled("builtin:html", true).unwrap();
        let package = install(&root, "custom-html", "html", "index.html");
        fs::write(package.join("index.html"), "<html>Custom reader</html>").unwrap();
        let custom = load_from(&path, &root).unwrap();
        assert_eq!(
            custom.renderer, "package",
            "installed packages cannot obtain the bundled HTML capability"
        );
        assert_eq!(custom.reader_name, "custom-html");
    }

    fn install(root: &Path, name: &str, extension: &str, entry: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("reader.json"), serde_json::json!({"apiVersion":1,"id":name,"name":name,"extensions":[extension],"entry":entry}).to_string()).unwrap();
        dir
    }
    #[test]
    fn packages_override_fallback_and_native_without_source_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("plan.GAN");
        fs::write(&file, "<project/>").unwrap();
        let root = tmp.path().join("readers");
        assert!(load_from(&file, &root).unwrap().notice.is_some());
        let dir = install(&root, "gantt", "gan", "index.html");
        fs::write(dir.join("index.html"), "<html>reader</html>").unwrap();
        let doc = load_from(&file, &root).unwrap();
        assert_eq!(doc.renderer, "package");
        assert!(doc.notice.is_none());
        assert_eq!(doc.text.as_deref(), Some("<project/>"));
        let office = tmp.path().join("deck.pptx");
        fs::write(&office, [0, 1]).unwrap();
        assert_eq!(load_from(&office, &root).unwrap().renderer, "native");
    }
    #[test]
    fn rejects_escaping_entries_duplicates_and_binary_text() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("readers");
        let dir = install(&root, "one", "mm", "../../outside.html");
        fs::write(tmp.path().join("outside.html"), "outside").unwrap();
        assert!(read_package(&dir).unwrap_err().contains("inside"));
        fs::write(
            dir.join("reader.json"),
            r#"{"apiVersion":1,"id":"one","name":"one","extensions":["mm"],"entry":"index.html"}"#,
        )
        .unwrap();
        fs::write(dir.join("index.html"), "one").unwrap();
        let two = install(&root, "two", "mm", "index.html");
        fs::write(two.join("index.html"), "two").unwrap();
        assert!(lookup(&root, "mm").unwrap_err().contains("Multiple"));
        let binary = tmp.path().join("file.bin");
        fs::write(&binary, [0, 1]).unwrap();
        assert!(load_from(&binary, &tmp.path().join("absent")).is_err());
    }
    #[test]
    fn api_v2_readers_get_a_size_instead_of_text_and_can_exceed_the_text_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("readers");
        let dir = install(&root, "zipper", "zip", "index.html");
        fs::write(dir.join("index.html"), "<html>zip reader</html>").unwrap();
        fs::write(
            dir.join("reader.json"),
            r#"{"apiVersion":2,"id":"zipper","name":"zipper","extensions":["zip"],"entry":"index.html"}"#,
        )
        .unwrap();
        let file = tmp.path().join("big.ZIP");
        File::create(&file)
            .unwrap()
            .set_len(MAX_TEXT_BYTES * 3)
            .unwrap();
        let doc = load_from(&file, &root).unwrap();
        assert_eq!(
            (doc.api_version, doc.size, doc.text.is_none()),
            (2, Some(MAX_TEXT_BYTES * 3), true)
        );
        assert_eq!(doc.renderer, "package");
        let text = tmp.path().join("v1.gan");
        fs::write(&text, "<project/>").unwrap();
        let v1 = install(&root, "gantt", "gan", "index.html");
        fs::write(v1.join("index.html"), "<html/>").unwrap();
        let doc = load_from(&text, &root).unwrap();
        assert_eq!((doc.api_version, doc.size), (1, None));
    }
    #[test]
    fn read_range_is_exact_bounded_and_rejects_bad_requests() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("data.bin");
        fs::write(&file, (0..=255u8).collect::<Vec<_>>()).unwrap();
        assert_eq!(
            read_range(&file, 250, 6).unwrap(),
            vec![250, 251, 252, 253, 254, 255]
        );
        assert!(read_range(&file, 0, 0).unwrap().is_empty());
        assert!(read_range(&file, 250, 7).unwrap_err().contains("outside"));
        assert!(
            read_range(&file, u64::MAX, 2)
                .unwrap_err()
                .contains("outside")
        );
        assert!(
            read_range(&file, 0, MAX_RANGE_BYTES + 1)
                .unwrap_err()
                .contains("exceeds")
        );
        assert!(read_range(tmp.path(), 0, 0).is_err());
        let big = tmp.path().join("sparse");
        File::create(&big)
            .unwrap()
            .set_len(6 * 1024 * 1024 * 1024)
            .unwrap();
        assert_eq!(
            read_range(&big, 5 * 1024 * 1024 * 1024, 4).unwrap(),
            vec![0; 4]
        );
    }
    #[test]
    fn rejects_oversized_text_and_unknown_api() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("large.txt");
        File::create(&file)
            .unwrap()
            .set_len(MAX_TEXT_BYTES + 1)
            .unwrap();
        assert!(
            load_from(&file, &tmp.path().join("absent"))
                .unwrap_err()
                .contains("limit")
        );
        let dir = install(tmp.path(), "reader", "mm", "index.html");
        fs::write(
            dir.join("reader.json"),
            r#"{"apiVersion":99,"id":"bad","name":"bad","extensions":["mm"],"entry":"index.html"}"#,
        )
        .unwrap();
        assert!(read_package(&dir).unwrap_err().contains("version"));
    }
}
