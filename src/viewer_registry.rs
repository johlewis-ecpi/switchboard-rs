//! Reader inventory, installation and test history, independent of the catalog.
use crate::{
    model::Result,
    services,
    viewer::{self, Document, Manifest},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

pub const NATIVE_EXTENSIONS: &[&str] =
    &["doc", "docx", "docm", "rtf", "ppt", "pptx", "pptm", "pdf"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub checked_at: String,
    pub sample_name: String,
    pub sample_extension: String,
    pub fingerprint: String,
    pub startup: String,
    pub rendering: String,
    pub detail: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    #[serde(default)]
    disabled: BTreeSet<String>,
    #[serde(default)]
    tests: BTreeMap<String, TestResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderInfo {
    pub key: String,
    pub name: String,
    pub id: Option<String>,
    pub version: Option<String>,
    pub extensions: Vec<String>,
    pub source: &'static str,
    pub path: Option<PathBuf>,
    pub enabled: bool,
    pub validation_error: Option<String>,
    pub conflicts: Vec<String>,
    pub fingerprint: Option<String>,
    pub last_test: Option<TestResult>,
    pub test_stale: bool,
}

fn native_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .unwrap_or_else(|| "Unknown macOS version".into())
        }
        #[cfg(not(target_os = "macos"))]
        {
            "Unavailable".into()
        }
    })
}

pub struct Registry {
    root: PathBuf,
    bundled: PathBuf,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new(viewer::reader_dir(), viewer::bundled_readers())
    }
}

