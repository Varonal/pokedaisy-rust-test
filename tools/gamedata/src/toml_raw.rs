//! A small TOML reader that keeps what a plain parser drops: comments and how numbers were
//! written. The `toml` crate is the authority on the syntax; this reader must agree with it
//! (checked at the end of `read_toml`). One `key = value` a line, like the old script.

use crate::records::RecordValue;
use crate::{fail, GameDataError};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;

/// A TOML value, in the shapes gamedata/ uses. `RawInt` remembers its literal (hex stays
/// hex in the generated Kotlin).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    RawInt(RawInt),
    Bool(bool),
    Str(String),
    List(Vec<Value>),
    /// An inline table `{ a = 1 }`.
    Table(BTreeMap<String, Value>),
    /// A [config.x.field] sub-table: the record that field holds, spelled out.
    Record(Box<RecordValue>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RawInt {
    pub value: i64,
    pub raw: String,
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::RawInt(i) => Some(i.value),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
    /// The plain value the `toml` crate would parse this as (for the agreement check).
    fn to_plain(&self) -> toml::Value {
        match self {
            Value::RawInt(i) => toml::Value::Integer(i.value),
            Value::Bool(b) => toml::Value::Boolean(*b),
            Value::Str(s) => toml::Value::String(s.clone()),
            Value::List(l) => toml::Value::Array(l.iter().map(|v| v.to_plain()).collect()),
            Value::Table(t) => toml::Value::Table(t.iter().map(|(k, v)| (k.clone(), v.to_plain())).collect()),
            Value::Record(_) => toml::Value::Table(toml::map::Map::new()),
        }
    }
}

/// One `key = value` line: the key, the value, the comments above it (`lead`), a trailing
/// `// ...` comment, and the line number.
pub type Entry = (String, Value, Vec<String>, Option<String>, usize);

/// A `[table]`: its name, the comments right above the header, its entries, the comments
/// after its last entry (`end`), and the header's line number.
#[derive(Debug)]
pub struct Table {
    pub name: String,
    pub lead: Vec<String>,
    pub entries: Vec<Entry>,
    pub end: Vec<String>,
    pub line: usize,
}

const VALUE_TOKEN: &str = r#"\s*(?:(?P<num>[+-]?(?:0x[0-9A-Fa-f_]+|[0-9][0-9_]*))|(?P<bool>true|false)|(?P<lit>'[^'\n]*')|(?P<str>"(?:[^"\\\n]|\\.)*")|(?P<open>[\[{]))"#;

fn parse_value(text: &str, pos: usize, r#where: &str) -> Result<(Value, usize), GameDataError> {
    let re = Regex::new(VALUE_TOKEN).unwrap();
    let m = re.captures_at(&text, pos).ok_or_else(|| {
        if std::env::var("GAMEDATA_DEBUG").is_ok() {
            eprintln!("DEBUG parse_value pos={pos} text={:?} token={:?}", text, VALUE_TOKEN);
        }
        fail(r#where, format!("can't read a value at {:?}", &text[pos..(pos + 30).min(text.len())]))
    })?;
    let end = m.get(0).unwrap().end(); // captures_at gives absolute positions
    if let Some(num) = m.name("num") {
        let raw = num.as_str();
        let body = raw.trim_start_matches(['+', '-']);
        let neg = raw.starts_with('-');
        let stripped = body.replace('_', "");
        let (digits, radix) = if let Some(h) = stripped.strip_prefix("0x").or_else(|| stripped.strip_prefix("0X")) {
            (h, 16)
        } else {
            (stripped.as_str(), 10)
        };
        let mut value = i64::from_str_radix(digits, radix).map_err(|_| fail(r#where, format!("bad number {raw:?}")))?;
        if neg { value = -value; }
        return Ok((Value::RawInt(RawInt { value, raw: raw.to_string() }), end));
    }
    if let Some(b) = m.name("bool") {
        let after = text.as_bytes().get(end);
        if after.map_or(false, |c| (*c as char).is_alphanumeric() || *c == b'_') {
            let re2 = Regex::new(r#"\s*(?:(?P<num>[+-]?(?:0x[0-9A-Fa-f_]+|[0-9][0-9_]*))|(?P<lit>'[^'\n]*')|(?P<str>"(?:[^"\\\n]|\\.)*")|(?P<open>[\[{]))"#).unwrap();
            let m2 = re2.captures_at(&text, pos).ok_or_else(|| fail(r#where, "can't read a value"))?;
            return parse_value2(&m2, text, pos, r#where);
        }
        return Ok((Value::Bool(b.as_str() == "true"), end));
    }
    if let Some(lit) = m.name("lit") {
        return Ok((Value::Str(lit.as_str()[1..lit.as_str().len() - 1].to_string()), end));
    }
    if let Some(st) = m.name("str") {
        let parsed: toml::Value = toml::from_str(&format!("v = {}", st.as_str()))
            .map_err(|e| fail(r#where, format!("bad string {:?}: {e}", st.as_str())))?;
        return Ok((Value::Str(parsed["v"].as_str().unwrap().to_string()), end));
    }
    let open = &text[m.get(0).unwrap().start()..end];
    parse_container(text, end, open.trim(), r#where)
}

fn parse_value2(m: &regex::Captures, text: &str, _pos: usize, r#where: &str) -> Result<(Value, usize), GameDataError> {
    let end = m.get(0).unwrap().end(); // captures_at gives absolute positions
    if let Some(num) = m.name("num") {
        let raw = num.as_str();
        let body = raw.trim_start_matches(['+', '-']);
        let neg = raw.starts_with('-');
        let stripped = body.replace('_', "");
        let (digits, radix) = if let Some(h) = stripped.strip_prefix("0x").or_else(|| stripped.strip_prefix("0X")) {
            (h, 16)
        } else {
            (stripped.as_str(), 10)
        };
        let mut value = i64::from_str_radix(digits, radix).map_err(|_| fail(r#where, format!("bad number {raw:?}")))?;
        if neg { value = -value; }
        return Ok((Value::RawInt(RawInt { value, raw: raw.to_string() }), end));
    }
    if let Some(lit) = m.name("lit") {
        return Ok((Value::Str(lit.as_str()[1..lit.as_str().len() - 1].to_string()), end));
    }
    if let Some(s) = m.name("str") {
        let parsed: toml::Value = toml::from_str(&format!("v = {}", s.as_str()))
            .map_err(|e| fail(r#where, format!("bad string {:?}: {e}", s.as_str())))?;
        return Ok((Value::Str(parsed["v"].as_str().unwrap().to_string()), end));
    }
    parse_container(text, end, text[m.get(0).unwrap().start()..end].trim(), r#where)
}

fn parse_container(text: &str, mut pos: usize, open: &str, r#where: &str) -> Result<(Value, usize), GameDataError> {
    if open == "[" {
        let mut items = Vec::new();
        loop {
            pos = skip_ws(text, pos);
            if text.as_bytes().get(pos) == Some(&b']') {
                return Ok((Value::List(items), pos + 1));
            }
            let (v, p) = parse_value(text, pos, r#where)?;
            items.push(v);
            pos = skip_ws(text, p);
            if text.as_bytes().get(pos) == Some(&b',') {
                pos += 1;
            }
        }
    }
    let mut table = BTreeMap::new();
    let key_re = Regex::new(r"([A-Za-z0-9_-]+)\s*=").unwrap();
    loop {
        pos = skip_ws(text, pos);
        if text.as_bytes().get(pos) == Some(&b'}') {
            return Ok((Value::Table(table), pos + 1));
        }
        let km = key_re.captures_at(text, pos).ok_or_else(|| fail(r#where, format!("bad inline table near {:?}", &text[pos..(pos + 30).min(text.len())])))?;
        let kend = km.get(0).unwrap().end();
        let (v, p) = parse_value(text, kend, r#where)?;
        table.insert(km[1].to_string(), v);
        pos = skip_ws(text, p);
        if text.as_bytes().get(pos) == Some(&b',') {
            pos += 1;
        }
    }
}

fn skip_ws(text: &str, mut pos: usize) -> usize {
    while pos < text.len() && matches!(text.as_bytes()[pos], b' ' | b'\t') {
        pos += 1;
    }
    pos
}

/// Reads one TOML file into its tables, comments included, then checks the values against
/// the `toml` crate (the authority on the syntax).
pub fn read_toml(path: &Path, root: &Path) -> Result<Vec<Table>, GameDataError> {
    let rel = crate::relpath(path, root);
    let bytes = std::fs::read(path).map_err(|e| fail(&rel, e.to_string()))?;
    let text0 = String::from_utf8(bytes).map_err(|e| fail(&rel, e.to_string()))?;
    let official: toml::Value = text0.parse().map_err(|e: toml::de::Error| fail(&rel, format!("not valid TOML: {e}")))?;
    let lines: Vec<&str> = text0.split('\n').collect();
    let mut tables = vec![Table { name: String::new(), lead: vec![], entries: vec![], end: vec![], line: 0 }];
    // Comment groups since the last key / header, split at blank lines.
    let mut groups: Vec<Vec<String>> = vec![vec![]];

    fn flush_end(tables: &mut Vec<Table>, groups: &mut Vec<Vec<String>>) {
        let g = std::mem::take(groups);
        tables.last_mut().unwrap().end.extend(g.into_iter().flatten());
        *groups = vec![vec![]];
    }

    for (n0, line) in lines.iter().enumerate() {
        let n = n0 + 1;
        let t = line.trim();
        let r#where = format!("{rel}:{n}");
        if t.is_empty() {
            if !groups.last().unwrap().is_empty() {
                groups.push(vec![]);
            }
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            let c = c.strip_prefix(' ').unwrap_or(c);
            groups.last_mut().unwrap().push(c.to_string());
            continue;
        }
        if let Some(h) = Regex::new(r"^\[([A-Za-z0-9_.-]+)\]$").unwrap().captures(t) {
            let lead = if groups.last().unwrap().is_empty() { vec![] } else { groups.pop().unwrap() };
            flush_end(&mut tables, &mut groups);
            tables.push(Table { name: h[1].to_string(), lead, entries: vec![], end: vec![], line: n });
            groups = vec![vec![]];
            continue;
        }
        let k = Regex::new(r"^([A-Za-z0-9_-]+)\s*=").unwrap().captures(t).ok_or_else(|| fail(&r#where, format!("can't read {t:?}")))?;
        let kend = k.get(0).unwrap().end(); // end within `t` (the trimmed line)
        let (value, rel_end) = parse_value(t, kend, &r#where)?;
        let pos = kend + (rel_end - kend); // parse_value returns an absolute index into `t`
        let rest = t[pos..].trim();
        if !rest.is_empty() && !rest.starts_with('#') {
            return Err(fail(&r#where, format!("unexpected {rest:?} after the value")));
        }
        let mut lead = vec![];
        for g in &groups {
            if !g.is_empty() {
                if !lead.is_empty() {
                    lead.push(String::new());
                }
                lead.extend(g.iter().cloned());
            }
        }
        let trail = if rest.is_empty() { None } else { Some(rest[1..].trim().to_string()) };
        tables.last_mut().unwrap().entries.push((k[1].to_string(), value, lead, trail, n));
        groups = vec![vec![]];
    }
    flush_end(&mut tables, &mut groups);

    // This reader must agree with the toml crate.
    let mut mine = toml::map::Map::new();
    for tab in &tables {
        let mut node: &mut toml::map::Map<String, toml::Value> = &mut mine;
        for part in tab.name.split('.').filter(|p| !p.is_empty()) {
            node = match node.entry(part.to_string()).or_insert_with(|| toml::Value::Table(toml::map::Map::new())) {
                toml::Value::Table(t) => t,
                _ => return Err(fail(&rel, "table/key clash")),
            };
        }
        for (key, value, _, _, _) in &tab.entries {
            node.insert(key.clone(), value.to_plain());
        }
    }
    let mine = toml::Value::Table(mine);
    if official != mine {
        return Err(fail(&rel, "this file uses TOML the generator doesn't read (keep to one `key = value` a line)"));
    }
    Ok(tables)
}
