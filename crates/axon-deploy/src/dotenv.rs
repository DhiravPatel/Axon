//! `.env` file loader.
//!
//! Standard line-based format: `KEY=VALUE` per line. Empty lines and
//! lines starting with `#` are comments. Values may be quoted with `"..."`
//! to include spaces; quotes are stripped. Unlike full dotenv libraries,
//! we don't expand `${VAR}` references — that's a clarity vs. surprise
//! trade-off and the runtime can do it explicitly with `env_get` later.

use std::collections::BTreeMap;
use std::path::Path;

/// Parse `bytes` as dotenv content; return the key/value pairs in source
/// order (preserved via `BTreeMap` insertion). Invalid lines are
/// silently dropped with a comment line.
pub fn parse(bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim().to_string();
        if key.is_empty() {
            continue;
        }
        let mut value = line[eq + 1..].trim().to_string();
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            if value.len() >= 2 {
                value = value[1..value.len() - 1].to_string();
            }
        }
        out.insert(key, value);
    }
    out
}

/// Load a `.env` file into the process environment. Existing process env
/// values are NOT overwritten by default (the typical convention so a
/// deployment that bakes secrets at boot wins over the repo-checked-in
/// defaults). Returns the number of variables newly set.
pub fn load(path: impl AsRef<Path>, overwrite: bool) -> std::io::Result<usize> {
    let bytes = std::fs::read(path)?;
    let map = parse(&bytes);
    let mut set = 0;
    for (k, v) in map {
        if overwrite || std::env::var_os(&k).is_none() {
            std::env::set_var(&k, v);
            set += 1;
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_keys() {
        let m = parse(b"FOO=bar\nBAZ=42\n");
        assert_eq!(m.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(m.get("BAZ"), Some(&"42".to_string()));
    }

    #[test]
    fn parse_strips_double_and_single_quotes() {
        let m = parse(b"GREETING=\"hello world\"\nNAME='alice'\n");
        assert_eq!(m.get("GREETING"), Some(&"hello world".to_string()));
        assert_eq!(m.get("NAME"), Some(&"alice".to_string()));
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let m = parse(b"# a comment\n\nFOO=bar\n# another\n");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn parse_drops_malformed_lines() {
        let m = parse(b"no equals here\nVALID=ok\n=missing-key\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("VALID"), Some(&"ok".to_string()));
    }

    #[test]
    fn load_preserves_existing_env_by_default() {
        let key = format!("AXON_DEPLOY_TEST_DOTENV_{}", std::process::id());
        let path = std::env::temp_dir().join(format!("{}.env", key));
        std::env::set_var(&key, "from-process");
        std::fs::write(&path, format!("{key}=from-dotenv\n")).unwrap();

        load(&path, /* overwrite = */ false).unwrap();
        assert_eq!(std::env::var(&key).unwrap(), "from-process");

        load(&path, /* overwrite = */ true).unwrap();
        assert_eq!(std::env::var(&key).unwrap(), "from-dotenv");

        std::env::remove_var(&key);
        let _ = std::fs::remove_file(&path);
    }
}
