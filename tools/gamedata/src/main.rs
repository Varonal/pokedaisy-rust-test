//! gen_gamedata: writes the per-game code every platform builds from gamedata/
//! (see gamedata/README.md).
//!
//!     gen_gamedata            # write
//!     gen_gamedata --check    # exit 1 if the written files are stale (CI)

use std::path::PathBuf;
use std::process::ExitCode;

fn root() -> PathBuf {
    // The crate lives in tools/gamedata; the repo root is two levels up.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent().unwrap().parent().unwrap().to_path_buf()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let root = root();
    match gamedata::generate(&root) {
        Ok((outputs, summary)) => {
            if check {
                let stale: Vec<String> = outputs
                    .iter()
                    .filter(|(path, text)| {
                        let current = std::fs::read_to_string(path).unwrap_or_default();
                        current != *text
                    })
                    .map(|(path, _)| gamedata::relpath(path, &root))
                    .collect();
                if !stale.is_empty() {
                    eprintln!("gen_gamedata: {} stale or edited by hand - run `make gamedata` and commit the result", stale.join(", "));
                    return ExitCode::FAILURE;
                }
                println!("gen_gamedata: {} files up to date ({summary})", outputs.len());
                return ExitCode::SUCCESS;
            }
            for (path, text) in &outputs {
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!("gen_gamedata: {}: {e}", gamedata::relpath(path, &root));
                    return ExitCode::FAILURE;
                }
            }
            let names: Vec<String> = outputs.iter().map(|(p, _)| gamedata::relpath(p, &root)).collect();
            println!("gen_gamedata: {summary} -> {}", names.join(", "));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen_gamedata: {e}");
            ExitCode::FAILURE
        }
    }
}
