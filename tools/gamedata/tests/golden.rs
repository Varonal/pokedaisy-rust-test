//! Golden tests: the generator's output must equal the committed files byte for byte,
//! and `--check` semantics must match (stale -> failure, up to date -> success).

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn outputs_match_the_committed_files() {
    let root = root();
    let (outputs, summary) = gamedata::generate(&root).expect("generate");
    assert_eq!(summary, "104 configs, 31 records");
    assert_eq!(outputs.len(), 3);
    for (path, text) in &outputs {
        let committed = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&committed, text, "{} differs from the committed file", path.display());
    }
}

#[test]
fn check_mode_reports_up_to_date() {
    // The same comparison main()'s --check does, as a library call.
    let root = root();
    let (outputs, _) = gamedata::generate(&root).unwrap();
    let stale: Vec<_> = outputs
        .iter()
        .filter(|(p, t)| std::fs::read_to_string(p).map(|c| c != *t).unwrap_or(true))
        .map(|(p, _)| gamedata::relpath(p, &root))
        .collect();
    assert!(stale.is_empty(), "stale: {stale:?}");
}

#[test]
fn binary_check_exits_zero_on_a_clean_tree() {
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .join(format!("target/debug/gen_gamedata"));
    if !bin.exists() {
        // cargo test builds the lib; the bin may not be built yet in some flows.
        return;
    }
    let out = std::process::Command::new(&bin).arg("--check").current_dir(root()).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("3 files up to date"));
}

/// Cross-check against the reference Python generator, when python3 is available:
/// run it in a scratch copy of the repo and diff its three outputs against ours.
#[test]
fn parity_with_the_python_generator_when_available() {
    let py = std::process::Command::new("python3").arg("--version").output();
    let Ok(_) = py else { return };
    let root = root();
    let scratch = std::env::temp_dir().join(format!("gamedata-parity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    // copy only what the python script reads
    for dir in ["gamedata", "app/src/main/kotlin/com/pokedaisy/app/companion/data", "scripts", "3ds/core"] {
        let src = root.join(dir);
        let dst = scratch.join(dir);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        copy_dir(&src, &dst);
    }
    // python writes the outputs in place
    let out = std::process::Command::new("python3")
        .arg(scratch.join("scripts/gen_gamedata.py"))
        .current_dir(&scratch)
        .output()
        .expect("run python gen_gamedata");
    assert!(out.status.success(), "python failed: {}", String::from_utf8_lossy(&out.stderr));
    let (outputs, _) = gamedata::generate(&root).unwrap();
    for (path, text) in &outputs {
        let rel = gamedata::relpath(path, &root);
        let py_text = std::fs::read_to_string(scratch.join(&rel)).unwrap();
        assert_eq!(&py_text, text, "{rel}: rust and python outputs differ");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap().path();
        if e.is_dir() {
            copy_dir(&e, &dst.join(e.file_name().unwrap()));
        } else {
            std::fs::copy(&e, dst.join(e.file_name().unwrap())).unwrap();
        }
    }
}
