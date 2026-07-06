use directories::ProjectDirs;
use std::path::PathBuf;

/// Resolved filesystem locations. Config lives at `~/.config/myelin`, data at
/// `~/.local/share/myelin` unless overridden via `MYELIN_DATA_DIR` — same
/// override convention NexusContext used for `NEXUS_CACHE_DIR`.
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Unix domain socket paths are capped at ~108 bytes (`SUN_LEN`), so the
    /// control socket lives under the platform runtime dir, not `data_dir`.
    pub runtime_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let dirs = ProjectDirs::from("", "", "myelin")
            .expect("could not determine a home directory for the current user");

        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = std::env::var_os("MYELIN_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.data_dir().to_path_buf());
        let runtime_dir = dirs
            .runtime_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| data_dir.clone());

        Self {
            config_dir,
            data_dir,
            runtime_dir,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn control_socket(&self) -> PathBuf {
        self.runtime_dir.join("myelin.sock")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("myelind.log")
    }
}