fn fingerprint(manifest: &Manifest, html: &str) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(manifest).map_err(|e| e.to_string())?);
    digest.update(html.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

impl Registry {
    pub fn new(root: PathBuf, bundled: PathBuf) -> Self {
        Self { root, bundled }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn state(&self) -> Result<State> {
        let path = self.root.join(".viewer-state.json");
        if !path.try_exists().map_err(|e| e.to_string())? {
            return Ok(State::default());
        }
        serde_json::from_slice(&viewer::bounded(&path, 4 * 1024 * 1024)?)
            .map_err(|e| format!("Viewer settings are unreadable; no changes were made: {e}"))
    }
    fn save(&self, state: &State) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let tmp = self.root.join(format!(".state-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            fs::write(
                &tmp,
                serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            fs::rename(&tmp, self.root.join(".viewer-state.json")).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    pub fn is_enabled(&self, key: &str) -> Result<bool> {
        Ok(!self.state()?.disabled.contains(key))
    }
    fn package_path(&self, key: &str) -> Result<PathBuf> {
        if key == "builtin:html" {
            return Ok(self.bundled.join("html"));
        }
        if key == "builtin:text" {
            return Ok(self.bundled.join("text"));
        }
        let name = key.strip_prefix("user:").ok_or("Unknown reader key")?;
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.starts_with('.')
        {
            return Err("Invalid reader key".into());
        }
        Ok(self.root.join(name))
    }
    fn paths(&self) -> Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_dir() || ty.is_symlink() {
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
    }
    pub fn inventory(&self) -> Result<Vec<ReaderInfo>> {
        let state = self.state()?;
        let native_fingerprint = format!(
            "quicklook-v3-{}-{}-{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            env!("CARGO_PKG_VERSION"),
            native_version()
        );
        let last_test = state.tests.get("builtin:native").cloned();
        let mut rows = vec![ReaderInfo {
            key: "builtin:native".into(),
            name: "macOS Quick Look".into(),
            id: Some("switchboard.native".into()),
            version: Some(native_version().into()),
            extensions: NATIVE_EXTENSIONS.iter().map(|s| (*s).into()).collect(),
            source: "system",
            path: None,
            enabled: !state.disabled.contains("builtin:native"),
            validation_error: if cfg!(target_os = "macos") {
                None
            } else {
                Some("Requires macOS".into())
            },
            conflicts: vec![],
            test_stale: last_test
                .as_ref()
                .is_some_and(|t| t.fingerprint != native_fingerprint),
            fingerprint: Some(native_fingerprint),
            last_test,
        }];
        let mut candidates = vec![
            (
                "builtin:text".to_string(),
                self.bundled.join("text"),
                "bundled",
            ),
            (
                "builtin:html".to_string(),
                self.bundled.join("html"),
                "bundled",
            ),
        ];
        for path in self.paths()? {
            candidates.push((
                format!(
                    "user:{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                path,
                "installed",
            ));
        }
        for (key, path, source) in candidates {
            let result = viewer::read_package(&path);
            let (manifest, hash, validation_error) = match result {
                Ok((m, html)) => {
                    let error = (source == "installed" && m.extensions.is_empty()).then(|| {
                        "Installed readers must declare at least one extension".to_string()
                    });
                    let hash = fingerprint(&m, &html)?;
                    (Some(m), Some(hash), error)
                }
                Err(e) => (None, None, Some(e)),
            };
            let last_test = state.tests.get(&key).cloned();
            rows.push(ReaderInfo {
                name: manifest
                    .as_ref()
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into()
                    }),
                id: manifest.as_ref().map(|m| m.id.clone()),
                version: manifest.as_ref().and_then(|m| m.version.clone()),
                extensions: manifest
                    .as_ref()
                    .map(|m| m.extensions.iter().map(|s| s.to_lowercase()).collect())
                    .unwrap_or_default(),
                enabled: !state.disabled.contains(&key),
                source,
                path: Some(path),
                validation_error,
                conflicts: vec![],
                test_stale: last_test
                    .as_ref()
                    .is_some_and(|t| hash.as_ref() != Some(&t.fingerprint)),
                fingerprint: hash,
                last_test,
                key,
            });
        }
        for i in 0..rows.len() {
            if rows[i].source != "installed"
                || !rows[i].enabled
                || rows[i].validation_error.is_some()
            {
                continue;
            }
            for j in 0..rows.len() {
                if i == j
                    || rows[j].source != "installed"
                    || !rows[j].enabled
                    || rows[j].validation_error.is_some()
                {
                    continue;
                }
                if rows[i].id == rows[j].id
                    || rows[i]
                        .extensions
                        .iter()
                        .any(|ext| rows[j].extensions.contains(ext))
                {
                    let name = rows[j].name.clone();
                    rows[i].conflicts.push(name);
                }
            }
        }
        Ok(rows)
    }
    pub fn resolve(&self, extension: &str) -> Result<Option<(Manifest, String)>> {
        let rows = self.inventory()?;
        let matches: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.source == "installed"
                    && r.enabled
                    && r.validation_error.is_none()
                    && r.extensions
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(extension))
            })
            .collect();
        if matches.len() > 1 || matches.iter().any(|r| !r.conflicts.is_empty()) {
            return Err(format!(
                "Multiple enabled readers conflict for .{extension}. Disable one in Settings → Viewers."
            ));
        }
        matches
            .first()
            .map(|r| viewer::read_package(&self.package_path(&r.key)?))
            .transpose()
    }
    fn info(&self, key: &str) -> Result<ReaderInfo> {
        self.inventory()?
            .into_iter()
            .find(|r| r.key == key)
            .ok_or("Viewer is no longer installed".into())
    }
    pub fn set_enabled(&self, key: &str, enabled: bool) -> Result<()> {
        let row = self.info(key)?;
        if enabled {
            if let Some(error) = row.validation_error {
                return Err(error);
            }
            if row.source == "installed" {
                for other in self.inventory()?.iter().filter(|r| {
                    r.key != key
                        && r.source == "installed"
                        && r.enabled
                        && r.validation_error.is_none()
                }) {
                    if row.id == other.id
                        || row.extensions.iter().any(|e| other.extensions.contains(e))
                    {
                        return Err(format!(
                            "Conflicts with {}. Disable that viewer first.",
                            other.name
                        ));
                    }
                }
            }
        }
        let mut state = self.state()?;
        if enabled {
            state.disabled.remove(key);
        } else {
            state.disabled.insert(key.to_string());
        }
        self.save(&state)
    }
    pub fn install(&self, source: &Path) -> Result<String> {
        let (mut manifest, html) = viewer::read_package(source)?;
        if manifest.extensions.is_empty() {
            return Err("Reader must declare at least one extension".into());
        }
        if manifest.id.len() > 120
            || !manifest
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            || manifest.id.starts_with('.')
        {
            return Err("Reader ID must contain only letters, digits, dots, hyphens or underscores, and cannot start with a dot".into());
        }
        for row in self.inventory()? {
            if row.id.as_ref() == Some(&manifest.id) {
                return Err(format!(
                    "{} is already installed; remove it before installing a replacement",
                    row.name
                ));
            }
            if row.source == "installed"
                && row.enabled
                && row.validation_error.is_none()
                && manifest
                    .extensions
                    .iter()
                    .any(|e| row.extensions.iter().any(|r| e.eq_ignore_ascii_case(r)))
            {
                return Err(format!(
                    "Conflicts with {}. Disable that viewer before installing.",
                    row.name
                ));
            }
        }
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let target = self.root.join(&manifest.id);
        if target.try_exists().map_err(|e| e.to_string())? {
            return Err("Installation directory already exists".into());
        }
        let staging = self.root.join(format!(".install-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let result = (|| {
            // The v1 package is self-contained: copy only the validated entry and manifest.
            manifest.entry = "index.html".into();
            fs::write(staging.join("index.html"), html).map_err(|e| e.to_string())?;
            fs::write(
                staging.join("reader.json"),
                serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            fs::rename(&staging, &target).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(staging);
        }
        result?;
        Ok(format!("user:{}", manifest.id))
    }
    pub fn remove(&self, key: &str) -> Result<()> {
        if !key.starts_with("user:") {
            return Err("System and bundled viewers can be disabled but not removed".into());
        }
        self.info(key)?;
        let path = self.package_path(key)?;
        let mut state = self.state()?;
        let removed = self.root.join(format!(".removed-{}", uuid::Uuid::new_v4()));
        fs::rename(&path, &removed).map_err(|e| e.to_string())?;
        state.tests.remove(key);
        state.disabled.remove(key);
        if let Err(e) = self.save(&state) {
            let _ = fs::rename(&removed, &path);
            return Err(e);
        }
        // remove_dir_all never follows a directory symlink; remove the link itself.
        let metadata = fs::symlink_metadata(&removed).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            fs::remove_file(removed)
        } else {
            fs::remove_dir_all(removed)
        }
        .map_err(|e| e.to_string())
    }
    pub fn test_sample(&self, key: &str, sample: &Path) -> Result<(Document, TestResult)> {
        let info = self.info(key)?;
        if let Some(error) = info.validation_error {
            return Err(error);
        }
        let extension = sample
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if key != "builtin:text" && !info.extensions.contains(&extension) {
            return Err(format!("Choose a sample supported by {}", info.name));
        }
        let mut test = TestResult {
            checked_at: Utc::now().to_rfc3339(),
            sample_name: sample
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            sample_extension: extension.clone(),
            fingerprint: info.fingerprint.ok_or("Missing viewer fingerprint")?,
            startup: "waiting".into(),
            rendering: "unverified".into(),
            detail: "Sample opened; awaiting startup check".into(),
        };
        let loaded = (|| -> Result<_> {
            let entry = crate::model::Entry::new(
                "Viewer test".into(),
                sample.to_string_lossy().into(),
                crate::model::Kind::File,
            );
            let path = viewer::path(&entry)?;
            if key == "builtin:native" {
                return Ok((None, None, None, 1));
            }
            let (manifest, html) = viewer::read_package(&self.package_path(key)?)?;
            if manifest.api_version == 2 {
                let size = path.metadata().map_err(|e| e.to_string())?.len();
                return Ok((Some(html), None, Some(size), 2));
            }
            let text = services::decode_text(&viewer::bounded(&path, 4 * 1024 * 1024)?)?;
            Ok((Some(html), Some(text), None, 1))
        })();
        let (html, text, size, api_version) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                test.startup = "notCompleted".into();
                test.detail = format!("Sample could not be loaded: {error}");
                self.record_test(key, &test)?;
                return Err(test.detail);
            }
        };
        let native = key == "builtin:native";
        Ok((
            Document {
                renderer: if native {
                    "native"
                } else if key == "builtin:html" {
                    "html"
                } else {
                    "package"
                },
                api_version,
                size,
                extension,
                text,
                reader_html: html,
                reader_name: info.name,
                notice: None,
            },
            test,
        ))
    }
    pub fn record_test(&self, key: &str, test: &TestResult) -> Result<()> {
        if self.info(key)?.fingerprint.as_ref() != Some(&test.fingerprint) {
            return Err("Viewer changed during testing. Run a new sample test.".into());
        }
        let mut state = self.state()?;
        state.tests.insert(key.into(), test.clone());
        self.save(&state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Registry) {
        let temp = tempfile::tempdir().unwrap();
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("readers");
        let registry = Registry::new(temp.path().join("installed"), bundled);
        (temp, registry)
    }
    fn package(root: &Path, id: &str, extension: &str) -> PathBuf {
        let path = root.join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("reader.json"), serde_json::json!({"apiVersion":1,"id":id,"name":id,"version":"1.0","extensions":[extension],"entry":"index.html"}).to_string()).unwrap();
        fs::write(path.join("index.html"), "<html>reader</html>").unwrap();
        path
    }
    #[test]
    fn installation_is_independent_rejects_conflicts_and_disabled_readers_are_not_dispatched() {
        let (temp, registry) = fixture();
        let original = package(temp.path(), "example.gantt", "GAN");
        let key = registry.install(&original).unwrap();
        fs::write(original.join("index.html"), "changed outside installation").unwrap();
        assert_eq!(
            registry.resolve("gan").unwrap().unwrap().1,
            "<html>reader</html>"
        );
        assert!(
            registry
                .install(&original)
                .unwrap_err()
                .contains("already installed")
        );
        let other = package(temp.path(), "example.second", "gan");
        assert!(registry.install(&other).unwrap_err().contains("Conflicts"));
        registry.set_enabled(&key, false).unwrap();
        assert!(registry.resolve("gan").unwrap().is_none());
        let other_key = registry.install(&other).unwrap();
        assert!(
            registry
                .set_enabled(&key, true)
                .unwrap_err()
                .contains("Conflicts")
        );
        registry.remove(&other_key).unwrap();
        registry.set_enabled(&key, true).unwrap();
        assert!(registry.resolve("gan").unwrap().is_some());
        registry.remove(&key).unwrap();
        assert!(
            original.join("reader.json").exists(),
            "removal must preserve original package"
        );
        assert!(registry.resolve("gan").unwrap().is_none());
        assert!(registry.remove("builtin:text").is_err());
        assert!(registry.remove("user:../outside").is_err());
    }
    #[test]
    fn broken_packages_are_visible_and_do_not_break_unrelated_formats() {
        let (temp, registry) = fixture();
        package(&registry.root, "good", "mm");
        let bad = package(&registry.root, "bad", "gan");
        fs::write(bad.join("reader.json"), "not json").unwrap();
        let rows = registry.inventory().unwrap();
        assert!(
            rows.iter()
                .any(|r| r.key == "user:bad" && r.validation_error.is_some())
        );
        assert!(registry.resolve("mm").unwrap().is_some());
        assert!(registry.set_enabled("user:bad", true).is_err());
        registry.set_enabled("user:bad", false).unwrap();
        let duplicate = package(&registry.root, "duplicate", "MM");
        assert!(registry.resolve("mm").unwrap_err().contains("Multiple"));
        assert!(
            registry
                .inventory()
                .unwrap()
                .iter()
                .any(|r| !r.conflicts.is_empty())
        );
        fs::remove_dir_all(duplicate).unwrap();
        let missing = registry.root.join("missing-entry");
        fs::create_dir(&missing).unwrap();
        assert!(
            registry
                .inventory()
                .unwrap()
                .iter()
                .any(|r| r.key == "user:missing-entry" && r.validation_error.is_some())
        );
        let external = package(temp.path(), "outside", "txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&external, registry.root.join("linked")).unwrap();
            assert!(
                registry
                    .inventory()
                    .unwrap()
                    .iter()
                    .any(|r| r.key == "user:linked" && r.validation_error.is_some())
            );
            registry.remove("user:linked").unwrap();
            assert!(external.join("reader.json").exists());
        }
    }
    #[test]
    fn history_survives_reload_and_changed_packages_require_new_tests() {
        let (temp, registry) = fixture();
        let key = registry
            .install(&package(temp.path(), "example.reader", "txt"))
            .unwrap();
        let row = registry.info(&key).unwrap();
        let test = TestResult {
            checked_at: Utc::now().to_rfc3339(),
            sample_name: "sample.txt".into(),
            sample_extension: "txt".into(),
            fingerprint: row.fingerprint.unwrap(),
            startup: "passed".into(),
            rendering: "reported".into(),
            detail: "Not visually confirmed".into(),
        };
        registry.record_test(&key, &test).unwrap();
        let reload = Registry::new(registry.root.clone(), registry.bundled.clone());
        let info = reload.info(&key).unwrap();
        assert!(!info.test_stale);
        assert_eq!(info.last_test.unwrap().rendering, "reported");
        fs::write(
            registry.package_path(&key).unwrap().join("index.html"),
            "new reader version",
        )
        .unwrap();
        assert!(reload.info(&key).unwrap().test_stale);
        assert!(
            registry
                .record_test(&key, &test)
                .unwrap_err()
                .contains("changed")
        );
        registry.set_enabled("builtin:text", false).unwrap();
        assert!(!reload.is_enabled("builtin:text").unwrap());
    }
    #[test]
    fn unreadable_settings_are_preserved_and_path_escapes_are_rejected() {
        let (temp, registry) = fixture();
        let original = package(temp.path(), "reader", "txt");
        fs::create_dir_all(&registry.root).unwrap();
        fs::write(registry.root.join(".viewer-state.json"), "broken state").unwrap();
        assert!(registry.install(&original).is_err());
        assert_eq!(
            fs::read_to_string(registry.root.join(".viewer-state.json")).unwrap(),
            "broken state"
        );
        fs::remove_file(registry.root.join(".viewer-state.json")).unwrap();
        let outside = temp.path().join("outside.html");
        fs::write(&outside, "outside").unwrap();
        let manifest = serde_json::json!({"apiVersion":1,"id":"reader","name":"reader","extensions":["txt"],"entry":"../outside.html"});
        fs::write(original.join("reader.json"), manifest.to_string()).unwrap();
        assert!(registry.install(&original).unwrap_err().contains("inside"));
        assert!(!registry.root.join("reader").exists());
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn tests_use_the_requested_reader_even_when_disabled_without_fallback() {
        let (temp, registry) = fixture();
        let key = registry
            .install(&package(temp.path(), "gantt", "gan"))
            .unwrap();
        let sample = temp.path().join("sample.gan");
        fs::write(&sample, "<project/>").unwrap();
        registry.set_enabled(&key, false).unwrap();
        let (doc, result) = registry.test_sample(&key, &sample).unwrap();
        assert_eq!(doc.reader_name, "gantt");
        assert!(doc.notice.is_none());
        assert_eq!(result.startup, "waiting");
        assert_eq!(result.rendering, "unverified");
        assert!(registry.test_sample("builtin:native", &sample).is_err());
        assert!(registry.resolve("gan").unwrap().is_none());
        let html = temp.path().join("page.html");
        fs::write(&html, "<h1>HTML test</h1>").unwrap();
        let (document, _) = registry.test_sample("builtin:html", &html).unwrap();
        assert_eq!(document.renderer, "html");
        assert_eq!(document.reader_name, "HTML Viewer");
        fs::write(&sample, [0, 1]).unwrap();
        assert!(
            registry
                .test_sample(&key, &sample)
                .unwrap_err()
                .contains("could not be loaded")
        );
        let result = registry.info(&key).unwrap().last_test.unwrap();
        assert_eq!(result.startup, "notCompleted");
        assert_eq!(result.rendering, "unverified");
    }
}
