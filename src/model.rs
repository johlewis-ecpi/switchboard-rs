use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub const APPLE_EPOCH: f64 = 978_307_200.0;
pub fn now() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0 - APPLE_EPOCH
}
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Folder,
    Url,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: Uuid,
    pub name: String,
    pub location: String,
    pub kind: Kind,
    pub notes: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "collectionID")]
    pub collection_id: Option<Uuid>,
    pub favorite: bool,
    pub added: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
}
impl Entry {
    pub fn new(name: String, location: String, kind: Kind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            location,
            kind,
            notes: String::new(),
            tags: vec![],
            collection_id: None,
            favorite: false,
            added: now(),
            bookmark: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentID")]
    pub parent_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TagMatch {
    #[default]
    Any,
    All,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagConstraint {
    #[serde(rename = "tagIDs")]
    pub tag_ids: BTreeSet<Uuid>,
    pub tag_match: TagMatch,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Added {
    #[default]
    Anytime,
    Today,
    ThisWeek,
    LastSevenDays,
    LastThirtyDays,
    Range,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Predicate {
    pub queries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "collectionID")]
    pub collection_id: Option<Uuid>,
    pub file_extension: String,
    #[serde(rename = "tagIDs")]
    pub tag_ids: BTreeSet<Uuid>,
    pub tag_match: TagMatch,
    pub additional_tags: Vec<TagConstraint>,
    pub added: Added,
    pub start_date: f64,
    pub end_date: f64,
}
impl Default for Predicate {
    fn default() -> Self {
        Self {
            queries: vec![String::new()],
            kind: None,
            favorite: None,
            collection_id: None,
            file_extension: String::new(),
            tag_ids: BTreeSet::new(),
            tag_match: TagMatch::Any,
            additional_tags: vec![],
            added: Added::Anytime,
            start_date: now(),
            end_date: now(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    #[default]
    Name,
    Newest,
    Kind,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum View {
    Thumbnails,
    SmallCards,
    #[default]
    MediumCards,
    LargeCards,
    CompactList,
    ComfortableList,
    Table,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedFilter {
    pub id: Uuid,
    pub name: String,
    pub criteria: Predicate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Sort>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    AdditionsAndManual,
    ManualOnly,
    AdditionsOnly,
}
// Swift synthesized associated-value enum representation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    AddTag {
        #[serde(rename = "_0")]
        id: Uuid,
    },
    RemoveTag {
        #[serde(rename = "_0")]
        id: Uuid,
    },
    SetCollection {
        #[serde(rename = "_0", default, skip_serializing_if = "Option::is_none")]
        id: Option<Uuid>,
    },
    Favorite {
        #[serde(rename = "_0")]
        value: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub predicate: Predicate,
    pub name_contains: String,
    pub location_contains: String,
    pub url_host: String,
    pub folder_path: String,
    pub actions: Vec<Action>,
    pub stop_processing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub entries: Vec<Entry>,
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub saved_filters: Vec<SavedFilter>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub view: View,
    pub configurations: BTreeMap<String, serde_json::Value>,
    pub sort: Sort,
    pub appearance: Appearance,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            entries: vec![],
            collections: vec![],
            tags: vec![],
            saved_filters: vec![],
            rules: vec![],
            view: View::default(),
            configurations: BTreeMap::new(),
            sort: Sort::Name,
            appearance: Appearance::System,
        }
    }
}
impl Library {
    pub fn normalize_tags(&mut self) {
        let old = std::mem::take(&mut self.tags);
        for mut t in old {
            t.name = t.name.trim().to_owned();
            if valid_tag(&t.name)
                && !self
                    .tags
                    .iter()
                    .any(|x| x.name.to_lowercase() == t.name.to_lowercase())
            {
                self.tags.push(t)
            }
        }
        for e in &mut self.entries {
            let mut names = vec![];
            for raw in &e.tags {
                let name = raw.trim();
                if !valid_tag(name) {
                    continue;
                }
                let canonical = if let Some(t) = self
                    .tags
                    .iter()
                    .find(|t| t.name.to_lowercase() == name.to_lowercase())
                {
                    t.name.clone()
                } else {
                    self.tags.push(Tag {
                        id: Uuid::new_v4(),
                        name: name.into(),
                        icon: None,
                        palette: None,
                    });
                    name.into()
                };
                if !names.contains(&canonical) {
                    names.push(canonical)
                }
            }
            e.tags = names;
        }
    }
    pub fn collection_path(&self, id: Uuid) -> String {
        let mut parts = vec![];
        let mut at = Some(id);
        let mut seen = BTreeSet::new();
        while let Some(id) = at {
            if !seen.insert(id) {
                break;
            }
            if let Some(c) = self.collections.iter().find(|c| c.id == id) {
                parts.push(c.name.clone());
                at = c.parent_id
            } else {
                break;
            }
        }
        parts.reverse();
        parts.join(" / ")
    }
    pub fn descendant(&self, mut id: Uuid, parent: Uuid) -> bool {
        let mut seen = BTreeSet::new();
        loop {
            if id == parent {
                return true;
            }
            if !seen.insert(id) {
                return false;
            }
            match self
                .collections
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.parent_id)
            {
                Some(p) => id = p,
                None => return false,
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        fn unique(ids: impl Iterator<Item = Uuid>) -> Result<()> {
            let mut seen = BTreeSet::new();
            for id in ids {
                if !seen.insert(id) {
                    return Err("Duplicate UUID".into());
                }
            }
            Ok(())
        }
        unique(self.entries.iter().map(|e| e.id))?;
        unique(self.collections.iter().map(|e| e.id))?;
        unique(self.tags.iter().map(|e| e.id))?;
        unique(self.saved_filters.iter().map(|e| e.id))?;
        unique(self.rules.iter().map(|e| e.id))?;
        let cs: BTreeSet<_> = self.collections.iter().map(|c| c.id).collect();
        for e in &self.entries {
            if e.name.trim().is_empty()
                || e.location.is_empty()
                || !e.added.is_finite()
                || (e.added + APPLE_EPOCH).abs() >= 253402300800.0
            {
                return Err("Invalid entry name, location or timestamp".into());
            }
            if e.collection_id.is_some_and(|id| !cs.contains(&id)) {
                return Err("Entry references an unavailable collection".into());
            }
        }
        for c in &self.collections {
            let mut at = Some(c.id);
            let mut seen = BTreeSet::new();
            while let Some(id) = at {
                if !seen.insert(id) {
                    return Err("Collection cycle".into());
                }
                let x = self
                    .collections
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or("Missing collection parent")?;
                at = x.parent_id;
            }
        }
        let mut names = BTreeSet::new();
        for t in &self.tags {
            if !valid_tag(&t.name) || !names.insert(t.name.to_lowercase()) {
                return Err("Invalid or duplicate tag name".into());
            }
        }
        Ok(())
    }
}
pub fn valid_tag(name: &str) -> bool {
    !name.trim().is_empty() && !name.contains(',')
}
pub fn web_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let s = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let u = url::Url::parse(&s).map_err(|_| "Invalid web address")?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none_or(|h| h.is_empty() || h.contains(' '))
    {
        return Err("Use an HTTP or HTTPS address with a host".into());
    }
    Ok(u.to_string())
}
pub fn standard_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c),
        }
    }
    out
}
pub fn address_key(e: &Entry) -> String {
    if e.kind != Kind::Url {
        return format!(
            "local:{}",
            standard_path(std::path::Path::new(&e.location)).display()
        );
    }
    let Ok(mut u) = url::Url::parse(&e.location) else {
        return format!("url:{}", e.location);
    };
    if matches!(
        (u.scheme(), u.port()),
        ("http", Some(80)) | ("https", Some(443))
    ) {
        let _ = u.set_port(None);
    }
    format!("url:{u}")
}
