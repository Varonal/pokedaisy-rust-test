//! The C targets in gamedata/targets/: which configs each one gets and r#where each field
//! of its struct comes from. 3ds.toml -> 3ds/core/pd_configs_gen.h.

use crate::config::Config;
use crate::records::{Kind, Record};
use crate::schema::{load_schema, Schema};
use crate::toml_raw::{read_toml, Value};
use crate::{fail, relpath, GameDataError};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

struct Resolver<'a> {
    configs: &'a HashMap<String, Config>,
    records: &'a HashMap<String, Record>,
    schema: &'a Schema,
    /// {config: {name: record}}: a target's `records`, which `@name.` paths start from.
    attached: HashMap<String, BTreeMap<String, String>>,
    root: &'a Path,
    cache: std::cell::RefCell<HashMap<String, Schema>>,
}

impl<'a> Resolver<'a> {
    fn config_fields(&self, key: &str) -> BTreeMap<String, Value> {
        let c = &self.configs[key];
        let mut fields = if let Some(base) = &c.base { self.config_fields(base) } else { BTreeMap::new() };
        for (name, value, _, _, _) in &c.entries {
            fields.insert(name.clone(), value.clone());
        }
        fields
    }

    fn record_fields(&self, name: &str, r#where: &str) -> Result<(BTreeMap<String, Value>, Schema), GameDataError> {
        let r = self.records.get(name).ok_or_else(|| fail(r#where, format!("{name} isn't a record in gamedata/gen3/records/ (still a Kotlin val?)")))?;
        if r.kind != Kind::Record {
            return Err(fail(r#where, format!("{name} isn't a record in gamedata/gen3/records/ (still a Kotlin val?)")));
        }
        let (mut fields, schema) = if let Some(base) = &r.base {
            self.record_fields(base, r#where)?
        } else {
            (BTreeMap::new(), r.schema.clone().unwrap())
        };
        for (n, value, _, _, _) in &r.entries {
            fields.insert(n.clone(), value.clone());
        }
        Ok((fields, schema))
    }

    /// A field of a config / record: set, else its schema's default; `from` followed.
    fn value(
        &self,
        fields: &BTreeMap<String, Value>,
        schema: &Schema,
        name: &str,
        r#where: &str,
        _key: &str,
    ) -> Result<(Value, SchemaFieldLite), GameDataError> {
        let f = schema.fields.get(name).ok_or_else(|| fail(r#where, format!("`{name}` isn't a {} field", schema.class)))?;
        let v = if let Some(v) = fields.get(name) {
            v.clone()
        } else if let Some(d) = &f.default {
            d.clone()
        } else {
            Value::Str(crate::NONE.to_string())
        };
        if let Some(t) = v.as_table() {
            if let Some(from) = t.get("from").and_then(|v| v.as_str()) {
                if t.keys().all(|k| k == "from" || k == "field") {
                    let other = self.config_fields(from);
                    let field = t.get("field").and_then(|v| v.as_str()).unwrap_or(name);
                    return self.value(&other, self.schema, field, r#where, from);
                }
            }
        }
        Ok((v, SchemaFieldLite { ftype: f.ftype.clone(), record: f.record.clone() }))
    }

    fn walk(&self, key: &str, path: &str, r#where: &str) -> Result<(Value, SchemaFieldLite), GameDataError> {
        let mut fields = self.config_fields(key);
        let mut schema = self.schema;
        let mut parts: Vec<&str> = path.split('.').collect();
        if let Some(first) = parts.first() {
            if let Some(name) = first.strip_prefix('@') {
                let record = self.attached.get(key).and_then(|m| m.get(name)).ok_or_else(|| {
                    fail(r#where, format!("{path}: the target gives {key} no `{name}` record (its `records`)"))
                })?;
                let (f, s) = self.record_fields(record, r#where)?;
                fields = f;
                schema = Box::leak(Box::new(s));
                parts = parts[1..].to_vec();
            }
        }
        for (depth, part) in parts.iter().enumerate() {
            let last = depth == parts.len() - 1;
            let (v, f) = self.value(&fields, schema, part, r#where, key)?;
            if last {
                return Ok((v, f));
            }
            let ftype = f.ftype.clone();
            if ftype == "long_list" || ftype == "int_list" {
                match &v {
                    Value::List(l) => {
                        // The next part indexes the list.
                        let next = parts[depth + 1];
                        if !next.chars().all(|c| c.is_ascii_digit()) {
                            return Err(fail(r#where, format!("{path}: `{next}` indexes a list")));
                        }
                        let i: usize = next.parse().map_err(|_| fail(r#where, format!("{path}: `{next}` indexes a list")))?;
                        let item = if i < l.len() { l[i].clone() } else { Value::Str(crate::NONE.to_string()) };
                        return Ok((item, SchemaFieldLite { ftype: if ftype == "long_list" { "long".into() } else { "int".into() }, record: None }));
                    }
                    _ => {
                        return Ok((Value::Str(crate::NONE.to_string()), SchemaFieldLite { ftype: if ftype == "long_list" { "long".into() } else { "int".into() }, record: None }));
                    }
                }
            }
            if let Value::Record(r) = &v {
                // a sub-table
                let mut inner = if let Some(base) = &r.base { self.record_fields(base, r#where)?.0 } else { BTreeMap::new() };
                for (n, val, _, _, _) in &r.entries {
                    inner.insert(n.clone(), val.clone());
                }
                let s = self.cache.borrow().get(&r.record).cloned().unwrap_or_else(|| load_schema(self.root, &r.record, &mut self.cache.borrow_mut()).unwrap());
                fields = inner;
                schema = Box::leak(Box::new(s));
                continue;
            }
            if v.as_str() == Some(crate::NONE) {
                if let Some(rec) = &f.record {
                    let s = self.cache.borrow().get(rec).cloned().unwrap_or_else(|| load_schema(self.root, rec, &mut self.cache.borrow_mut()).unwrap());
                    fields = BTreeMap::new();
                    schema = Box::leak(Box::new(s));
                    continue;
                }
                return Err(fail(r#where, format!("{path}: {key} has no `{part}` and it holds no record with defaults")));
            }
            if let Value::Str(s) = &v {
                let (rf, rs) = self.record_fields(s, r#where)?;
                fields = rf;
                schema = Box::leak(Box::new(rs));
                continue;
            }
            return Err(fail(r#where, format!("{path}: `{part}` of {key} is {v:?}, which a C target can't follow")));
        }
        Err(GameDataError("unreachable".into()))
    }
}

struct SchemaFieldLite {
    ftype: String,
    record: Option<String>,
}

fn c_value(v: &Value, ftype: &str, names: &BTreeMap<String, Value>, lists: &HashMap<String, &Record>, r#where: &str, none: Option<&str>) -> Result<String, GameDataError> {
    let is_list_type = matches!(ftype, "long_list" | "int_list" | "int_set" | "long_set" | "int_map");
    if is_list_type {
        let l = v.as_list();
        if l.map_or(true, |l| l.is_empty()) {
            return Ok(if ftype == "int_map" { "{ { 0 } }".into() } else { "{ 0 }".into() });
        }
        let l = l.unwrap();
        let items: Vec<String> = l
            .iter()
            .map(|i| {
                if let Some(pair) = i.as_list() {
                    format!("{{ {} }}", pair.iter().map(|x| format!("{}", x.as_int().unwrap())).collect::<Vec<_>>().join(", "))
                } else {
                    c_value(i, "int", names, lists, r#where, None).unwrap()
                }
            })
            .collect();
        return Ok(format!("{{ {} }}", items.join(", ")));
    }
    if let Some(t) = v.as_table() {
        return Err(fail(r#where, format!("{t:?} is Kotlin; the C needs the value in gamedata/")));
    }
    if v.as_str() == Some(crate::NONE) {
        return Ok("0".into());
    }
    if let Some(n) = none {
        if v.as_int() == Some(n.parse::<i64>().unwrap_or(i64::MIN)) || v.as_str() == Some(n) {
            return Ok("0".into());
        }
    }
    if ftype == "string" {
        let s = v.as_str().unwrap();
        return Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    if ftype == "bool" || matches!(v, Value::Bool(_)) {
        return Ok(if v.as_bool().unwrap_or(false) { "true" } else { "false" }.into());
    }
    if let Some(i) = v.as_int() {
        let raw = match v {
            Value::RawInt(r) => r.raw.replace('_', ""),
            _ => format!("{i}"),
        };
        return Ok(raw);
    }
    if let Value::Str(s) = v {
        if let Some(list) = lists.get(s) {
            let items: Vec<String> = list.items.iter().map(|i| c_value(i, "int", names, lists, r#where, None).unwrap()).collect();
            return Ok(format!("{{ {} }}", items.join(", ")));
        }
        let mut node: Option<&Value> = None;
        let mut cur = names;
        let mut leaf: Option<&Value> = None;
        for part in s.split('.') {
            match cur.get(part) {
                Some(Value::Str(_)) => {
                    leaf = Some(cur.get(part).unwrap());
                    node = leaf;
                }
                Some(Value::Table(_t)) => {
                    // descend
                    let t2 = match cur.get(part) {
                        Some(Value::Table(t)) => t,
                        _ => unreachable!(),
                    };
                    let _ = t2;
                    // restructure: keep a reference chain
                    node = Some(cur.get(part).unwrap());
                    if let Value::Table(t) = cur.get(part).unwrap() {
                        cur = t;
                    }
                }
                _ => return Err(fail(r#where, format!("{s} has no C name in the target's [names]"))),
            }
        }
        if let Some(Value::Str(c)) = node {
            if cur.get(s.rsplit('.').next().unwrap_or(s)).is_some() && leaf.is_some() {
                return Ok(leaf.unwrap().as_str().unwrap().to_string());
            }
            return Ok(c.clone());
        }
        return Err(fail(r#where, format!("{s} has no C name in the target's [names]")));
    }
    Err(fail(r#where, format!("can't write {v:?} in C")))
}

/// One target file -> its output path and text.
pub fn c_target(
    path: &Path,
    configs: &HashMap<String, Config>,
    records: &HashMap<String, Record>,
    schema: &Schema,
    cache: &mut HashMap<String, Schema>,
    root: &Path,
) -> Result<(PathBuf, String), GameDataError> {
    let rel = relpath(path, root);
    let tables = read_toml(path, root)?;
    let top: BTreeMap<String, Value> = tables[0].entries.iter().map(|(k, v, _, _, _)| (k.clone(), v.clone())).collect();
    let mut names: BTreeMap<String, Value> = BTreeMap::new();
    let mut parts: Vec<(String, Vec<(String, Value, String)>)> = vec![];
    let mut wanted: Vec<(String, String, Vec<String>, Vec<String>, String, BTreeMap<String, String>)> = vec![];
    for tab in &tables[1..] {
        let r#where = format!("{rel}:{}", tab.line);
        let entries: Vec<(String, Value, usize)> = tab.entries.iter().map(|(k, v, _, _, l)| (k.clone(), v.clone(), *l)).collect();
        if tab.name == "names" || tab.name.starts_with("names.") {
            let mut node = &mut names;
            for p in tab.name.split('.').skip(1) {
                node = match node.entry(p.to_string()).or_insert_with(|| Value::Table(BTreeMap::new())) {
                    Value::Table(t) => t,
                    _ => return Err(fail(&r#where, "names clash")),
                };
            }
            for (k, v, _) in &entries {
                node.insert(k.clone(), v.clone());
            }
        } else if let Some(p) = tab.name.strip_prefix("part.") {
            parts.push((p.to_string(), entries.iter().map(|(k, v, l)| (k.clone(), v.clone(), format!("{rel}:{l}"))).collect()));
        } else if let Some(key) = tab.name.strip_prefix("config.") {
            if !configs.contains_key(key) {
                return Err(fail(&r#where, format!("{key} isn't a config")));
            }
            let get = |n: &str| entries.iter().find(|(k, _, _)| k == n).map(|(_, v, _)| v.clone());
            let part_list = match get("parts") {
                Some(Value::List(l)) => l.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>(),
                _ => vec![],
            };
            for p in &part_list {
                if !parts.iter().any(|(n, _)| n == p) {
                    return Err(fail(&r#where, format!("no [part.{p}]")));
                }
            }
            let attached = match get("records") {
                Some(Value::Table(t)) => t.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect(),
                _ => BTreeMap::new(),
            };
            for name in attached.values() {
                if records.get(name).map_or(true, |r| r.kind != Kind::Record) {
                    return Err(fail(&r#where, format!("{name} isn't a record in gamedata/gen3/records/")));
                }
            }
            let cname = get("name").and_then(|v| v.as_str().map(|s| s.to_string())).ok_or_else(|| fail(&r#where, "a config target needs `name`"))?;
            wanted.push((key.to_string(), cname, part_list, tab.lead.clone(), r#where, attached));
        } else {
            return Err(fail(&r#where, format!("unexpected [{}]", tab.name)));
        }
    }
    let lists: HashMap<String, &Record> = records.iter().filter(|(_, r)| r.kind == Kind::List).map(|(n, r)| (n.clone(), r)).collect();
    let res = Resolver {
        configs,
        records,
        schema,
        attached: wanted.iter().map(|(k, _, _, _, _, a)| (k.clone(), a.clone())).collect(),
        root,
        cache: std::cell::RefCell::new(cache.clone()),
    };
    let struct_name = top.get("struct").and_then(|v| v.as_str()).ok_or_else(|| fail(&rel, "no struct"))?;
    let prefix = top.get("prefix").and_then(|v| v.as_str()).ok_or_else(|| fail(&rel, "no prefix"))?;
    let output = top.get("output").and_then(|v| v.as_str()).ok_or_else(|| fail(&rel, "no output"))?;
    let mut out = vec![
        format!("// Generated by scripts/gen_gamedata.py from gamedata/ ({rel}) - don't edit by hand: change"),
        "// the TOML there and run `make gamedata` (gamedata/README.md). Included by pd_game.c only.".to_string(),
        "#ifndef PD_CONFIGS_GEN_H".to_string(),
        "#define PD_CONFIGS_GEN_H".to_string(),
        String::new(),
        "#include \"pd_game.h\"".to_string(),
    ];
    for (key, cname, want, lead, _where, _) in &wanted {
        let c = &configs[key];
        out.push(String::new());
        out.extend(crate::emit_kt::kt_comments(lead, ""));
        out.push(format!("// {}: {key} ({})", c.file, want.join(", ")));
        out.push(format!("static const struct {struct_name} {prefix}{cname} = {{"));
        for part in want {
            for (field, spec, fwhere) in &parts.iter().find(|(n, _)| n == part).unwrap().1 {
                let (spec_path, none) = if let Some(t) = spec.as_table() {
                    (t.get("path").and_then(|v| v.as_str()).ok_or_else(|| fail(fwhere, "a part spec needs `path`"))?.to_string(),
                     Some(t.get("none").map(|v| match v { Value::RawInt(r) => r.raw.clone(), Value::Str(x) => x.clone(), _ => "0".into() }).unwrap_or_default()))
                } else {
                    (spec.as_str().ok_or_else(|| fail(fwhere, "a part spec is a path or { path, none }"))?.to_string(), None)
                };
                let (v, f) = res.walk(key, &spec_path, fwhere)?;
                out.push(format!("    .{field} = {},", c_value(&v, &f.ftype, &names, &lists, fwhere, none.as_deref())?));
            }
        }
        out.push("};".to_string());
    }
    out.push(String::new());
    out.push("#endif".to_string());
    let mut text = out.join("\n");
    text.push('\n');
    Ok((root.join(output), text))
}
