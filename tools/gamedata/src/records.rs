//! The records and lists in gamedata/gen3/records/: [record.NAME] (a schema's class,
//! optionally built on another record with `base`) and [list.NAME] (`of` = Int / Long,
//! `items` = numbers or Kotlin names).

use crate::config::check_entries;
use crate::schema::{is_ref_name, load_schema, Schema};
use crate::toml_raw::{read_toml, Entry, Value};
use crate::{fail, records_dir, relpath, GameDataError};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const LIST_TYPES: [(&str, &str); 2] = [("Int", "int"), ("Long", "long")];

/// A [config.x.field] sub-table: a record spelled out r#where a config sets it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordValue {
    pub record: String,
    pub class: String,
    pub base: Option<String>,
    pub entries: Vec<Entry>,
    pub end: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Record {
    pub name: String,
    pub kind: Kind,
    pub file: String,
    pub r#where: String,
    pub rank: (String, usize),
    pub lead: Vec<String>,
    pub end: Vec<String>,
    pub entries: Vec<Entry>,
    pub base: Option<String>,
    pub schema: Option<Schema>,
    pub of: Option<String>,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Record,
    List,
}

pub fn load_records(root: &Path, cache: &mut HashMap<String, Schema>) -> Result<HashMap<String, Record>, GameDataError> {
    let mut records: HashMap<String, Record> = HashMap::new();
    let dir = records_dir(root);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| fail("gamedata/gen3/records", e.to_string()))?
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().map_or(false, |e| e == "toml"))
        .collect();
    paths.sort();
    for path in paths {
        let rel = relpath(&path, root);
        let tables = read_toml(&path, root)?;
        if let Some(first) = tables[0].entries.first() {
            return Err(fail(&format!("{rel}:{}", first.4), "keys before the first [record.*] / [list.*]"));
        }
        for (i, tab) in tables[1..].iter().enumerate() {
            let r#where = format!("{rel}:{}", tab.line);
            let m = Regex::new(r"^(record|list)\.([A-Za-z0-9_]+)$").unwrap().captures(&tab.name);
            let (kind, name) = match &m {
                Some(m) => (m[1].to_string(), m[2].to_string()),
                None => return Err(fail(&r#where, format!("[{}] isn't [record.<UPPER_SNAKE>] or [list.<UPPER_SNAKE>]", tab.name))),
            };
            if !Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap().is_match(&name) {
                return Err(fail(&r#where, format!("[{}] isn't [record.<UPPER_SNAKE>] or [list.<UPPER_SNAKE>]", tab.name)));
            }
            if records.contains_key(&name) {
                return Err(fail(&r#where, format!("{name} is also in {}", records[&name].file)));
            }
            // `of` / `items` are a list's own keys; a record's `items` is a field.
            let keys: &[&str] = if kind == "list" { &["schema", "base", "of", "items"] } else { &["schema", "base"] };
            let meta: HashMap<String, Value> = tab.entries.iter().filter(|e| keys.contains(&e.0.as_str())).map(|e| (e.0.clone(), e.1.clone())).collect();
            let entries: Vec<Entry> = tab.entries.iter().filter(|e| !keys.contains(&e.0.as_str())).cloned().collect();
            let mut r = Record {
                name: name.clone(),
                kind: if kind == "list" { Kind::List } else { Kind::Record },
                file: rel.clone(),
                r#where: r#where.clone(),
                rank: (rel.clone(), i),
                lead: tab.lead.clone(),
                end: tab.end.clone(),
                entries,
                base: meta.get("base").and_then(|v| v.as_str().map(|s| s.to_string())),
                schema: None,
                of: None,
                items: vec![],
            };
            if kind == "list" {
                if !r.entries.is_empty()
                    || !LIST_TYPES.iter().any(|(n, _)| meta.get("of").and_then(|v| v.as_str()) == Some(*n))
                    || !matches!(meta.get("items"), Some(Value::List(_)))
                {
                    return Err(fail(&r#where, "a list has `of` (Int or Long) and `items`, nothing else"));
                }
                let items = meta.get("items").unwrap().as_list().unwrap().to_vec();
                for item in &items {
                    let ok = matches!(item, Value::RawInt(_)) || item.as_str().map_or(false, is_ref_name);
                    if !ok {
                        return Err(fail(&r#where, format!("list item {item:?} isn't a number or a Kotlin name")));
                    }
                }
                r.of = meta.get("of").and_then(|v| v.as_str()).map(|s| s.to_string());
                r.items = items;
            } else {
                let schema_name = meta.get("schema").and_then(|v| v.as_str())
                    .ok_or_else(|| fail(&r#where, "a record names its `schema`"))?;
                let s = load_schema(root, schema_name, cache)?;
                check_entries(&r.entries, &s, &rel)?;
                r.schema = Some(s);
            }
            records.insert(name, r);
        }
    }
    for r in records.values() {
        if r.kind != Kind::Record {
            continue;
        }
        let mut root = r;
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(base_name) = &root.base {
            if records.get(base_name).is_none() {
                break;
            }
            if !seen.insert(root.name.clone()) {
                return Err(fail(&r.r#where, "its base chain loops"));
            }
            let base = &records[base_name];
            if base.kind != Kind::Record || base.schema.as_ref().unwrap().name != r.schema.as_ref().unwrap().name {
                return Err(fail(&r.r#where, format!("base {} isn't a {} record", base_name, r.schema.as_ref().unwrap().class)));
            }
            root = base;
        }
        if root.base.is_none() {
            let set_fields: HashSet<&String> = root.entries.iter().map(|e| &e.0).collect();
            let schema = r.schema.as_ref().unwrap();
            let missing: Vec<&String> = schema.fields.values()
                .filter(|f| f.required && !set_fields.contains(&f.name))
                .map(|f| &f.name)
                .collect();
            if !missing.is_empty() {
                return Err(fail(&root.r#where, format!("has no base, so it must set {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))));
            }
        }
    }
    Ok(records)
}
