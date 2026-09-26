use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn default_port() -> u16 {
    8787
}
fn default_shared_dir() -> String {
    "./files".to_string()
}
fn default_max_upload() -> u64 {
    1024 * 1024 * 1024 // 1 GiB
}
fn default_true() -> bool {
    true
}
fn default_history_size() -> usize {
    50
}
fn default_data_dir() -> String {
    "~/.cliplan".to_string()
}

/// Application configuration. Loaded from `~/.cliplan/config.toml` (or a
/// path given with `--config`) and then overridden by CLI flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_shared_dir")]
    pub shared_directory: String,

    #[serde(default = "default_max_upload")]
    pub max_upload_size: u64,

    #[serde(default = "default_true")]
    pub clipboard_sync: bool,

    #[serde(default = "default_true")]
    pub clipboard_history: bool,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard_history_size: Option<usize>,

    #[serde(default = "default_true")]
    pub discovery: bool,

    /// Directory used for the sqlite db, uploaded files, and clipboard
    /// history blobs. Defaults to `~/.cliplan`.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    /// Device display name announced during discovery & pairing.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            shared_directory: default_shared_dir(),
            max_upload_size: default_max_upload(),
            clipboard_sync: true,
            clipboard_history: true,
            clipboard_history_size: None,
            discovery: true,
            data_dir: default_data_dir(),
            device_name: None,
        }
    }
}

impl Config {
    pub fn history_size(&self) -> usize {
        self.clipboard_history_size.unwrap_or_else(default_history_size)
    }

    pub fn data_dir_path(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }

    pub fn shared_dir_path(&self) -> PathBuf {
        let expanded = expand_tilde(&self.shared_directory);
        if expanded.is_absolute() {
            expanded
        } else {
            self.data_dir_path().join(expanded)
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir_path().join("cliplan.db")
    }

    pub fn files_dir(&self) -> PathBuf {
        self.shared_dir_path()
    }

    /// Load config from disk, falling back to defaults if the file does not
    /// exist. Malformed config files are reported as errors rather than
    /// silently ignored.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {e}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.data_dir_path())?;
        std::fs::create_dir_all(self.files_dir())?;
        std::fs::create_dir_all(self.data_dir_path().join("history"))?;
        Ok(())
    }
}

fn expand_tilde(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest);
        }
    } else if input == "~" {
        if let Some(home) = dirs_home() {
            return home;
        }
    }
    PathBuf::from(input)
}

/// Minimal home-directory lookup so we don't need an extra crate just for
/// this one thing.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
