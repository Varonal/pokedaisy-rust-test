//! Reads a Kotlin data class's constructor fields out of the app's sources, so the schemas
//! can be checked against the classes they describe.

use crate::{fail, GameDataError, DATA_REL};
use regex::Regex;
use std::path::Path;

pub struct KtField {
    pub name: String,
    pub kotlin: String,
    pub default: Option<String>,
    pub comments: Vec<String>,
}

fn skip_string(s: &str, j: usize) -> usize {
    let b = s.as_bytes();
    let q = b[j];
    let mut j = j + 1;
    while b[j] != q {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    j + 1
}

/// s[i] opens a bracket; the index just past its match (comments and strings skipped).
fn match_paren(s: &str, i: usize) -> Result<usize, GameDataError> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut j = i;
    while j < s.len() {
        if s[j..].starts_with("//") {
            j = s[j..].find('\n').map(|o| j + o).unwrap_or(s.len());
            continue;
        }
        if s[j..].starts_with("/*") {
            j = s[j..].find("*/").map(|o| j + o + 2).unwrap_or(s.len());
            continue;
        }
        if b[j] == b'"' || b[j] == b'\'' {
            j = skip_string(s, j);
            continue;
        }
        if matches!(b[j], b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(b[j], b')' | b']' | b'}') {
            depth -= 1;
            if depth == 0 {
                return Ok(j + 1);
            }
        }
        j += 1;
    }
    Err(GameDataError("unbalanced".into()))
}

/// [{name, kotlin, default, comments}] of a data class's constructor.
pub fn kotlin_class_fields(root: &Path, kt_file: &str, cls: &str) -> Result<Vec<KtField>, GameDataError> {
    let path = root.join(DATA_REL).join(kt_file);
    let src = std::fs::read_to_string(&path).map_err(|e| fail(&crate::relpath(&path, root), e.to_string()))?;
    let re = Regex::new(&format!(r"(?m)^data class {cls}\(")).unwrap();
    let m = re.find(&src).ok_or_else(|| fail(kt_file, format!("no `data class {cls}(` in {kt_file}")))?;
    let open = m.end() - 1;
    let close = match_paren(&src, open)?;
    let body = &src[m.end()..close - 1];
    let mut fields = vec![];
    let mut comments: Vec<String> = vec![];
    let mut in_doc = false;
    let field_re = Regex::new(r"val (\w+): ([^=]+?)(?:\s*=\s*(.*?))?,?(?:\s*//\s*(.*))?$").unwrap();
    for line in body.split('\n') {
        let t = line.trim();
        if in_doc || t.starts_with("/*") {
            in_doc = !t.ends_with("*/");
            let mut text = Regex::new(r"^/\*\*?|\*/$").unwrap().replace_all(t, "").trim().to_string();
            if text.starts_with('*') {
                text = text[1..].trim().to_string();
            }
            if !text.is_empty() {
                comments.push(text);
            }
            continue;
        }
        if let Some(c) = t.strip_prefix("//") {
            comments.push(c.trim().to_string());
            continue;
        }
        if let Some(f) = field_re.captures(t) {
            let mut cs = comments.clone();
            if let Some(c4) = f.get(4) {
                cs.push(c4.as_str().to_string());
            }
            fields.push(KtField {
                name: f[1].to_string(),
                kotlin: f[2].trim().to_string(),
                default: f.get(3).map(|d| d.as_str().to_string()),
                comments: cs,
            });
            comments = vec![];
        }
    }
    Ok(fields)
}
