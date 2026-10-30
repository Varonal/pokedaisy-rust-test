//! The game configs in gamedata/gen3/: [config.<name>] tables with `base` / `private` /
//! `lazy` / `port` meta keys, checked against the schema, and [config.<name>.<field>]
//! sub-tables spelling out a record.

use crate::records::RecordValue;
use crate::schema::{is_ref_name, load_schema, Schema};
use crate::toml_raw::{read_toml, Entry, Table, Value};
use crate::{fail, gamedata, records_dir, relpath, GameDataError, GROUP_ORDER, META_KEYS, NONE, RETAIL_ORDER};
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Config {
    pub key: String,
    pub file: String,
    pub rank: ((usize, usize, String), usize),
    pub r#where: String,
    pub lead: Vec<String>,
    pub entries: Vec<Entry>,
    pub end: Vec<String>,
    pub base: Option<String>,
    pub private: bool,
    pub lazy: bool,
    pub port: Option<String>,
}

pub fn file_rank(root: &Path, path: &Path) -> (usize, usize, String) {
    let gen3 = gamedata(root).join("gen3");
    let rel = relpath(path, &gen3);
    let mut parts = rel.split('/');
    let group = parts.next().unwrap_or("");
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let g = GROUP_ORDER.iter().position(|g| *g == group).unwrap_or(GROUP_ORDER.len());
    let r = if group == "retail" { RETAIL_ORDER.iter().position(|n| *n == name).unwrap_or(RETAIL_ORDER.len()) } else { RETAIL_ORDER.len() };
    (g, r, rel)
}

fn config_paths(root: &Path) -> Result<Vec<PathBuf>, GameDataError> {
    let records = records_dir(root);
    let mut out = vec![];
    collect(root.join("gamedata").join("gen3"), &records, &mut out)?;
    out.sort_by_key(|p| (file_rank(root, p), p.clone()));
    Ok(out)
}

fn collect(dir: PathBuf, skip: &Path, out: &mut Vec<PathBuf>) -> Result<(), GameDataError> {
    let rd = std::fs::read_dir(&dir).map_err(|e| fail(&relpath(&dir, skip.parent().unwrap().parent().unwrap()), e.to_string()))?;
    for entry in rd {
        let path = entry.map_err(|e| fail("gamedata", e.to_string()))?.path();
        if path.is_dir() {
            if path != skip {
                collect(path, skip, out)?;
            }
        } else if path.extension().map_or(false, |e| e == "toml") && !path.starts_with(skip) {
            out.push(path);
        }
    }
    Ok(())
}

