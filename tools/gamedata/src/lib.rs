//! gamedata: writes the per-game code every PokeDaisy platform builds from `gamedata/`
//! (see gamedata/README.md). This is the Rust port of the old `scripts/gen_gamedata.py`;
//! it checks every file against the schemas in `gamedata/schema/` (and each schema against
//! the Kotlin class it describes), resolves `base` / `from`, and writes:
//!   - the app's NativeConfigsGen.kt: one Kotlin val per config, comments included;
//!   - the app's GameDataRecordsGen.kt: the records and lists in gamedata/gen3/records/;
//!   - each target in gamedata/targets/: 3ds.toml -> the 3DS / Linux companion's
//!     3ds/core/pd_configs_gen.h (its struct pd_config per config).
//!
//!     cargo run -p gamedata              # write
//!     cargo run -p gamedata -- --check   # exit 1 if the written files are stale (CI)
//!
//! The golden tests (tests/) compare the output byte for byte with the committed files.

pub mod c_target;
pub mod config;
pub mod emit_kt;
pub mod kotlin_src;
pub mod records;
pub mod schema;
pub mod toml_raw;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i64 = 1;
pub const CONFIG_SCHEMA: &str = "gen3_config";
pub const GROUP_ORDER: [&str; 3] = ["retail", "ports", "hacks"];
pub const RETAIL_ORDER: [&str; 4] = ["firered", "leafgreen", "emerald", "ruby_sapphire"];
pub const META_KEYS: [&str; 4] = ["base", "private", "lazy", "port"];
pub const NONE: &str = "none"; // a nullable field's null

pub const DATA_REL: &str = "app/src/main/kotlin/com/pokedaisy/app/companion/data";
pub const KOTLIN_OUT_REL: &str = "app/src/main/kotlin/com/pokedaisy/app/companion/data/NativeConfigsGen.kt";
pub const RECORDS_OUT_REL: &str = "app/src/main/kotlin/com/pokedaisy/app/companion/data/GameDataRecordsGen.kt";

#[derive(Debug)]
pub struct GameDataError(pub String);

impl std::fmt::Display for GameDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GameDataError {}

pub fn fail(r#where: &str, msg: impl Into<String>) -> GameDataError {
    GameDataError(format!("{}: {}", r#where, msg.into()))
}

pub fn relpath(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn gamedata(root: &Path) -> PathBuf {
    root.join("gamedata")
}

pub fn schemas_dir(root: &Path) -> PathBuf {
    gamedata(root).join("schema")
}

pub fn records_dir(root: &Path) -> PathBuf {
    gamedata(root).join("gen3").join("records")
}

pub fn targets_dir(root: &Path) -> PathBuf {
    gamedata(root).join("targets")
}

/// Every output file and its text, with the "N configs, M records" summary. Output order
/// matches the old script: the two Kotlin files, then each target in gamedata/targets/.
pub fn generate(root: &Path) -> Result<(Vec<(PathBuf, String)>, String), GameDataError> {
    let mut schemas = HashMap::new();
    let schema = schema::load_schema(root, CONFIG_SCHEMA, &mut schemas)?;
    let configs = config::load_configs(root, &schema, &mut schemas)?;
    let records = records::load_records(root, &mut schemas)?;
    let summary = format!("{} configs, {} records", configs.len(), records.len());
    let mut outputs = vec![
        (root.join(KOTLIN_OUT_REL), emit_kt::kotlin(&configs, &schema, &mut schemas)),
        (root.join(RECORDS_OUT_REL), emit_kt::record_kotlin(&records, &mut schemas)),
    ];
    let mut targets = vec![];
    for entry in std::fs::read_dir(targets_dir(root)).map_err(|e| fail("gamedata/targets", e.to_string()))? {
        let path = entry.map_err(|e| fail("gamedata/targets", e.to_string()))?.path();
        if path.extension().map_or(false, |e| e == "toml") {
            targets.push(path);
        }
    }
    targets.sort();
    for target in targets {
        outputs.push(c_target::c_target(&target, &configs, &records, &schema, &mut schemas, root)?);
    }
    Ok((outputs, summary))
}
