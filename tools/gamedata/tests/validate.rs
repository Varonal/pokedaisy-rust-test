//! Unit tests for the value validation (check_value) and the config ordering.

use gamedata::config::{check_value, in_order, Config};
use gamedata::toml_raw::{RawInt, Value};
use std::collections::{BTreeMap, HashMap};

fn int(v: i64) -> Value {
    Value::RawInt(RawInt { value: v, raw: format!("{v}") })
}

#[test]
fn long_accepts_any_int_bool_is_not_an_int() {
    assert!(check_value(&int(5), "long", false, "w", "n").is_ok());
    assert!(check_value(&Value::Bool(true), "long", false, "w", "n").is_err());
}

#[test]
fn int_range_is_checked() {
    assert!(check_value(&int(i64::from(i32::MIN)), "int", false, "w", "n").is_ok());
    assert!(check_value(&int(2i64.pow(31)), "int", false, "w", "n").is_err());
}

#[test]
fn none_only_when_nullable() {
    assert!(check_value(&Value::Str("none".into()), "string", true, "w", "n").is_ok());
    assert!(check_value(&Value::Str("none".into()), "string", false, "w", "n").is_err());
}

#[test]
fn sets_and_maps_must_not_repeat_keys() {
    let dup = Value::List(vec![int(1), int(1)]);
    assert!(check_value(&dup, "int_set", false, "w", "n").is_err());
    let ok = Value::List(vec![int(1), int(2)]);
    assert!(check_value(&ok, "int_set", false, "w", "n").is_ok());
    let map = Value::List(vec![Value::List(vec![int(1), int(2)]), Value::List(vec![int(1), int(3)])]);
    assert!(check_value(&map, "int_map", false, "w", "n").is_err());
}

#[test]
fn int_range_needs_two_ints() {
    let ok = Value::List(vec![int(1), int(2)]);
    assert!(check_value(&ok, "int_range", false, "w", "n").is_ok());
    let bad = Value::List(vec![int(1)]);
    assert!(check_value(&bad, "int_range", false, "w", "n").is_err());
}

#[test]
fn ref_must_be_a_kotlin_name() {
    assert!(check_value(&Value::Str("POKEDEX_EMERALD".into()), "ref", false, "w", "n").is_ok());
    assert!(check_value(&Value::Str("CardStyle.KANTO".into()), "ref", false, "w", "n").is_ok());
    assert!(check_value(&Value::Str("1bad".into()), "ref", false, "w", "n").is_err());
}

#[test]
fn item_descs_preset_rules() {
    let mut t = BTreeMap::new();
    t.insert("preset".to_string(), Value::Str("vanilla".into()));
    t.insert("base".to_string(), int(0x083DB028));
    t.insert("count".to_string(), int(377));
    assert!(check_value(&Value::Table(t.clone()), "item_descs", false, "w", "n").is_ok());
    t.insert("stride".to_string(), int(1)); // preset takes only base and count
    assert!(check_value(&Value::Table(t), "item_descs", false, "w", "n").is_err());
    let mut t2 = BTreeMap::new();
    t2.insert("preset".to_string(), Value::Str("nope".into()));
    assert!(check_value(&Value::Table(t2), "item_descs", false, "w", "n").is_err());
    let mut t3 = BTreeMap::new();
    t3.insert("base".to_string(), int(1));
    assert!(check_value(&Value::Table(t3), "item_descs", false, "w", "n").is_err()); // needs stride + descOff
}

fn cfg(key: &str, base: Option<&str>, rank: usize) -> Config {
    Config {
        key: key.into(),
        file: "f.toml".into(),
        rank: ((0, 0, "f.toml".into()), rank),
        r#where: format!("f.toml:{rank}"),
        lead: vec![],
        entries: vec![],
        end: vec![],
        base: base.map(|s| s.into()),
        private: false,
        lazy: false,
        port: None,
    }
}

#[test]
fn in_order_puts_bases_first() {
    let mut configs: HashMap<String, Config> = HashMap::new();
    configs.insert("child".into(), cfg("child", Some("parent"), 0));
    configs.insert("parent".into(), cfg("parent", None, 1));
    let ordered = in_order(&configs).unwrap();
    assert_eq!(ordered[0].key, "parent");
    assert_eq!(ordered[1].key, "child");
}

#[test]
fn in_order_detects_loops() {
    let mut configs: HashMap<String, Config> = HashMap::new();
    configs.insert("a".into(), cfg("a", Some("b"), 0));
    configs.insert("b".into(), cfg("b", Some("a"), 1));
    assert!(in_order(&configs).is_err());
}
