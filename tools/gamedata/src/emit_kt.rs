//! The Kotlin output: NativeConfigsGen.kt (one val per config) and GameDataRecordsGen.kt
//! (the records and lists). Byte-for-byte the old generator's output.

use crate::config::{in_order, Config};
use crate::records::Record;
use crate::schema::Schema;
use crate::toml_raw::{Entry, RawInt, Value};
use crate::{fail, GameDataError};
use std::collections::{HashMap, HashSet};

pub fn kt_name(key: &str) -> String {
    format!("NATIVE_{}", key.to_uppercase())
}

/// Hex addresses carry the L, as NativeReader.kt wrote them; a decimal literal is a Long
/// r#where one is expected.
pub fn kt_int(v: &Value, long: bool) -> String {
    let raw = match v {
        Value::RawInt(RawInt { raw, .. }) => raw.clone(),
        other => format!("{}", other.as_int().unwrap()),
    };
    if long && raw.contains('x') {
        format!("{raw}L")
    } else {
        raw
    }
}

pub fn kt_value(value: &Value, ftype: &str, field: &str, indent: &str, cache: &mut HashMap<String, Schema>) -> Result<String, GameDataError> {
    if let Value::Record(r) = value {
        let head = if let Some(base) = &r.base { format!("{base}.copy") } else { r.class.clone() };
        let schema = load_schema_cache(cache, &r.record)?;
        return kt_call(&head, &r.entries, &r.end, &schema, indent, cache);
    }
    if let Some(t) = value.as_table() {
        if let Some(k) = t.get("kotlin") {
            return Ok(k.as_str().unwrap().to_string());
        }
        if let Some(r) = t.get("ref") {
            return Ok(r.as_str().unwrap().to_string());
        }
        if let Some(from) = t.get("from") {
            let f = t.get("field").and_then(|v| v.as_str()).unwrap_or(field);
            return Ok(format!("{}.{}", kt_name(from.as_str().unwrap()), f));
        }
    }
    if value.as_str() == Some(crate::NONE) {
        return Ok("null".into());
    }
    match ftype {
        "long" | "int" => Ok(kt_int(value, ftype == "long")),
        "bool" => Ok(if value.as_bool().unwrap() { "true" } else { "false" }.into()),
        "char" => {
            let s = value.as_str().unwrap();
            Ok(format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")))
        }
        "string" => {
            let s = value.as_str().unwrap();
            Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$")))
        }
        "ref" => Ok(value.as_str().unwrap().to_string()),
        "int_range" => {
            let l = value.as_list().unwrap();
            Ok(format!("{}..{}", kt_int(&l[0], false), kt_int(&l[1], false)))
        }
        "long_list" | "int_list" => {
            let l = value.as_list().unwrap();
            if l.is_empty() {
                return Ok("emptyList()".into());
            }
            let items: Vec<String> = l
                .iter()
                .map(|v| {
                    let base = kt_int(v, false);
                    if ftype == "long_list" && !base.contains('x') {
                        format!("{base}L")
                    } else {
                        kt_int(v, ftype == "long_list")
                    }
                })
                .collect();
            Ok(format!("listOf({})", items.join(", ")))
        }
        "int_set" | "long_set" => {
            let l = value.as_list().unwrap();
            if l.is_empty() {
                return Ok("emptySet()".into());
            }
            let items: Vec<String> = l
                .iter()
                .map(|v| {
                    let base = kt_int(v, false);
                    if ftype == "long_set" && !base.contains('x') {
                        format!("{base}L")
                    } else {
                        kt_int(v, ftype == "long_set")
                    }
                })
                .collect();
            Ok(format!("setOf({})", items.join(", ")))
        }
        "int_map" => {
            let l = value.as_list().unwrap();
            if l.is_empty() {
                return Ok("emptyMap()".into());
            }
            let pairs: Vec<String> = l
                .iter()
                .map(|p| {
                    let pp = p.as_list().unwrap();
                    format!("{} to {}", kt_int(&pp[0], false), kt_int(&pp[1], false))
                })
                .collect();
            Ok(format!("mapOf({})", pairs.join(", ")))
        }
        "item_descs" => {
            let t = value.as_table().unwrap();
            let mut args: HashMap<&str, String> = HashMap::new();
            for (k, v) in t {
                if k == "preset" {
                    continue;
                }
                let itype = crate::schema::ITEM_DESC_FIELDS.iter().find(|(n, _)| n == k).map(|(_, t)| *t).unwrap();
                args.insert(Box::leak(k.clone().into_boxed_str()), kt_value(v, itype, k, indent, cache)?);
            }
            if let Some(preset) = t.get("preset").and_then(|v| v.as_str()) {
                let fn_name = crate::schema::PRESETS.iter().find(|(n, _)| *n == preset).map(|(_, f)| *f).unwrap();
                let mut call_args = vec![];
                for k in ["base", "count"] {
                    if let Some(a) = args.get(k) {
                        call_args.push(a.clone());
                    }
                }
                return Ok(format!("{fn_name}({})", call_args.join(", ")));
            }
            let mut head = vec![];
            for k in ["base", "stride", "descOff"] {
                head.push(args[k].clone());
            }
            let mut named = vec![];
            for k in ["idOff", "first", "count", "next"] {
                if let Some(a) = args.get(k) {
                    named.push(format!("{k} = {a}"));
                }
            }
            head.extend(named);
            Ok(format!("ItemDescTable({})", head.join(", ")))
        }
        other => Err(fail("emit", format!("unknown type {other}"))),
    }
}

fn load_schema_cache(cache: &mut HashMap<String, Schema>, name: &str) -> Result<Schema, GameDataError> {
    if let Some(s) = cache.get(name) {
        return Ok(clone_schema(s));
    }
    Err(GameDataError(format!("schema {name} not loaded")))
}

fn clone_schema(s: &Schema) -> Schema {
    Schema { name: s.name.clone(), class: s.class.clone(), fields: s.fields.clone() }
}

pub fn kt_comments(lines: &[String], indent: &str) -> Vec<String> {
    lines.iter().map(|l| format!("{indent}//{}", if l.is_empty() { String::new() } else { format!(" {l}") })).collect()
}

/// `head(` + one named argument a line + `)`, at the given indentation.
pub fn kt_call(
    head: &str,
    entries: &[Entry],
    end: &[String],
    schema: &Schema,
    indent: &str,
    cache: &mut HashMap<String, Schema>,
) -> Result<String, GameDataError> {
    if entries.is_empty() && end.is_empty() {
        return Ok(format!("{head}()"));
    }
    let inner = format!("{indent}    ");
    let mut out = vec![format!("{head}(")];
    for (name, value, lead, trail, _) in entries {
        out.extend(kt_comments(lead, &inner));
        let ftype = schema.fields.get(name).map(|f| f.ftype.as_str()).unwrap_or("ref");
        let v = kt_value(value, ftype, name, &inner, cache)?;
        out.push(format!("{inner}{name} = {v},{}", if let Some(t) = trail { format!(" // {t}") } else { String::new() }));
    }
    out.extend(kt_comments(end, &inner));
    out.push(format!("{indent})"));
    Ok(out.join("\n"))
}

pub fn kotlin(configs: &HashMap<String, Config>, schema: &Schema, cache: &mut HashMap<String, Schema>) -> String {
    let mut out = vec![
        "package com.pokedaisy.app.companion.data".to_string(),
        String::new(),
        "// Generated by scripts/gen_gamedata.py from gamedata/gen3/ - don't edit by hand: change the".to_string(),
        "// TOML there and run `make gamedata` (gamedata/README.md). One val per config, each built on".to_string(),
        "// its `base` with .copy(), so a config is still its own object (fireRedHackFor goes by identity).".to_string(),
    ];
    let ordered = in_order(configs).unwrap_or_default();
    let mut last_file: Option<String> = None;
    for c in &ordered {
        if last_file.as_deref() != Some(&c.file) {
            out.push(String::new());
            out.push(format!("// ---- {} ----", c.file));
            last_file = Some(c.file.clone());
        }
        out.push(String::new());
        out.extend(kt_comments(&c.lead, ""));
        let head = if let Some(base) = &c.base { format!("{}.copy", kt_name(base)) } else { "NativeConfig".to_string() };
        let decl = format!("{}val {}", if c.private { "private " } else { "" }, kt_name(&c.key));
        if c.lazy {
            out.push(format!("{decl} by lazy {{"));
            match kt_call(&head, &c.entries, &c.end, schema, "    ", cache) {
                Ok(call) => out.push(format!("    {call}")),
                Err(_) => out.push(String::new()),
            }
            out.push("}".to_string());
        } else {
            match kt_call(&head, &c.entries, &c.end, schema, "", cache) {
                Ok(call) => out.push(format!("{decl} = {call}")),
                Err(_) => out.push(String::new()),
            }
        }
    }
    let mut ports: Vec<(&String, &String)> = configs.values().filter_map(|c| c.port.as_ref().map(|p| (p, &c.key))).collect();
    ports.sort();
    if !ports.is_empty() {
        out.push(String::new());
        out.push("/** The ported configs, by game code + revision (\"BPRD0\"): gamedata/gen3/ports/. */".to_string());
        out.push("val RETAIL_PORTS: Map<String, () -> NativeConfig> = mapOf(".to_string());
        for (port, key) in ports {
            out.push(format!("    \"{port}\" to {{ {} }},", kt_name(key)));
        }
        out.push(")".to_string());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn record_kotlin(records: &HashMap<String, Record>, cache: &mut HashMap<String, Schema>) -> String {
    let mut out = vec![
        "package com.pokedaisy.app.companion.data".to_string(),
        String::new(),
        "// Generated by scripts/gen_gamedata.py from gamedata/gen3/records/ - don't edit by hand: change the".to_string(),
        "// TOML there and run `make gamedata` (gamedata/README.md). Each one is built on first use, so".to_string(),
        "// they can name each other and vals in any other file, whichever file is initialised first.".to_string(),
    ];
    let mut ordered: Vec<&Record> = records.values().collect();
    ordered.sort_by(|a, b| a.rank.cmp(&b.rank));
    let mut last_file: Option<String> = None;
    for r in ordered {
        if last_file.as_deref() != Some(&r.file) {
            out.push(String::new());
            out.push(format!("// ---- {} ----", r.file));
            last_file = Some(r.file.clone());
        }
        out.push(String::new());
        out.extend(kt_comments(&r.lead, ""));
        if r.kind == crate::records::Kind::List {
            let items: Vec<String> = r.items.iter().map(|v| match v {
                Value::Str(s) => s.clone(),
                other => kt_int(other, r.of.as_deref() == Some("Long")),
            }).collect();
            out.push(format!("val {}: List<{}> by lazy {{ listOf({}) }}", r.name, r.of.as_deref().unwrap(), items.join(", ")));
            continue;
        }
        let schema = r.schema.as_ref().unwrap();
        let cls = &schema.class;
        let head = if let Some(base) = &r.base { format!("{base}.copy") } else { cls.clone() };
        out.push(format!("val {}: {cls} by lazy {{", r.name));
        if let Ok(call) = kt_call(&head, &r.entries, &r.end, schema, "    ", cache) {
            out.push(format!("    {call}"));
        }
        out.push("}".to_string());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn unused(_: &HashSet<String>) {}
