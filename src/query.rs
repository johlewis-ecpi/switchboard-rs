use crate::model::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use uuid::Uuid;
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}
pub fn metadata(e: &Entry, q: &str) -> bool {
    let q = fold(q.trim());
    q.is_empty()
        || [&e.name, &e.location, &e.notes, &e.tags.join(" ")]
            .iter()
            .any(|s| fold(s).contains(&q))
}
pub fn tags_match(e: &Entry, ids: &BTreeSet<Uuid>, mode: TagMatch, l: &Library) -> bool {
    if ids.is_empty() {
        return true;
    }
    let names: Vec<_> = ids
        .iter()
        .filter_map(|id| l.tags.iter().find(|t| t.id == *id).map(|t| &t.name))
        .collect();
    if names.len() != ids.len() {
        return false;
    }
    match mode {
        TagMatch::Any => names.iter().any(|n| e.tags.contains(n)),
        TagMatch::All => names.iter().all(|n| e.tags.contains(n)),
    }
}
fn local_day(t: f64) -> Option<NaiveDate> {
    DateTime::from_timestamp((t + APPLE_EPOCH) as i64, 0)
        .map(|d| d.with_timezone(&Local).date_naive())
}
pub fn predicate(e: &Entry, p: &Predicate, l: &Library) -> bool {
    predicate_on(e, p, l, Local::now().date_naive())
}
pub fn predicate_on(e: &Entry, p: &Predicate, l: &Library, today: NaiveDate) -> bool {
    if !p.queries.iter().all(|q| metadata(e, q))
        || p.kind.is_some_and(|k| k != e.kind)
        || p.favorite.is_some_and(|f| f != e.favorite)
    {
        return false;
    }
    if let Some(id) = p.collection_id
        && (!l.collections.iter().any(|c| c.id == id)
            || !e.collection_id.is_some_and(|c| l.descendant(c, id)))
    {
        return false;
    }
    let ext = p.file_extension.trim().trim_matches('.');
    if !ext.is_empty() {
        let path = if e.kind == Kind::Url {
            url::Url::parse(&e.location)
                .ok()
                .map(|u| u.path().to_owned())
                .unwrap_or_default()
        } else {
            e.location.clone()
        };
        if !std::path::Path::new(&path)
            .extension()
            .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case(ext))
        {
            return false;
        }
    }
    if !tags_match(e, &p.tag_ids, p.tag_match, l)
        || !p
            .additional_tags
            .iter()
            .all(|g| tags_match(e, &g.tag_ids, g.tag_match, l))
    {
        return false;
    }
    if p.added == Added::Anytime {
        return true;
    }
    let Some(day) = local_day(e.added) else {
        return false;
    };
    let (start, end) = match p.added {
        Added::Today => (today, today),
        Added::ThisWeek => {
            #[cfg(target_os = "macos")]
            let first =
                (objc2_foundation::NSCalendar::currentCalendar().firstWeekday() as u32 + 6) % 7;
            #[cfg(not(target_os = "macos"))]
            let first = 1;
            let delta = (today.weekday().num_days_from_sunday() + 7 - first) % 7;
            let d = today - Duration::days(delta as i64);
            (d, d + Duration::days(6))
        }
        Added::LastSevenDays => (today - Duration::days(6), today),
        Added::LastThirtyDays => (today - Duration::days(29), today),
        Added::Range => {
            let (Some(a), Some(b)) = (local_day(p.start_date), local_day(p.end_date)) else {
                return false;
            };
            (a, b)
        }
        Added::Anytime => unreachable!(),
    };
    day >= start && day <= end
}
// Numeric runs compare by value, so “Item 2” sorts before “Item 10”.
#[cfg(target_os = "macos")]
pub fn natural(a: &str, b: &str) -> Ordering {
    use objc2_foundation::{NSComparisonResult, NSString};
    match NSString::from_str(a).localizedStandardCompare(&NSString::from_str(b)) {
        NSComparisonResult::Ascending => Ordering::Less,
        NSComparisonResult::Descending => Ordering::Greater,
        _ => Ordering::Equal,
    }
}
#[cfg(not(target_os = "macos"))]
pub fn natural(a: &str, b: &str) -> Ordering {
    let a = fold(a);
    let b = fold(b);
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek(), b.peek()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let x: String = std::iter::from_fn(|| a.next_if(|c| c.is_ascii_digit())).collect();
                let y: String = std::iter::from_fn(|| b.next_if(|c| c.is_ascii_digit())).collect();
                let x = x.trim_start_matches('0');
                let y = y.trim_start_matches('0');
                let c = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if c != Ordering::Equal {
                    return c;
                }
            }
            _ => {
                let c = a.next().cmp(&b.next());
                if c != Ordering::Equal {
                    return c;
                }
            }
        }
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenActivity {
    pub count: u64,
    pub last_opened: f64,
}

