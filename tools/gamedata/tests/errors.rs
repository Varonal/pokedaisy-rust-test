//! Error-path tests on copies of gamedata/ in a temp dir: bad bases, unknown fields,
//! duplicate keys, loops, stale schemas - each must fail with the same message shape
//! the old script gave (file:line: message).

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

/// A minimal repo skeleton: schema + one config + one record + one target.
fn skeleton() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("gamedata-err-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    let gd = dir.join("gamedata");
    std::fs::create_dir_all(gd.join("schema")).unwrap();
    std::fs::create_dir_all(gd.join("gen3/retail")).unwrap();
    std::fs::create_dir_all(gd.join("gen3/records")).unwrap();
    std::fs::create_dir_all(gd.join("targets")).unwrap();
    std::fs::create_dir_all(dir.join("app/src/main/kotlin/com/pokedaisy/app/companion/data")).unwrap();
    std::fs::write(
        dir.join("app/src/main/kotlin/com/pokedaisy/app/companion/data/NativeReader.kt"),
        "data class NativeConfig(\n    val a: Long,\n    val b: Int = 3,\n)\n",
    )
    .unwrap();
    std::fs::write(
        gd.join("schema/gen3_config.toml"),
        "version = 1\nkotlin_class = 'NativeConfig'\nkotlin_file = 'NativeReader.kt'\n\n[field.a]\nkotlin = 'Long'\ntype = 'long'\n\n[field.b]\nkotlin = 'Int'\ntype = 'int'\ndefault = 3\n",
    )
    .unwrap();
    std::fs::write(gd.join("gen3/retail/game.toml"), "[config.game]\na = 0x02024284\n").unwrap();
    std::fs::write(gd.join("gen3/records/r.toml"), "[list.NUMS]\nof = 'Int'\nitems = [1, 2]\n").unwrap();
    std::fs::write(
        gd.join("targets/t.toml"),
        "output = 'out.h'\nstruct = 'pd_config'\nprefix = 'PD_'\n\n[part.ram]\na = 'a'\n\n[config.game]\nname = 'GAME'\nparts = ['ram']\n",
    )
    .unwrap();
    dir
}

fn gen(dir: &PathBuf) -> Result<String, String> {
    gamedata::generate(dir).map(|(_, s)| s).map_err(|e| e.0)
}

#[test]
fn unknown_field_fails_with_file_and_line() {
    let dir = skeleton();
    std::fs::write(dir.join("gamedata/gen3/retail/game.toml"), "[config.game]\na = 1\nnope = 2\n").unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("gamedata/gen3/retail/game.toml:3"), "{err}");
    assert!(err.contains("isn't a NativeConfig field"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_base_fails() {
    let dir = skeleton();
    std::fs::write(dir.join("gamedata/gen3/retail/game.toml"), "[config.game]\nbase = 'missing'\na = 1\n").unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("base \"missing\" isn't a config") || err.contains("base 'missing' isn't a config"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn root_config_must_set_required_fields() {
    let dir = skeleton();
    std::fs::write(dir.join("gamedata/gen3/retail/game.toml"), "[config.game]\nb = 4\n").unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("must set a"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_field_fails() {
    // toml itself rejects a duplicate key; the generator's own check fires when the
    // reader is fed the table another way - here we assert the reader surfaces it.
    let dir = skeleton();
    std::fs::write(dir.join("gamedata/gen3/retail/game.toml"), "[config.game]\na = 1\na = 2\n").unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("duplicate") || err.contains("is set twice"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn base_loop_fails() {
    let dir = skeleton();
    std::fs::write(
        dir.join("gamedata/gen3/retail/game.toml"),
        "[config.a]\nbase = 'b'\na = 1\n\n[config.b]\nbase = 'a'\na = 2\n",
    )
    .unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("loops"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_must_match_the_kotlin_class() {
    let dir = skeleton();
    std::fs::write(
        dir.join("gamedata/schema/gen3_config.toml"),
        "version = 1\nkotlin_class = 'NativeConfig'\nkotlin_file = 'NativeReader.kt'\n\n[field.a]\nkotlin = 'Long'\ntype = 'long'\n\n[field.b]\nkotlin = 'String'\ntype = 'string'\ndefault = 'x'\n",
    )
    .unwrap();
    let err = gen(&dir).unwrap_err();
    assert!(err.contains("`b` is Int in NativeConfig, String in the schema"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn happy_skeleton_generates_all_three_outputs() {
    let dir = skeleton();
    let (outputs, summary) = gamedata::generate(&dir).map(|(o, s)| (o, s)).unwrap();
    assert_eq!(summary, "1 configs, 1 records");
    assert_eq!(outputs.len(), 3);
    let kt = &outputs[0].1;
    assert!(kt.contains("val NATIVE_GAME = NativeConfig("), "{kt}");
    assert!(kt.contains("a = 0x02024284L,"), "{kt}");
    let h = &outputs[2].1;
    assert!(h.contains("static const struct pd_config PD_GAME = {"), "{h}");
    assert!(h.contains(".a = 0x02024284,"), "{h}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_gamedata_loads_and_matches_counts() {
    let (outputs, summary) = gamedata::generate(&root()).unwrap();
    assert_eq!(summary, "104 configs, 31 records");
    assert_eq!(outputs.len(), 3);
}
