use crate::{
    model::*,
    store::{Store, atomic_write, read_bounded},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Portable {
    pub schema_version: u32,
    pub entries: Vec<Entry>,
    pub collections: Vec<Collection>,
    pub tags: Vec<Tag>,
    pub saved_filters: Vec<SavedFilter>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}
impl Portable {
    pub fn from_library(l: &Library) -> Self {
        let mut entries = l.entries.clone();
        for e in &mut entries {
            e.bookmark = None
        }
        Self {
            schema_version: 1,
            entries,
            collections: l.collections.clone(),
            tags: l.tags.clone(),
            saved_filters: l.saved_filters.clone(),
            rules: l.rules.clone(),
        }
    }
    pub fn library(&self) -> Library {
        Library {
            entries: self.entries.clone(),
            collections: self.collections.clone(),
            tags: self.tags.clone(),
            saved_filters: self.saved_filters.clone(),
            rules: self.rules.clone(),
            ..Library::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err("Unsupported portable schema version".into());
        }
        if self.entries.len() > 10000
            || [
                self.collections.len(),
                self.tags.len(),
                self.saved_filters.len(),
                self.rules.len(),
            ]
            .into_iter()
            .any(|n| n > 5000)
        {
            return Err("Portable library exceeds item limits".into());
        }
        self.library().validate()?;
        for e in &self.entries {
            if e.kind == Kind::Url {
                web_url(&e.location)?;
            } else if !Path::new(&e.location).is_absolute() || e.location.contains('\0') {
                return Err("Imported local paths must be absolute and contain no NUL".into());
            }
        }
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self> {
        let b = read_bounded(path, 10 * 1024 * 1024)?;
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("html") || s.eq_ignore_ascii_case("htm"))
        {
            return parse_html(std::str::from_utf8(&b).map_err(|_| "Bookmark HTML must be UTF-8")?);
        }
        let v: serde_json::Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        let mut p = if v.get("schemaVersion").is_some() {
            serde_json::from_value::<Self>(v).map_err(|e| e.to_string())?
        } else {
            Self::from_library(&serde_json::from_value::<Library>(v).map_err(|e| e.to_string())?)
        };
        p.validate()?;
        let mut l = p.library();
        l.normalize_tags();
        p.tags = l.tags;
        p.entries = l.entries;
        for e in &mut p.entries {
            e.bookmark = None
        }
        p.validate()?;
        Ok(p)
    }
}
pub fn fingerprint(l: &Library) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::to_value(l).expect("library encoding"))
                .expect("library encoding")
        )
    )
}
#[derive(Clone, Debug, Serialize)]
pub struct ImportPreview {
    pub fingerprint: String,
    pub candidate: Library,
    pub added: usize,
    pub skipped: usize,
    pub remapped: usize,
    pub retained_appearance: usize,
    pub process_rules: bool,
}
fn remap(id: Uuid, map: &mut BTreeMap<Uuid, Uuid>) -> Uuid {
    *map.entry(id).or_insert_with(Uuid::new_v4)
}
fn map_predicate(p: &mut Predicate, t: &mut BTreeMap<Uuid, Uuid>, c: &mut BTreeMap<Uuid, Uuid>) {
    p.tag_ids = p.tag_ids.iter().map(|id| remap(*id, t)).collect();
    for g in &mut p.additional_tags {
        g.tag_ids = g.tag_ids.iter().map(|id| remap(*id, t)).collect()
    }
    p.collection_id = p.collection_id.map(|id| remap(id, c));
}
fn unique_name(base: &str, names: &mut BTreeSet<String>) -> String {
    let mut s = base.to_owned();
    let mut n = 2;
    while !names.insert(s.to_lowercase()) {
        s = format!("{base} imported {n}");
        n += 1
    }
    s
}
pub fn preview(
    l: &Library,
    p: &Portable,
    keep_both: bool,
    process_rules: bool,
) -> Result<ImportPreview> {
    p.validate()?;
    let mut next = l.clone();
    let mut tm = BTreeMap::new();
    let mut cm = BTreeMap::new();
    let mut remapped = 0;
    let mut retained = 0;
    for tag in &p.tags {
        if let Some(t) = next
            .tags
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(&tag.name))
        {
            tm.insert(tag.id, t.id);
            retained += 1;
        } else {
            let mut t = tag.clone();
            if next.tags.iter().any(|x| x.id == t.id) {
                t.id = Uuid::new_v4();
                remapped += 1
            }
            tm.insert(tag.id, t.id);
            next.tags.push(t)
        }
    }
    for c in &p.collections {
        let id = if next.collections.iter().any(|x| x.id == c.id) {
            remapped += 1;
            Uuid::new_v4()
        } else {
            c.id
        };
        cm.insert(c.id, id);
    }
    for c in &p.collections {
        let mut c = c.clone();
        c.id = cm[&c.id];
        c.parent_id = c.parent_id.map(|id| remap(id, &mut cm));
        next.collections.push(c)
    }
    let mut keys: BTreeSet<_> = l.entries.iter().map(address_key).collect();
    let mut added_ids = vec![];
    let mut skipped = 0;
    for e in &p.entries {
        let key = address_key(e);
        if !keep_both && !keys.insert(key) {
            skipped += 1;
            continue;
        }
        let mut e = e.clone();
        if next.entries.iter().any(|x| x.id == e.id) {
            e.id = Uuid::new_v4();
            remapped += 1
        }
        e.collection_id = e.collection_id.map(|id| remap(id, &mut cm));
        e.bookmark = None;
        for n in &mut e.tags {
            if let Some(t) = next.tags.iter().find(|t| t.name.eq_ignore_ascii_case(n)) {
                *n = t.name.clone()
            }
        }
        added_ids.push(e.id);
        next.entries.push(e)
    }
    let mut names: BTreeSet<_> = next
        .saved_filters
        .iter()
        .map(|s| s.name.to_lowercase())
        .collect();
    for s in &p.saved_filters {
        let mut s = s.clone();
        if next.saved_filters.iter().any(|x| x.id == s.id) {
            s.id = Uuid::new_v4();
            remapped += 1
        }
        s.name = unique_name(&s.name, &mut names);
        map_predicate(&mut s.criteria, &mut tm, &mut cm);
        next.saved_filters.push(s)
    }
    if process_rules {
        crate::rules::apply(&mut next, &added_ids, true);
    }
    let mut names: BTreeSet<_> = next.rules.iter().map(|r| r.name.to_lowercase()).collect();
    for r in &p.rules {
        let mut r = r.clone();
        if next.rules.iter().any(|x| x.id == r.id) {
            r.id = Uuid::new_v4();
            remapped += 1
        }
        r.name = unique_name(&r.name, &mut names);
        r.enabled = false;
        map_predicate(&mut r.predicate, &mut tm, &mut cm);
        for a in &mut r.actions {
            match a {
                Action::AddTag { id } | Action::RemoveTag { id } => *id = remap(*id, &mut tm),
                Action::SetCollection { id } => *id = id.map(|id| remap(id, &mut cm)),
                _ => {}
            }
        }
        next.rules.push(r)
    }
    next.normalize_tags();
    next.validate()?;
    Ok(ImportPreview {
        fingerprint: fingerprint(l),
        candidate: next,
        added: added_ids.len(),
        skipped,
        remapped,
        retained_appearance: retained,
        process_rules,
    })
}
pub fn apply(s: &mut Store, p: &ImportPreview) -> Result<()> {
    if fingerprint(&s.data) != p.fingerprint {
        return Err("Library changed. Generate a new import preview".into());
    }
    p.candidate.validate()?;
    if s.path.exists() {
        s.checkpoint("pre-import")?;
    }
    s.commit(p.candidate.clone(), "Import", false)?;
    s.undo.clear();
    s.redo.clear();
    Ok(())
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn csv(l: &Library) -> String {
    let mut lines = vec![
        [
            "Name",
            "Kind",
            "Location",
            "Notes",
            "Tags",
            "Collection",
            "Favorite",
            "Added",
        ]
        .map(quote)
        .join(","),
    ];
    for e in &l.entries {
        let date = chrono::DateTime::from_timestamp((e.added + APPLE_EPOCH) as i64, 0)
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default();
        let kind = match e.kind {
            Kind::File => "file",
            Kind::Folder => "folder",
            Kind::Url => "url",
        };
        lines.push(
            [
                e.name.clone(),
                kind.into(),
                e.location.clone(),
                e.notes.clone(),
                e.tags.join(", "),
                e.collection_id
                    .map(|id| l.collection_path(id))
                    .unwrap_or_default(),
                e.favorite.to_string(),
                date,
            ]
            .map(|s| quote(&s))
            .join(","),
        );
    }
    lines.join("\r\n") + "\r\n"
}
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub fn entities(s: &str) -> String {
    let re = regex::Regex::new(r"&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos|nbsp);").unwrap();
    re.replace_all(s, |c: &regex::Captures| match &c[1] {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        "nbsp" => " ".into(),
        n => {
            let v = if let Some(x) = n.strip_prefix("#x") {
                u32::from_str_radix(x, 16).ok()
            } else {
                n.strip_prefix('#').and_then(|x| x.parse().ok())
            };
            v.and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| c[0].into())
        }
    })
    .into_owned()
}
pub fn html(l: &Library) -> String {
    fn group(l: &Library, parent: Option<Uuid>, out: &mut String) {
        out.push_str("<DL><p>\n");
        for c in l.collections.iter().filter(|c| c.parent_id == parent) {
            out.push_str(&format!("<DT><H3>{}</H3>\n", escape(&c.name)));
            group(l, Some(c.id), out);
        }
        for e in l
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Url && e.collection_id == parent)
        {
            out.push_str(&format!("<DT><A HREF=\"{}\" ADD_DATE=\"{}\" TAGS=\"{}\" SWITCHBOARD_FAVORITE=\"{}\">{}</A>\n",escape(&e.location),(e.added+APPLE_EPOCH) as i64,escape(&e.tags.join(",")),e.favorite,escape(&e.name)));
            if !e.notes.is_empty() {
                out.push_str(&format!("<DD>{}\n", escape(&e.notes)))
            }
        }
        out.push_str("</DL><p>\n");
    }
    let mut out="<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Switchboard</TITLE>\n<H1>Switchboard</H1>\n".into();
    group(l, None, &mut out);
    out
}
pub fn clean_html(s: &str) -> String {
    let re = regex::Regex::new(
        r"(?is)<!--.*?-->|<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>",
    )
    .unwrap();
    re.replace_all(s, "").into_owned()
}
pub fn parse_html(s: &str) -> Result<Portable> {
    let s = clean_html(s);
    let tokens=regex::Regex::new(r"(?is)<H3\b[^>]*>(.*?)</H3\s*>|<DL\b[^>]*>|</DL\s*>|<A\b([^>]*)>(.*?)</A\s*>|<DD\b[^>]*>([^<]*)").unwrap();
    let attrs =
        regex::Regex::new(r#"(?i)([\w_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap();
    let strip = regex::Regex::new(r"<[^>]*>").unwrap();
    let mut l = Library::default();
    let mut stack = vec![];
    let mut parent = None;
    let mut pending = None;
    for c in tokens.captures_iter(&s) {
        if let Some(name) = c.get(1) {
            let id = Uuid::new_v4();
            l.collections.push(Collection {
                id,
                name: entities(&strip.replace_all(name.as_str(), "")),
                parent_id: parent,
                icon: None,
                palette: None,
            });
            pending = Some(id);
        } else if c[0].to_ascii_lowercase().starts_with("<dl") {
            stack.push(parent);
            if let Some(id) = pending.take() {
                parent = Some(id)
            }
        } else if c[0].to_ascii_lowercase().starts_with("</dl") {
            parent = stack.pop().unwrap_or(None);
        } else if let Some(a) = c.get(2) {
            let mut m = BTreeMap::new();
            for a in attrs.captures_iter(a.as_str()) {
                m.insert(
                    a[1].to_lowercase(),
                    entities(
                        a.get(2)
                            .or_else(|| a.get(3))
                            .or_else(|| a.get(4))
                            .map_or("", |s| s.as_str()),
                    ),
                );
            }
            let Some(location) = m.get("href").and_then(|s| web_url(s).ok()) else {
                continue;
            };
            let mut e = Entry::new(entities(&strip.replace_all(&c[3], "")), location, Kind::Url);
            if e.name.trim().is_empty() {
                e.name = e.location.clone()
            }
            e.collection_id = parent;
            e.tags = m
                .get("tags")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            e.favorite = m
                .get("switchboard_favorite")
                .is_some_and(|s| s == "true" || s == "1");
            if let Some(date) = m.get("add_date").and_then(|s| s.parse::<f64>().ok()) {
                e.added = date - APPLE_EPOCH;
            }
            l.entries.push(e);
        } else if let Some(note) = c.get(4)
            && let Some(e) = l.entries.last_mut()
        {
            e.notes = entities(note.as_str()).trim().into();
        }
        if l.entries.len() > 10000 || l.collections.len() > 5000 {
            return Err("Bookmark HTML exceeds item limits".into());
        }
    }
    if l.entries.is_empty() && l.collections.is_empty() {
        return Err("No supported bookmarks or folders".into());
    }
    l.normalize_tags();
    let p = Portable::from_library(&l);
    p.validate()?;
    Ok(p)
}
pub fn export(path: &Path, l: &Library, format: &str) -> Result<()> {
    l.validate()?;
    let bytes = match format {
        "csv" => csv(l).into_bytes(),
        "html" => html(l).into_bytes(),
        _ => serde_json::to_vec_pretty(&Portable::from_library(l)).map_err(|e| e.to_string())?,
    };
    atomic_write(path, &bytes)
}
