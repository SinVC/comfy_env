use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use toml_edit::{Array, DocumentMut, Item, Table, value};

#[derive(Debug, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub paths: ConfigPaths,
    #[serde(default)]
    pub runtime: ConfigRuntime,
    #[serde(default)]
    pub tx: TxConfig,
    #[serde(default)]
    pub ops: OpsConfig,
}

#[derive(Debug, Default, Deserialize)]
pub struct OpsConfig {
    pub retention_count: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ConfigPaths {
    pub comfyui_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ConfigRuntime {
    pub python: Option<String>,
    pub prod_env: PathBuf,
}

impl Default for ConfigRuntime {
    fn default() -> Self {
        Self {
            python: None,
            prod_env: ".venv-prod".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct TxConfig {
    pub timeout_seconds: u32,
    smoke_test_cmd: Option<String>,
    pub smoke_test: Option<SmokeTestConfig>,
}

impl Default for TxConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 120,
            smoke_test_cmd: None,
            smoke_test: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SmokeTestConfig {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupEditMode {
    UpsertExact,
    RemoveNames,
}

pub fn read_config(path: &Path) -> Result<Option<RuntimeConfig>> {
    let Some(text) = crate::fs_support::read_text_if_exists(path)? else {
        return Ok(None);
    };

    let config = toml_edit::de::from_str::<RuntimeConfig>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if config
        .tx
        .smoke_test_cmd
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        eprintln!("warning: tx.smoke_test_cmd is deprecated and ignored; use [tx.smoke_test]");
    }
    Ok(Some(config))
}

pub fn read_dependency_group(path: &Path, group: &str) -> Result<Vec<String>> {
    dependency_group(&read_project(path)?, group)
}

pub fn read_project(path: &Path) -> Result<DocumentMut> {
    let text = crate::fs_support::read_text_if_exists(path)?.unwrap_or_default();
    parse_pyproject(path, &text)
}

pub fn dependency_group(doc: &DocumentMut, group: &str) -> Result<Vec<String>> {
    let Some(groups) = doc.get("dependency-groups") else {
        return Ok(Vec::new());
    };
    let Some(item) = groups
        .as_table_like()
        .context("dependency-groups must be a table")?
        .get(group)
    else {
        return Ok(Vec::new());
    };
    let Some(array) = item.as_array() else {
        bail!("dependency-groups.{group} is not an array");
    };

    Ok(array
        .iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect())
}

pub fn rewrite_dependency_group(
    path: &Path,
    group: &str,
    mode: GroupEditMode,
    specs: &[String],
) -> crate::domain::AppResult<()> {
    let mut doc = read_project(path)?;
    let groups = doc
        .as_table_mut()
        .entry("dependency-groups")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .context("dependency-groups must be a table")?;
    let array = groups
        .entry(group)
        .or_insert(value(Array::new()))
        .as_array_mut()
        .with_context(|| format!("dependency-groups.{group} must be an array"))?;

    // Walk backwards so only the last request for each normalized name survives.
    let mut names = HashSet::new();
    let mut unique = Vec::new();
    for spec in specs.iter().rev() {
        if names.insert(normalize_package_name(spec)) && mode == GroupEditMode::UpsertExact {
            unique.push(spec);
        }
    }
    if mode == GroupEditMode::RemoveNames {
        let existing: HashSet<_> = array
            .iter()
            .filter_map(|v| v.as_str())
            .map(normalize_package_name)
            .collect();
        if let Some(missing) = specs
            .iter()
            .find(|name| !existing.contains(&normalize_package_name(name)))
        {
            return Err(crate::domain::AppError::usage(format!(
                "The dependency `{missing}` could not be found in `dependency-groups.{group}`"
            )));
        }
    }
    array.retain(|entry| {
        !entry
            .as_str()
            .is_some_and(|spec| names.contains(&normalize_package_name(spec)))
    });
    for spec in unique.into_iter().rev() {
        array.push(spec.as_str());
    }

    crate::fs_support::atomic_write(path, doc.to_string().as_bytes())?;
    Ok(())
}

pub fn normalize_package_name(spec: &str) -> String {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let token = trimmed
        .split(['<', '>', '=', '!', '~', ';', '[', '@'])
        .next()
        .unwrap_or(trimmed)
        .trim();

    let mut normalized = String::with_capacity(token.len());
    let mut previous_dash = false;
    for ch in token.chars() {
        let mapped = if matches!(ch, '-' | '_' | '.') {
            '-'
        } else {
            ch.to_ascii_lowercase()
        };
        if mapped == '-' {
            if previous_dash {
                continue;
            }
            previous_dash = true;
            normalized.push(mapped);
        } else {
            previous_dash = false;
            normalized.push(mapped);
        }
    }

    normalized.trim_matches('-').to_string()
}

pub fn is_package_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bytes.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'.' | b'_'))
}

pub fn is_exact_pin(spec: &str) -> bool {
    let Some((name, version)) = spec.split_once("==") else {
        return false;
    };

    is_package_name(name)
        && !version.is_empty()
        && version
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'!' | b'+' | b'-' | b'_'))
}

