use crate::{
    model::*,
    query::{OpenActivity, WindowState},
    rules,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;
#[derive(Clone, Debug)]
pub struct Patch {
    pub before: Option<Entry>,
    pub after: Option<Entry>,
    pub index: usize,
}
#[derive(Clone, Debug)]
pub struct History {
    pub label: String,
    pub patches: Vec<Patch>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub schema_version: u32,
    pub created: f64,
    pub library: Library,
}
pub struct Store {
    pub data: Library,
    pub path: PathBuf,
    pub protected: bool,
    pub error: Option<String>,
    pub undo: Vec<History>,
    pub redo: Vec<History>,
    pub activity: BTreeMap<Uuid, OpenActivity>,
}
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SWITCHBOARD_LIBRARY") {
        return path.into();
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/Switchboard/library.json")
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let f = File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err("Input exceeds size limit".into());
    }
    let mut bytes = vec![];
    f.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Input grew beyond size limit".into());
    }
    Ok(bytes)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Invalid storage path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".switchboard-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
impl Store {
    pub fn load(path: PathBuf) -> Self {
        let mut s = Self {
            data: Library::default(),
            path,
            protected: false,
            error: None,
            undo: vec![],
            redo: vec![],
            activity: BTreeMap::new(),
        };
        if s.path.exists() {
            match read_bounded(&s.path, 40 * 1024 * 1024)
                .and_then(|b| serde_json::from_slice::<Library>(&b).map_err(|e| e.to_string()))
            {
                Ok(mut l) => {
                    l.normalize_tags();
                    s.data = l
                }
                Err(e) => {
                    s.protected = true;
                    s.error = Some(format!(
                        "Library could not be read. Saving is disabled to protect it. Restore a backup to recover. {e}"
                    ))
                }
            }
        }
        s.activity = read_bounded(&s.path.with_file_name("activity.json"), 4 * 1024 * 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        s.activity
            .retain(|_, stats| stats.count > 0 && stats.last_opened.is_finite());
        s
    }
    pub fn record_open(&mut self, id: Uuid) -> Result<()> {
        let mut activity = self.activity.clone();
        let stats = activity.entry(id).or_default();
        stats.count = stats.count.saturating_add(1);
        stats.last_opened = now();
        atomic_write(
            &self.path.with_file_name("activity.json"),
            &serde_json::to_vec_pretty(&activity).map_err(|e| e.to_string())?,
        )?;
        self.activity = activity;
        Ok(())
    }
    pub fn preferences_path(&self) -> PathBuf {
        self.path.with_file_name("preferences.json")
    }
    pub fn load_preferences(&self) -> WindowState {
        let mut w = fs::read(self.preferences_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        WindowState::validate(&mut w, &self.data);
        w
    }
    pub fn save_preferences(&self, w: &WindowState) -> Result<()> {
        atomic_write(
            &self.preferences_path(),
            &serde_json::to_vec_pretty(w).map_err(|e| e.to_string())?,
        )
    }
    pub fn backup_dir(&self) -> PathBuf {
        self.path.with_file_name("Backups")
    }
    pub fn checkpoint(&self, prefix: &str) -> Result<PathBuf> {
        let bytes = read_bounded(&self.path, 40 * 1024 * 1024)?;
        let l: Library = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        l.validate()?;
        let path = self
            .backup_dir()
            .join(format!("{prefix}-{}.json", Uuid::new_v4()));
        self.write_backup(&path, &l)?;
        Ok(path)
    }
    fn write_backup(&self, path: &Path, l: &Library) -> Result<()> {
        l.validate()?;
        let b = Backup {
            schema_version: 1,
            created: now(),
            library: l.clone(),
        };
        atomic_write(
            path,
            &serde_json::to_vec_pretty(&b).map_err(|e| e.to_string())?,
        )
    }
    pub fn daily_backup(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        let p = self.backup_dir().join(format!("daily-{day}.json"));
        if !p.exists() {
            let l: Library = serde_json::from_slice(&read_bounded(&self.path, 40 * 1024 * 1024)?)
                .map_err(|e| e.to_string())?;
            self.write_backup(&p, &l)?;
        }
        let mut daily: Vec<_> = fs::read_dir(self.backup_dir())
            .map_err(|e| e.to_string())?
            .filter_map(|x| x.ok())
            .map(|x| x.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("daily-"))
            })
            .collect();
        daily.sort();
        let remove = daily.len().saturating_sub(30);
        for p in daily.into_iter().take(remove) {
            fs::remove_file(p).map_err(|e| e.to_string())?
        }
        Ok(())
    }
    fn persist(&self, l: &Library) -> Result<()> {
        if self.protected {
            return Err("Saving is disabled: restore a valid backup first".into());
        }
        l.validate()?;
        let bytes = serde_json::to_vec_pretty(l).map_err(|e| e.to_string())?;
        if self.path.exists() {
            for suffix in ["before-smart-collections", "before-bulk-organization"] {
                let p = self.path.with_file_name(format!("library.json.{suffix}"));
                if !p.exists() {
                    atomic_write(&p, &read_bounded(&self.path, 40 * 1024 * 1024)?)?
                }
            }
            self.daily_backup()?;
        }
        atomic_write(&self.path, &bytes)
    }
    pub fn commit(&mut self, mut next: Library, label: &str, history: bool) -> Result<usize> {
        rules::disable_invalid(&mut next);
        let patches = diff(&self.data, &next);
        if self.data == next {
            return Ok(0);
        }
        self.persist(&next)?;
        self.data = next;
        let n = patches.len();
        if history && !patches.is_empty() {
            self.undo.push(History {
                label: label.into(),
                patches,
            });
            self.redo.clear()
        }
        Ok(n)
    }
    pub fn add(&mut self, mut entries: Vec<Entry>) -> Result<Vec<Uuid>> {
        let mut next = self.data.clone();
        let mut ids = vec![];
        let mut existing = vec![];
        for e in entries.drain(..) {
            if let Some(old) = next.entries.iter().find(|old| old.location == e.location) {
                existing.push(old.id)
            } else {
                ids.push(e.id);
                next.entries.push(e)
            }
        }
        let before_rules = next.clone();
        rules::apply(&mut next, &ids, true);
        self.commit(next, "Addition rules", false)?;
        let patches = diff(&before_rules, &self.data);
        if !patches.is_empty() {
            self.undo.push(History {
                label: "Addition rules".into(),
                patches,
            });
            self.redo.clear()
        }
        ids.extend(existing);
        Ok(ids)
    }
    pub fn update(&mut self, expected: &Entry, mut changed: Entry) -> Result<usize> {
        let mut next = self.data.clone();
        let i = next
            .entries
            .iter()
            .position(|e| e.id == expected.id)
            .ok_or("Entry no longer exists")?;
        if next.entries[i] != *expected {
            return Err("Entry changed; reload its details".into());
        }
        changed.name = changed.name.trim().into();
        if changed.name.is_empty() {
            return Err("Name must not be blank".into());
        }
        if changed.kind == Kind::Url {
            changed.location = web_url(&changed.location)?
        }
        next.entries[i] = changed;
        next.normalize_tags();
        self.commit(next, "Edit entry", true)
    }
    pub fn bulk(&mut self, ids: &[Uuid], action: &str, value: &str) -> Result<usize> {
        let mut next = self.data.clone();
        let collection = if action == "collection" && !value.is_empty() {
            let id = Uuid::parse_str(value).map_err(|e| e.to_string())?;
            if !next.collections.iter().any(|c| c.id == id) {
                return Err("Collection no longer exists".into());
            }
            Some(id)
        } else {
            None
        };
        let tag = if matches!(action, "addTag" | "removeTag") {
            Some(
                next.tags
                    .iter()
                    .find(|t| t.id.to_string() == value)
                    .ok_or("Tag no longer exists")?
                    .name
                    .clone(),
            )
        } else {
            None
        };
        if action == "remove" {
            next.entries.retain(|e| !ids.contains(&e.id))
        } else {
            for e in &mut next.entries {
                if !ids.contains(&e.id) {
                    continue;
                }
                match action {
                    "favorite" => e.favorite = value == "true",
                    "collection" => e.collection_id = collection,
                    "addTag" => {
                        let t = tag.clone().unwrap();
                        if !e.tags.contains(&t) {
                            e.tags.push(t)
                        }
                    }
                    "removeTag" => e.tags.retain(|t| Some(t) != tag.as_ref()),
                    _ => return Err("Unknown bulk action".into()),
                }
            }
        }
        self.commit(next, "Bulk organization", true)
    }
    pub fn replay(&mut self, redo: bool) -> Result<()> {
        let source = if redo { &self.redo } else { &self.undo };
        let h = source.last().ok_or("No history available")?.clone();
        let result = (|| {
            let mut next = self.data.clone();
            let patches = h.patches.clone();
            for p in &patches {
                let (expected, target) = if redo {
                    (&p.before, &p.after)
                } else {
                    (&p.after, &p.before)
                };
                match (expected, target) {
                    (Some(a), Some(b)) => {
                        let e = next
                            .entries
                            .iter_mut()
                            .find(|e| e.id == a.id)
                            .ok_or("History target is missing")?;
                        apply_patch(e, a, b)?
                    }
                    (Some(a), None) => {
                        let i = next
                            .entries
                            .iter()
                            .position(|e| e.id == a.id)
                            .ok_or("Removed entry is missing")?;
                        if next.entries[i] != *a {
                            return Err("Entry changed since this action".into());
                        }
                        next.entries.remove(i);
                    }
                    (None, Some(b)) => {
                        if next.entries.iter().any(|e| e.id == b.id) {
                            return Err("Entry identity already exists".into());
                        }
                        next.entries
                            .insert(p.index.min(next.entries.len()), b.clone());
                    }
                    _ => {}
                }
            }
            for e in &next.entries {
                if e.tags
                    .iter()
                    .any(|n| !next.tags.iter().any(|t| t.name == *n))
                {
                    return Err("History references deleted tags".into());
                }
            }
            self.persist(&next)?;
            self.data = next;
            Ok(())
        })();
        if result.is_err() {
            self.undo.clear();
            self.redo.clear();
            return result;
        }
        if redo {
            self.redo.pop();
            self.undo.push(h)
        } else {
            self.undo.pop();
            self.redo.push(h)
        }
        Ok(())
    }
    pub fn backup_now(&mut self) -> Result<PathBuf> {
        if self.protected {
            return Err("Unreadable libraries cannot be verified backups".into());
        }
        if !self.path.exists() {
            self.persist(&self.data)?
        }
        self.checkpoint("manual")
    }
    pub fn restore_preview(path: &Path) -> Result<Backup> {
        let b: Backup = serde_json::from_slice(&read_bounded(path, 40 * 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
        if b.schema_version != 1 {
            return Err("Unsupported backup version".into());
        }
        b.library.validate()?;
        Ok(b)
    }
    pub fn restore(&mut self, b: &Backup) -> Result<()> {
        if b.schema_version != 1 {
            return Err("Unsupported backup version".into());
        }
        b.library.validate()?;
        if self.path.exists() {
            if self.protected {
                let p = self
                    .backup_dir()
                    .join(format!("unreadable-{}.json", Uuid::new_v4()));
                atomic_write(&p, &read_bounded(&self.path, 40 * 1024 * 1024)?)?
            } else {
                self.checkpoint("pre-restore")?;
            }
        }
        atomic_write(
            &self.path,
            &serde_json::to_vec_pretty(&b.library).map_err(|e| e.to_string())?,
        )?;
        self.data = b.library.clone();
        self.protected = false;
        self.error = None;
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }
}
fn diff(before: &Library, after: &Library) -> Vec<Patch> {
    let mut patches = vec![];
    for (i, e) in before.entries.iter().enumerate() {
        let a = after.entries.iter().find(|x| x.id == e.id);
        if a != Some(e) {
            patches.push(Patch {
                before: Some(e.clone()),
                after: a.cloned(),
                index: i,
            })
        }
    }
    for (i, e) in after.entries.iter().enumerate() {
        if !before.entries.iter().any(|x| x.id == e.id) {
            patches.push(Patch {
                before: None,
                after: Some(e.clone()),
                index: i,
            })
        }
    }
    patches
}
fn apply_patch(e: &mut Entry, a: &Entry, b: &Entry) -> Result<()> {
    macro_rules! field {
        ($f:ident) => {
            if a.$f != b.$f {
                if e.$f != a.$f {
                    return Err(format!("History conflict in {}", stringify!($f)));
                }
                e.$f = b.$f.clone();
            }
        };
    }
    field!(name);
    field!(location);
    field!(kind);
    field!(notes);
    field!(tags);
    field!(collection_id);
    field!(favorite);
    field!(added);
    field!(bookmark);
    Ok(())
}
