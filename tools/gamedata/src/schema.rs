//! The schemas in gamedata/schema/: every field of the Kotlin data class they describe,
//! its type, its default, whether it's nullable / required, and which schema a `record`
//! field spells out as a sub-table.

use crate::kotlin_src::kotlin_class_fields;
use crate::toml_raw::{read_toml, Value};
use crate::{fail, relpath, schemas_dir, GameDataError, SCHEMA_VERSION};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub const TYPES: [&str; 13] = [
    "long", "int", "bool", "char", "string", "item_descs", "int_range", "long_list", "int_list",
    "int_set", "long_set", "int_map", "ref",
];
pub const KOTLIN_TYPES: [(&str, &str); 7] = [
    ("Long", "long"),
    ("Int", "int"),
    ("Boolean", "bool"),
    ("Char", "char"),
    ("String", "string"),
    ("ItemDescTable", "item_descs"),
    ("IntRange", "int_range"),
];
pub const ITEM_DESC_FIELDS: [(&str, &str); 8] = [
    ("preset", "string"),
    ("base", "long"),
    ("stride", "int"),
    ("descOff", "int"),
    ("idOff", "int"),
    ("first", "int"),
    ("count", "int"),
    ("next", "item_descs"),
];
pub const PRESETS: [(&str, &str); 2] = [("vanilla", "vanillaItems"), ("japanese", "japaneseItems")];

#[derive(Debug)]
pub struct SchemaField {
    pub name: String,
    pub kotlin: String,
    pub ftype: String,
    pub nullable: bool,
    pub required: bool,
    pub default: Option<Value>,
    pub record: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Schema {
    pub name: String,
    pub class: String,
    pub fields: BTreeMap<String, SchemaField>,
}

pub fn schema_type(kotlin: &str) -> &str {
    let base = kotlin.trim_end_matches('?');
    let base = base.split('<').next().unwrap_or(base);
    KOTLIN_TYPES
        .iter()
        .find(|(k, _)| *k == base)
        .map(|(_, t)| *t)
        .unwrap_or("ref")
}

pub fn load_schema(
    root: &Path,
    name: &str,
    cache: &mut HashMap<String, Schema>,
) -> Result<Schema, GameDataError> {
    if let Some(s) = cache.get(name) {
        return Ok(clone_schema(s));
    }
    let path = schemas_dir(root).join(format!("{name}.toml"));
    let rel = relpath(&path, root);
    if !path.exists() {
        return Err(fail(&rel, format!("no schema {rel}")));
    }
    let tables = read_toml(&path, root)?;
    let top: BTreeMap<String, Value> = tables[0].entries.iter().map(|(k, v, _, _, _)| (k.clone(), v.clone())).collect();
    if top.get("version").and_then(|v| v.as_int()) != Some(SCHEMA_VERSION) {
        return Err(fail(&rel, format!("schema version {:?}, this generator reads {SCHEMA_VERSION}", top.get("version").and_then(|v| v.as_int()))));
    }
    let class = top.get("kotlin_class").and_then(|v| v.as_str()).ok_or_else(|| fail(&rel, "no kotlin_class"))?.to_string();
    let kotlin_file = top.get("kotlin_file").and_then(|v| v.as_str()).ok_or_else(|| fail(&rel, "no kotlin_file"))?.to_string();
    let mut fields = BTreeMap::new();
    for tab in &tables[1..] {
        if !tab.name.starts_with("field.") {
            return Err(fail(&format!("{rel}:{}", tab.line), format!("unexpected table [{}]", tab.name)));
        }
        let f: BTreeMap<String, Value> = tab.entries.iter().map(|(k, v, _, _, _)| (k.clone(), v.clone())).collect();
        let ftype = f.get("type").and_then(|v| v.as_str()).ok_or_else(|| fail(&format!("{rel}:{}", tab.line), "no type"))?;
        if !TYPES.contains(&ftype) {
            let mut sorted = TYPES.to_vec();
            sorted.sort();
            return Err(fail(&format!("{rel}:{}", tab.line), format!("type {ftype:?} isn't one of {sorted:?}")));
        }
        let kotlin = f.get("kotlin").and_then(|v| v.as_str()).ok_or_else(|| fail(&format!("{rel}:{}", tab.line), "no kotlin"))?.to_string();
        fields.insert(
            tab.name[6..].to_string(),
            SchemaField {
                name: tab.name[6..].to_string(),
                kotlin: kotlin.clone(),
                ftype: ftype.to_string(),
                nullable: kotlin.ends_with('?'),
                required: !f.contains_key("default"),
                default: f.get("default").cloned(),
                record: f.get("record").and_then(|v| v.as_str()).map(|s| s.to_string()),
            },
        );
    }
    let schema = Schema { name: name.to_string(), class: class.clone(), fields };
    let kotlin = kotlin_class_fields(root, &kotlin_file, &class)?;
    let kotlin: HashMap<String, String> = kotlin.into_iter().map(|f| (f.name, f.kotlin)).collect();
    for f in kotlin.keys() {
        if !schema.fields.contains_key(f) {
            return Err(fail(&rel, format!("{class} has `{f}` but the schema doesn't")));
        }
    }
    for f in schema.fields.keys() {
        if !kotlin.contains_key(f) {
            return Err(fail(&rel, format!("the schema has `{f}` but {class} doesn't")));
        }
    }
    for (f, ktype) in &kotlin {
        if schema.fields[f].kotlin != *ktype {
            return Err(fail(&rel, format!("`{f}` is {ktype} in {class}, {} in the schema", schema.fields[f].kotlin)));
        }
    }
    let records: Vec<String> = schema.fields.values().filter_map(|f| f.record.clone()).collect();
    for r in records {
        load_schema(root, &r, cache)?;
    }
    cache.insert(name.to_string(), clone_schema(&schema));
    Ok(schema)
}

fn clone_schema(s: &Schema) -> Schema {
    Schema {
        name: s.name.clone(),
        class: s.class.clone(),
        fields: s.fields.clone(),
    }
}

impl Clone for SchemaField {
    fn clone(&self) -> Self {
        SchemaField {
            name: self.name.clone(),
            kotlin: self.kotlin.clone(),
            ftype: self.ftype.clone(),
            nullable: self.nullable,
            required: self.required,
            default: self.default.clone(),
            record: self.record.clone(),
        }
    }
}

/// `default` of a schema field, if it has one.
pub fn schema_default(f: &SchemaField, r#where: &str) -> Result<Option<Value>, GameDataError> {
    let _ = r#where;
    Ok(f.default.clone())
}

pub fn is_ref_name(s: &str) -> bool {
    Regex::new(r"^[A-Za-z_][\w.]*$").unwrap().is_match(s)
}