fn parse_pyproject(path: &Path, text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{GroupEditMode, is_exact_pin, normalize_package_name, rewrite_dependency_group};
    use crate::fs_support::read_text_if_exists;

    fn temp_pyproject(contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("pyproject.toml"), contents).expect("write pyproject");
        dir
    }

    #[test]
    fn normalize_package_names_matches_policy() {
        assert_eq!(normalize_package_name("NumPy"), "numpy");
        assert_eq!(normalize_package_name("foo_bar.baz"), "foo-bar-baz");
        assert_eq!(normalize_package_name("demo @ https://x"), "demo");
    }

    #[test]
    fn exact_pin_requires_pkg_and_version() {
        assert!(is_exact_pin("numpy==1.26.4"));
        assert!(!is_exact_pin("numpy>=1.26"));
        assert!(!is_exact_pin("==1.26.4"));
        assert!(!is_exact_pin("numpy=="));
    }

    #[test]
    fn rewrite_preserves_quoted_overrides_key() {
        let dir = temp_pyproject(
            r#"[dependency-groups]
"overrides" = []
"#,
        );
        let path = dir.path().join("pyproject.toml");
        rewrite_dependency_group(
            &path,
            "overrides",
            GroupEditMode::UpsertExact,
            &[String::from("transformers==4.44.0")],
        )
        .expect("rewrite");

        let text = read_text_if_exists(&path).expect("read").expect("exists");
        assert!(text.contains("\"overrides\" = ["));
    }

    #[test]
    fn rewrite_dedupes_existing_entries_and_uses_last_wins() {
        let dir = temp_pyproject(
            r#"[dependency-groups]
overrides = [
  "numpy==1.26.3",
  "numpy==1.26.4",
  "pillow==10.0.0",
]
"#,
        );
        let path = dir.path().join("pyproject.toml");
        rewrite_dependency_group(
            &path,
            "overrides",
            GroupEditMode::UpsertExact,
            &[String::from("numpy==1.26.1"), String::from("numpy==1.26.2")],
        )
        .expect("rewrite");

        let text = read_text_if_exists(&path).expect("read").expect("exists");
        assert!(text.contains("numpy==1.26.2"));
        assert!(!text.contains("numpy==1.26.1"));
        assert!(!text.contains("numpy==1.26.3"));
        assert!(!text.contains("numpy==1.26.4"));
        assert!(text.contains("pillow==10.0.0"));
    }

    #[test]
    fn rewrite_remove_names_removes_all_duplicates() {
        let dir = temp_pyproject(
            r#"[dependency-groups]
overrides = [
  "numpy==1.26.3",
  "numpy==1.26.4",
  "pillow==10.0.0",
]
"#,
        );
        let path = dir.path().join("pyproject.toml");
        rewrite_dependency_group(
            &path,
            "overrides",
            GroupEditMode::RemoveNames,
            &[String::from("numpy")],
        )
        .expect("rewrite");

        let text = read_text_if_exists(&path).expect("read").expect("exists");
        assert!(!text.contains("numpy==1.26.3"));
        assert!(!text.contains("numpy==1.26.4"));
        assert!(text.contains("pillow==10.0.0"));
    }
}
