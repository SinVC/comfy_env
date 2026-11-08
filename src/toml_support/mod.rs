use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value, value,
};

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
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub run: RunConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    pub extra_args: RunExtraArgs,
    pub sync_before_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RunExtraArgs {
    Args(Vec<String>),
    Legacy(String),
}

impl Default for RunExtraArgs {
    fn default() -> Self {
        Self::Args(Vec::new())
    }
}

impl RunExtraArgs {
    pub fn to_args(&self) -> Vec<OsString> {
        match self {
            Self::Args(args) => args.iter().map(OsString::from).collect(),
            Self::Legacy(text) => text.split_whitespace().map(OsString::from).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PolicyConfig {
    #[serde(default = "default_core_packages")]
    pub core_packages: Vec<String>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            core_packages: default_core_packages(),
        }
    }
}

fn default_core_packages() -> Vec<String> {
    [
        "torch",
        "torchvision",
        "torchaudio",
        "xformers",
        "triton",
        "onnxruntime",
        "onnxruntime-gpu",
        "numpy",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
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
    pub candidate_root: PathBuf,
}

impl Default for ConfigRuntime {
    fn default() -> Self {
        Self {
            python: None,
            prod_env: ".venv-prod".into(),
            candidate_root: ".venv-candidate".into(),
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
    Replace,
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

pub fn set_import_config(path: &Path, comfyui_dir: &Path, python: &str) -> Result<()> {
    let text = crate::fs_support::read_text_if_exists(path)?.unwrap_or_default();
    let mut doc = text.parse::<DocumentMut>()?;
    set_runtime_config(&mut doc, comfyui_dir, python)?;
    crate::fs_support::atomic_write(path, doc.to_string().as_bytes())
}

pub fn config_for_init(path: &Path) -> Result<(DocumentMut, RuntimeConfig)> {
    let doc = read_or_template(path)?;
    let config = toml_edit::de::from_str(&doc.to_string())
        .with_context(|| format!("invalid configuration for {}", path.display()))?;
    Ok((doc, config))
}

pub fn configure_init(doc: &mut DocumentMut, comfyui_dir: &Path, python: &str) -> Result<()> {
    set_runtime_config(doc, comfyui_dir, python)?;
    for section in ["tx", "policy", "ops", "run"] {
        ensure_table(doc.as_table_mut(), section)?;
    }
    Ok(())
}

pub fn project_for_init(path: &Path, python: &str, marker: &str) -> Result<DocumentMut> {
    let mut doc = read_or_template(path)?;
    let project = ensure_table(doc.as_table_mut(), "project")?;
    set_value(
        project,
        "requires-python",
        Value::from(format!("=={python}.*")),
    );
    let tool = ensure_table(doc.as_table_mut(), "tool")?;
    let uv = ensure_table(tool, "uv")?;
    set_value(
        uv,
        "environments",
        Value::Array([marker].into_iter().collect()),
    );
    Ok(doc)
}

fn read_or_template(path: &Path) -> Result<DocumentMut> {
    let (text, source) = match crate::fs_support::read_text_if_exists(path)? {
        Some(text) => (text, path.to_owned()),
        None => {
            let template = path.with_extension("toml.template");
            let text = std::fs::read_to_string(&template)
                .with_context(|| format!("failed to read {}", template.display()))?;
            (text, template)
        }
    };
    text.parse()
        .with_context(|| format!("failed to parse {}", source.display()))
}

fn set_runtime_config(doc: &mut DocumentMut, comfyui_dir: &Path, python: &str) -> Result<()> {
    let comfyui_dir = comfyui_dir
        .to_str()
        .context("ComfyUI path is not valid UTF-8")?;
    let paths = ensure_table(doc.as_table_mut(), "paths")?;
    set_value(paths, "comfyui_dir", Value::from(comfyui_dir));
    let runtime = ensure_table(doc.as_table_mut(), "runtime")?;
    set_value(runtime, "python", Value::from(python));
    Ok(())
}

fn ensure_table<'a>(parent: &'a mut dyn TableLike, key: &str) -> Result<&'a mut dyn TableLike> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .with_context(|| format!("{key} must be a table"))
}

fn set_value(table: &mut dyn TableLike, key: &str, mut new: Value) {
    if let Some(old) = table.get_mut(key).and_then(Item::as_value_mut) {
        *new.decor_mut() = old.decor().clone();
        *old = new;
    } else {
        table.insert(key, Item::Value(new));
    }
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

    if mode == GroupEditMode::Replace {
        array.clear();
        for spec in specs {
            array.push(spec.as_str());
        }
        crate::fs_support::atomic_write(path, doc.to_string().as_bytes())?;
        return Ok(());
    }

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

pub fn upsert_uv_overrides(path: &Path, pins: &[String]) -> crate::domain::AppResult<()> {
    let mut doc = read_project(path)?;
    let tool = doc
        .as_table_mut()
        .entry("tool")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .context("tool must be a table")?;
    let uv = tool
        .entry("uv")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .context("tool.uv must be a table")?;
    let overrides = uv
        .entry("override-dependencies")
        .or_insert(value(Array::new()))
        .as_array_mut()
        .context("tool.uv.override-dependencies must be an array")?;
    let names: HashSet<_> = pins.iter().map(|pin| normalize_package_name(pin)).collect();
    overrides.retain(|entry| {
        !entry
            .as_str()
            .is_some_and(|spec| names.contains(&normalize_package_name(spec)))
    });
    for pin in pins {
        overrides.push(pin.as_str());
    }
    crate::fs_support::atomic_write(path, doc.to_string().as_bytes())?;
    Ok(())
}

pub fn remove_dependency_group(path: &Path, group: &str) -> Result<()> {
    let mut doc = read_project(path)?;
    if let Some(groups) = doc
        .get_mut("dependency-groups")
        .and_then(Item::as_table_like_mut)
    {
        groups.remove(group);
    }
    crate::fs_support::atomic_write(path, doc.to_string().as_bytes())
}

pub fn configure_torch_index(path: &Path, url: &str) -> Result<String> {
    let mut doc = read_project(path)?;
    let uv = doc
        .as_table_mut()
        .entry("tool")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("tool must be a table")?
        .entry("uv")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("tool.uv must be a table")?;

    let indexes = uv
        .entry("index")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .context("tool.uv.index must be an array of tables")?;
    let base = torch_index_name(url);
    let mut name = base.clone();
    let mut suffix = 2;
    loop {
        let existing_url_matches = indexes
            .iter()
            .find(|index| index.get("name").and_then(Item::as_str) == Some(&name))
            .map(|index| index.get("url").and_then(Item::as_str) == Some(url));
        match existing_url_matches {
            Some(true) => break,
            Some(false) => {
                name = format!("{base}-{suffix}");
                suffix += 1;
            }
            None => {
                let mut index = Table::new();
                index["name"] = value(name.as_str());
                index["url"] = value(url);
                index["explicit"] = value(true);
                indexes.push(index);
                break;
            }
        }
    }

    let sources = uv
        .entry("sources")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("tool.uv.sources must be a table")?;
    for package in ["torch", "torchvision", "torchaudio"] {
        let mut source = InlineTable::new();
        source.insert("index", Value::from(name.as_str()));
        sources[package] = Item::Value(Value::InlineTable(source));
    }

    crate::fs_support::atomic_write(path, doc.to_string().as_bytes())?;
    Ok(name)
}

fn torch_index_name(url: &str) -> String {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    let segment = path.rsplit('/').next().unwrap_or("index");
    let slug: String = segment
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    format!("pytorch-{}", if slug.is_empty() { "index" } else { slug })
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
