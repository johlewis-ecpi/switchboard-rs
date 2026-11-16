use crate::{
    model::*,
    query::{fold, predicate},
};
use std::path::Path;
use uuid::Uuid;
pub fn valid_rule(r: &Rule, l: &Library) -> bool {
    !r.name.trim().is_empty()
        && !r.actions.is_empty()
        && r.predicate
            .tag_ids
            .iter()
            .chain(
                r.predicate
                    .additional_tags
                    .iter()
                    .flat_map(|g| g.tag_ids.iter()),
            )
            .all(|id| l.tags.iter().any(|t| t.id == *id))
        && r.predicate
            .collection_id
            .is_none_or(|id| l.collections.iter().any(|c| c.id == id))
        && r.actions.iter().all(|a| match a {
            Action::AddTag { id } | Action::RemoveTag { id } => l.tags.iter().any(|t| t.id == *id),
            Action::SetCollection { id } => {
                id.is_none_or(|id| l.collections.iter().any(|c| c.id == id))
            }
            _ => true,
        })
        && (r.folder_path.is_empty() || Path::new(&r.folder_path).is_absolute())
        && (r.url_host.is_empty()
            || (!r.url_host.contains(['/', ':', ' ', '@'])
                && url::Host::parse(&r.url_host).is_ok()))
}
pub fn disable_invalid(l: &mut Library) {
    let invalid: Vec<_> = l
        .rules
        .iter()
        .filter(|r| !valid_rule(r, l))
        .map(|r| r.id)
        .collect();
    for r in &mut l.rules {
        if invalid.contains(&r.id) {
            r.enabled = false
        }
    }
}
pub fn matches(e: &Entry, r: &Rule, l: &Library) -> bool {
    predicate(e, &r.predicate, l)
        && fold(&e.name).contains(&fold(r.name_contains.trim()))
        && fold(&e.location).contains(&fold(r.location_contains.trim()))
        && (r.url_host.is_empty()
            || (e.kind == Kind::Url
                && url::Url::parse(&e.location)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .is_some_and(|h| h.eq_ignore_ascii_case(&r.url_host))))
        && (r.folder_path.is_empty()
            || (e.kind != Kind::Url
                && standard_path(Path::new(&e.location))
                    .starts_with(standard_path(Path::new(&r.folder_path)))))
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct RuleChange {
    pub before: Entry,
    pub after: Entry,
    pub rules: Vec<String>,
    pub overrides: Vec<String>,
}
pub fn apply(l: &mut Library, ids: &[Uuid], addition: bool) -> Vec<RuleChange> {
    let snapshot = l.clone();
    let mut changes = vec![];
    for e in &mut l.entries {
        if !ids.contains(&e.id) {
            continue;
        }
        let before = e.clone();
        let mut names = vec![];
        let mut overrides = vec![];
        let mut writers = std::collections::BTreeMap::new();
        for r in &snapshot.rules {
            if !r.enabled
                || !valid_rule(r, &snapshot)
                || matches!(
                    (addition, r.trigger),
                    (true, Trigger::ManualOnly) | (false, Trigger::AdditionsOnly)
                )
                || !matches(e, r, &snapshot)
            {
                continue;
            }
            names.push(r.name.clone());
            for a in &r.actions {
                let key = match a {
                    Action::AddTag { id } | Action::RemoveTag { id } => format!("tag:{id}"),
                    Action::SetCollection { .. } => "collection".into(),
                    Action::Favorite { .. } => "favorite".into(),
                };
                let old = e.clone();
                match a {
                    Action::AddTag { id } => {
                        let t = &snapshot.tags.iter().find(|t| t.id == *id).unwrap().name;
                        if !e.tags.contains(t) {
                            e.tags.push(t.clone())
                        }
                    }
                    Action::RemoveTag { id } => {
                        let t = &snapshot.tags.iter().find(|t| t.id == *id).unwrap().name;
                        e.tags.retain(|n| n != t)
                    }
                    Action::SetCollection { id } => e.collection_id = *id,
                    Action::Favorite { value } => e.favorite = *value,
                }
                if *e != old
                    && let Some(previous) = writers.insert(key, r.name.clone())
                {
                    overrides.push(format!("{previous} overridden by {}", r.name))
                }
            }
            if r.stop_processing {
                break;
            }
        }
        if *e != before {
            changes.push(RuleChange {
                before,
                after: e.clone(),
                rules: names,
                overrides,
            })
        }
    }
    changes
}