pub fn load_configs(root: &Path, schema: &Schema, cache: &mut HashMap<String, Schema>) -> Result<HashMap<String, Config>, GameDataError> {
    let mut configs: HashMap<String, Config> = HashMap::new();
    let paths = config_paths(root)?;
    for path in &paths {
        let rel = relpath(path, root);
        let tables = read_toml(path, root)?;
        if let Some(first) = tables[0].entries.first() {
            return Err(fail(&format!("{rel}:{}", first.4), "keys before the first [config.*]"));
        }
        for (i, tab) in tables[1..].iter().enumerate() {
            let r#where = format!("{rel}:{}", tab.line);
            let m = Regex::new(r"^config\.([a-z0-9_]+)((?:\.\w+)*)$").unwrap().captures(&tab.name)
                .ok_or_else(|| fail(&r#where, format!("[{}] isn't [config.<lower_snake_name>] or a record of one", tab.name)))?;
            let key = m[1].to_string();
            let path_parts: Vec<&str> = m[2].split('.').filter(|p| !p.is_empty()).collect();
            if !path_parts.is_empty() {
                let c = configs.get(&key).filter(|c| c.file == rel)
                    .ok_or_else(|| fail(&r#where, format!("[{}] must follow [config.{key}] in the same file", tab.name)))?;
                let mut c = Config {
                    key: c.key.clone(),
                    file: c.file.clone(),
                    rank: c.rank.clone(),
                    r#where: c.r#where.clone(),
                    lead: c.lead.clone(),
                    entries: c.entries.clone(),
                    end: c.end.clone(),
                    base: c.base.clone(),
                    private: c.private,
                    lazy: c.lazy,
                    port: c.port.clone(),
                };
                add_record(&mut c, &path_parts, tab, schema, root, &rel, cache)?;
                configs.insert(key, c);
                continue;
            }
            if configs.contains_key(&key) {
                let other = &configs[&key].file;
                return Err(fail(&r#where, format!("config {key} is also in {other}")));
            }
            let mut meta: BTreeMap<String, Value> = BTreeMap::new();
            let mut entries: Vec<Entry> = vec![];
            for e in &tab.entries {
                if META_KEYS.contains(&e.0.as_str()) {
                    meta.insert(e.0.clone(), e.1.clone());
                } else {
                    entries.push(e.clone());
                }
            }
            check_entries(&entries, schema, &rel)?;
            configs.insert(
                key.clone(),
                Config {
                    key,
                    file: rel.clone(),
                    rank: ((file_rank(root, path), i)),
                    r#where,
                    lead: tab.lead.clone(),
                    entries,
                    end: tab.end.clone(),
                    base: meta.get("base").and_then(|v| v.as_str().map(|s| s.to_string())),
                    private: meta.get("private").and_then(|v| v.as_bool()).unwrap_or(false),
                    lazy: meta.get("lazy").and_then(|v| v.as_bool()).unwrap_or(false),
                    port: meta.get("port").and_then(|v| v.as_str().map(|s| s.to_string())),
                },
            );
        }
    }
    for c in configs.values() {
        if let Some(base) = &c.base {
            if !configs.contains_key(base) {
                return Err(fail(&c.r#where, format!("base {base:?} isn't a config")));
            }
        }
        for (name, value, _, _, line) in &c.entries {
            if let Some(t) = value.as_table() {
                if let Some(from) = t.get("from").and_then(|v| v.as_str()) {
                    if !configs.contains_key(from) {
                        return Err(fail(&format!("{}:{line}", c.file), format!("from {from:?} isn't a config")));
                    }
                }
            }
            let _ = name;
        }
    }
    let mut ports: HashMap<String, String> = HashMap::new();
    for c in configs.values() {
        let mut root_key = c;
        let mut chain: HashSet<String> = HashSet::new();
        while let Some(base) = &root_key.base {
            if !chain.insert(root_key.key.clone()) {
                return Err(fail(&c.r#where, "its base chain loops"));
            }
            root_key = &configs[base];
        }
        let set_fields: HashSet<&String> = root_key.entries.iter().map(|e| &e.0).collect();
        let missing: Vec<&String> = schema.fields.values()
            .filter(|f| f.required && !set_fields.contains(&f.name))
            .map(|f| &f.name)
            .collect();
        if !missing.is_empty() {
            return Err(fail(&root_key.r#where, format!("has no base, so it must set {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))));
        }
        if let Some(port) = &c.port {
            if let Some(other) = ports.get(port) {
                return Err(fail(&c.r#where, format!("port {port} is also {other}")));
            }
            ports.insert(port.clone(), c.key.clone());
        }
    }
    Ok(configs)
}

/// A [config.<key>.<field>(.<field>...)] sub-table: the record that field holds, spelled out.
fn add_record(
    config: &mut Config,
    path_parts: &[&str],
    tab: &Table,
    schema: &Schema,
    root: &Path,
    rel: &str,
    cache: &mut HashMap<String, Schema>,
) -> Result<(), GameDataError> {
    let r#where = format!("{rel}:{}", tab.line);
    let mut entries = &mut config.entries;
    let mut s = schema;
    for (depth, part) in path_parts.iter().enumerate() {
        let f = s.fields.get(*part).ok_or_else(|| fail(&r#where, format!("`{part}` isn't a {} field", s.class)))?;
        let record_name = f.record.clone().ok_or_else(|| fail(&r#where, format!("`{part}` holds no record to spell out (its schema field has no `record`)")))?;
        load_schema(root, &record_name, cache)?;
        s = cache.get(&record_name).unwrap();
        let existing = entries.iter().find(|e| e.0 == *part);
        if depth == path_parts.len() - 1 {
            if existing.is_some() {
                return Err(fail(&r#where, format!("`{part}` is already set")));
            }
            let mut base = None;
            let mut fields: Vec<Entry> = vec![];
            for e in &tab.entries {
                if e.0 == "base" {
                    base = e.1.as_str().map(|s| s.to_string());
                } else {
                    fields.push(e.clone());
                }
            }
            let rec_schema = cache.get(&record_name).unwrap();
            check_entries(&fields, rec_schema, rel)?;
            let record = RecordValue {
                record: record_name.clone(),
                class: rec_schema.class.clone(),
                base,
                entries: fields,
                end: tab.end.clone(),
            };
            entries.push((part.to_string(), Value::Record(Box::new(record)), tab.lead.clone(), None, tab.line));
        } else {
            let existing = existing.ok_or_else(|| fail(&r#where, format!("[{}] must come first", path_parts[..depth + 1].join("."))))?;
            match &existing.1 {
                Value::Record(_) => {
                    entries = match entries.iter_mut().find(|e| e.0 == *part) {
                        Some(e) => match &mut e.1 {
                            Value::Record(r) => &mut r.entries,
                            _ => unreachable!(),
                        },
                        None => unreachable!(),
                    };
                }
                _ => return Err(fail(&r#where, format!("[{}] must come first", path_parts[..depth + 1].join(".")))),
            }
        }
    }
    Ok(())
}

pub fn check_entries(entries: &[Entry], schema: &Schema, rel: &str) -> Result<(), GameDataError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for (name, value, _, _, line) in entries {
        let r#where = format!("{rel}:{line}");
        let f = schema.fields.get(name).ok_or_else(|| fail(&r#where, format!("`{name}` isn't a {} field (gamedata/schema/{}.toml)", schema.class, schema.name)))?;
        if !seen.insert(name) {
            return Err(fail(&r#where, format!("`{name}` is set twice")));
        }
        check_value(value, &f.ftype, f.nullable, &r#where, name)?;
    }
    Ok(())
}

pub fn check_value(value: &Value, ftype: &str, nullable: bool, r#where: &str, name: &str) -> Result<(), GameDataError> {
    if let Some(t) = value.as_table() {
        let keys: HashSet<&String> = t.keys().collect();
        if ["kotlin", "ref", "from"].iter().any(|k| keys.len() == 1 && keys.contains(&k.to_string()))
            || (keys.contains(&"from".to_string()) && keys.contains(&"field".to_string()) && keys.len() == 2)
        {
            if !t.values().all(|v| matches!(v, Value::Str(_))) {
                return Err(fail(r#where, format!("`{name}`: {t:?} must hold strings")));
            }
            return Ok(());
        }
    }
    if value.as_str() == Some(NONE) {
        if !nullable {
            return Err(fail(r#where, format!("`{name}` can't be {NONE}")));
        }
        return Ok(());
    }
    let is_int = |v: &Value| matches!(v, Value::RawInt(_));
    let ok = match ftype {
        "long" => is_int(value),
        "int" => is_int(value) && (-2i64.pow(31)..2i64.pow(31)).contains(&value.as_int().unwrap()),
        "bool" => matches!(value, Value::Bool(_)),
        "char" => value.as_str().map_or(false, |s| s.chars().count() == 1),
        "string" => matches!(value, Value::Str(_)),
        "ref" => value.as_str().map_or(false, is_ref_name),
        "int_range" => value.as_list().map_or(false, |l| l.len() == 2 && l.iter().all(&is_int)),
        "long_list" | "int_list" => value.as_list().map_or(false, |l| l.iter().all(&is_int)),
        "int_set" | "long_set" => value.as_list().map_or(false, |l| {
            l.iter().all(&is_int) && l.iter().map(|v| v.as_int().unwrap()).collect::<HashSet<_>>().len() == l.len()
        }),
        // a Map<Int, Int> as [[key, value], ...]
        "int_map" => value.as_list().map_or(false, |l| {
            l.iter().all(|p| p.as_list().map_or(false, |pp| pp.len() == 2 && pp.iter().all(&is_int)))
                && l.iter().filter_map(|p| p.as_list().and_then(|pp| pp[0].as_int())).collect::<HashSet<_>>().len() == l.len()
        }),
        "item_descs" => value.as_table().is_some(),
        _ => return Err(fail(r#where, format!("unknown type {ftype}"))),
    };
    if !ok {
        return Err(fail(r#where, format!("`{name}` should be a {ftype}, not {value:?}")));
    }
    if ftype == "item_descs" {
        let t = value.as_table().unwrap();
        for (k, v) in t {
            let Some(&itype) = crate::schema::ITEM_DESC_FIELDS.iter().find(|(n, _)| n == k).map(|(_, t)| t) else {
                let names: Vec<&str> = crate::schema::ITEM_DESC_FIELDS.iter().map(|(n, _)| *n).collect();
                return Err(fail(r#where, format!("`{name}.{k}` isn't an item_descs field ({})", names.join(", "))));
            };
            check_value(v, itype, k == "next", r#where, &format!("{name}.{k}"))?;
        }
        if let Some(preset) = t.get("preset").and_then(|v| v.as_str()) {
            let names: Vec<&str> = crate::schema::PRESETS.iter().map(|(n, _)| *n).collect();
            if !crate::schema::PRESETS.iter().any(|(n, _)| *n == preset) {
                return Err(fail(r#where, format!("`{name}.preset` is one of {names:?}")));
            }
            let extra: Vec<&String> = t.keys().filter(|k| !["preset", "base", "count"].contains(&k.as_str())).collect();
            if !extra.is_empty() {
                return Err(fail(r#where, format!("`{name}` with a preset takes only base and count")));
            }
        } else if !["base", "stride", "descOff"].iter().all(|k| t.contains_key(*k)) {
            return Err(fail(r#where, format!("`{name}` needs base, stride and descOff (or a preset)")));
        }
    }
    Ok(())
}

pub fn dependencies(c: &Config) -> HashSet<String> {
    let mut deps = HashSet::new();
    if let Some(b) = &c.base {
        deps.insert(b.clone());
    }
    for (_, value, _, _, _) in &c.entries {
        if let Some(t) = value.as_table() {
            if let Some(from) = t.get("from").and_then(|v| v.as_str()) {
                deps.insert(from.to_string());
            }
        }
    }
    deps
}

/// Every config after the ones it builds on, else in file order (Kotlin initialises a file's
/// vals top down).
pub fn in_order(configs: &HashMap<String, Config>) -> Result<Vec<Config>, GameDataError> {
    let mut pending: Vec<Config> = {
        let mut v: Vec<&Config> = configs.values().collect();
        v.sort_by(|a, b| a.rank.cmp(&b.rank));
        v.into_iter().map(|c| Config {
            key: c.key.clone(),
            file: c.file.clone(),
            rank: c.rank.clone(),
            r#where: c.r#where.clone(),
            lead: c.lead.clone(),
            entries: c.entries.clone(),
            end: c.end.clone(),
            base: c.base.clone(),
            private: c.private,
            lazy: c.lazy,
            port: c.port.clone(),
        }).collect()
    };
    let mut done: HashSet<String> = HashSet::new();
    let mut out = vec![];
    while !pending.is_empty() {
        let pos = pending.iter().position(|c| dependencies(c).iter().all(|d| done.contains(d)));
        let Some(pos) = pos else {
            return Err(fail(&pending[0].r#where, "its base / from refs loop"));
        };
        let ready = pending.remove(pos);
        done.insert(ready.key.clone());
        out.push(ready);
    }
    Ok(out)
}