pub fn rank_activity(
    entries: &mut Vec<&Entry>,
    scope: &str,
    activity: &BTreeMap<Uuid, OpenActivity>,
) {
    if !matches!(scope, "recent" | "frequent") {
        return;
    }
    entries.retain(|e| activity.get(&e.id).is_some_and(|a| a.count > 0));
    entries.sort_by(|a, b| {
        let a_stats = &activity[&a.id];
        let b_stats = &activity[&b.id];
        let count = if scope == "frequent" {
            b_stats.count.cmp(&a_stats.count)
        } else {
            Ordering::Equal
        };
        count
            .then_with(|| b_stats.last_opened.total_cmp(&a_stats.last_opened))
            .then_with(|| natural(&a.name, &b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}
fn default_minimal_icon_size() -> u32 {
    80
}
fn default_minimal_wrap() -> bool {
    true
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub scope: String,
    pub query: String,
    pub selected: BTreeSet<Uuid>,
    pub selected_tags: BTreeSet<Uuid>,
    pub tag_match: TagMatch,
    pub inspector: bool,
    pub sidebar: bool,
    pub minimal: bool,
    pub bloom: bool,
    pub rows: usize,
    #[serde(default = "default_minimal_icon_size")]
    pub minimal_icon_size: u32,
    #[serde(default = "default_minimal_wrap")]
    pub minimal_wrap: bool,
    pub order: Vec<Uuid>,
    pub hierarchical: bool,
    pub show_collection_names: bool,
    pub single_click: bool,
    pub search_mode: String,
    pub launcher_enabled: bool,
    pub launcher_key: String,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            scope: "all".into(),
            query: String::new(),
            selected: BTreeSet::new(),
            selected_tags: BTreeSet::new(),
            tag_match: TagMatch::Any,
            inspector: true,
            sidebar: true,
            minimal: false,
            bloom: true,
            rows: 1,
            minimal_icon_size: default_minimal_icon_size(),
            minimal_wrap: true,
            order: vec![],
            hierarchical: false,
            show_collection_names: true,
            single_click: false,
            search_mode: "metadata".into(),
            launcher_enabled: true,
            launcher_key: "Control+Alt+Space".into(),
        }
    }
}
impl WindowState {
    pub fn validate(&mut self, l: &Library) {
        if !matches!(
            self.scope.as_str(),
            "all" | "favorites" | "recent" | "frequent" | "file" | "folder" | "url"
        ) && !Uuid::parse_str(&self.scope)
            .ok()
            .is_some_and(|id| l.collections.iter().any(|c| c.id == id))
            && !self
                .scope
                .strip_prefix("smart:")
                .and_then(|s| Uuid::parse_str(s).ok())
                .is_some_and(|id| l.saved_filters.iter().any(|s| s.id == id))
        {
            self.scope = "all".into()
        }
        self.selected
            .retain(|id| l.entries.iter().any(|e| e.id == *id));
        self.selected_tags
            .retain(|id| l.tags.iter().any(|t| t.id == *id));
        self.rows = self.rows.clamp(1, 8);
        self.minimal_icon_size = self.minimal_icon_size.clamp(48, 160);
        let mut seen = BTreeSet::new();
        self.order.retain(|id| seen.insert(*id));
    }
}
pub fn scope_match(e: &Entry, l: &Library, w: &WindowState) -> bool {
    let is_query = !w.query.trim().is_empty() || !w.selected_tags.is_empty();
    match w.scope.as_str() {
        "all" => !w.hierarchical || is_query || e.collection_id.is_none(),
        "favorites" => e.favorite,
        "recent" | "frequent" => true,
        "file" => e.kind == Kind::File,
        "folder" => e.kind == Kind::Folder,
        "url" => e.kind == Kind::Url,
        s if s.starts_with("smart:") => Uuid::parse_str(&s[6..])
            .ok()
            .and_then(|id| l.saved_filters.iter().find(|f| f.id == id))
            .is_some_and(|f| predicate(e, &f.criteria, l)),
        s => Uuid::parse_str(s).ok().is_some_and(|id| {
            l.collections.iter().any(|c| c.id == id)
                && e.collection_id.is_some_and(|c| {
                    if w.hierarchical && !is_query {
                        c == id
                    } else {
                        l.descendant(c, id)
                    }
                })
        }),
    }
}
pub fn candidates<'a>(l: &'a Library, w: &WindowState) -> Vec<&'a Entry> {
    let mut entries: Vec<_> = l
        .entries
        .iter()
        .filter(|e| scope_match(e, l, w) && tags_match(e, &w.selected_tags, w.tag_match, l))
        .collect();
    entries.sort_by(|a, b| match l.sort {
        Sort::Name => natural(&a.name, &b.name),
        Sort::Newest => b.added.total_cmp(&a.added),
        Sort::Kind => a.kind.cmp(&b.kind).then_with(|| natural(&a.name, &b.name)),
    });
    entries
}
pub fn visible<'a>(l: &'a Library, w: &WindowState) -> Vec<&'a Entry> {
    candidates(l, w)
        .into_iter()
        .filter(|e| metadata(e, &w.query))
        .collect()
}
pub fn launcher<'a>(l: &'a Library, q: &str) -> Vec<&'a Entry> {
    let q = fold(q.trim());
    let mut ranked: Vec<_> = l
        .entries
        .iter()
        .filter_map(|e| {
            let n = fold(&e.name);
            let rank = if q.is_empty() || n == q {
                0
            } else if n.starts_with(&q) {
                1
            } else if n.contains(&q) {
                2
            } else if metadata(e, &q) {
                3
            } else {
                return None;
            };
            Some((rank, e))
        })
        .collect();
    ranked.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then_with(|| natural(&a.name, &b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    ranked.into_iter().take(50).map(|(_, e)| e).collect()
}
pub fn ordered(ids: &[Uuid], saved: &[Uuid]) -> Vec<Uuid> {
    let set: BTreeSet<_> = ids.iter().copied().collect();
    let mut seen = BTreeSet::new();
    saved
        .iter()
        .chain(ids)
        .filter(|id| set.contains(id) && seen.insert(**id))
        .copied()
        .collect()
}
pub fn reorder(all: &[Uuid], visible: &[Uuid], moving: Uuid, target: Uuid) -> Vec<Uuid> {
    let mut v = visible.to_vec();
    let (Some(i), Some(j)) = (
        v.iter().position(|id| *id == moving),
        v.iter().position(|id| *id == target),
    ) else {
        return all.to_vec();
    };
    v.remove(i);
    v.insert(j, moving);
    let set: BTreeSet<_> = visible.iter().collect();
    let mut it = v.into_iter();
    all.iter()
        .map(|id| {
            if set.contains(id) {
                it.next().unwrap_or(*id)
            } else {
                *id
            }
        })
        .collect()
}
pub fn day_timestamp(s: &str) -> Result<f64> {
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let t = Local
        .from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        .earliest()
        .ok_or("Invalid local date")?;
    Ok(t.timestamp() as f64 - APPLE_EPOCH)
}
