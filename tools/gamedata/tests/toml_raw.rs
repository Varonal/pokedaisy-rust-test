//! Unit tests for the comment-preserving TOML reader.

use gamedata::toml_raw::{read_toml, Value};
use std::path::PathBuf;

fn write(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("gamedata-test-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

#[test]
fn keeps_comments_leads_trails_and_raw_ints() {
    let dir = tmp();
    let p = write(&dir, "a.toml", "# top\n\n# lead one\n# lead two\n[table]\n# above\nkey = 0xFF  # trail\nother = 12\n");
    let root = dir.clone();
    let tables = read_toml(&p, &root).unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[1].name, "table");
    assert_eq!(tables[1].lead, vec!["lead one".to_string(), "lead two".to_string()]);
    let (k, v, lead, trail, line) = &tables[1].entries[0];
    assert_eq!(k, "key");
    assert_eq!(v.as_int(), Some(0xFF));
    assert_eq!(lead, &vec!["above".to_string()]);
    assert_eq!(trail.as_deref(), Some("trail"));
    assert_eq!(*line, 7);
    let (k2, v2, _, _, _) = &tables[1].entries[1];
    assert_eq!((k2.as_str(), v2.as_int()), ("other", Some(12)));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn blank_line_splits_comment_groups() {
    let dir = tmp();
    let p = write(&dir, "a.toml", "# one\n\n# two\nkey = true\n");
    let tables = read_toml(&p, &dir).unwrap();
    assert_eq!(tables[0].entries[0].2, vec!["one".to_string(), String::new(), "two".to_string()]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn values_of_every_shape() {
    let dir = tmp();
    let p = write(&dir, "a.toml", "[t]\ni = 0x0203988C\nd = -5\nl = 'lit'\ns = \"esc\\n\\\"q\"\nb = false\nlist = [1, 0x2, 'x']\nmap = { a = 1, b = 'two' }\nempty = []\n");
    let tables = read_toml(&p, &dir).unwrap();
    let e: std::collections::HashMap<_, _> = tables[1].entries.iter().map(|(k, v, _, _, _)| (k.clone(), v.clone())).collect();
    assert_eq!(e["i"].as_int(), Some(0x0203988C));
    assert_eq!(e["d"].as_int(), Some(-5));
    assert_eq!(e["l"].as_str(), Some("lit"));
    assert_eq!(e["s"].as_str(), Some("esc\n\"q"));
    assert_eq!(e["b"].as_bool(), Some(false));
    let l = e["list"].as_list().unwrap();
    assert_eq!(l[0].as_int(), Some(1));
    assert_eq!(l[1].as_int(), Some(2));
    assert_eq!(l[2].as_str(), Some("x"));
    assert_eq!(e["map"].as_table().unwrap()["a"].as_int(), Some(1));
    assert!(e["empty"].as_list().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hex_literal_is_kept_verbatim() {
    let dir = tmp();
    let p = write(&dir, "a.toml", "[t]\nx = 0x0A_BC\n");
    let tables = read_toml(&p, &dir).unwrap();
    match &tables[1].entries[0].1 {
        Value::RawInt(r) => {
            assert_eq!(r.raw, "0x0A_BC");
            assert_eq!(r.value, 0xABC);
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rejects_syntax_the_reader_does_not_cover() {
    let dir = tmp();
    // multi-line array: toml accepts it, the reader must refuse (it agrees-checks)
    let p = write(&dir, "a.toml", "[t]\nlist = [\n  1,\n]\n");
    assert!(read_toml(&p, &dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rejects_trailing_garbage_after_a_value() {
    // The toml crate rejects it first; either way the file must not load.
    let dir = tmp();
    let p = write(&dir, "a.toml", "[t]\nkey = 1 2\n");
    assert!(read_toml(&p, &dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bool_not_confused_with_a_word() {
    let dir = tmp();
    // `truex` is not a bool; toml would reject the file, and so must we (agreement check).
    let p = write(&dir, "a.toml", "[t]\nkey = truex\n");
    assert!(read_toml(&p, &dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}
