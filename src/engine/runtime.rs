// src/engine/runtime.rs

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEnvironment {
    Host,
    Flatpak,
    Snap,
}

pub struct RuntimeAdapter {
    env_type: RuntimeEnvironment,
    home_dir: PathBuf,
}

impl RuntimeAdapter {
    pub fn detect() -> Self {
        let home_dir = PathBuf::from(env::var("HOME").expect("HOME environment variable must be set"));
        
        let env_type = if Path::new("/.flatpak-info").exists() {
            RuntimeEnvironment::Flatpak
        } else if env::var("SNAP").is_ok() {
            RuntimeEnvironment::Snap
        } else {
            RuntimeEnvironment::Host
        };

        Self { env_type, home_dir }
    }

    /// Resolves the secure configuration path depending on the host sandbox rules
    pub fn config_dir(&self) -> PathBuf {
        match self.env_type {
            RuntimeEnvironment::Snap => {
                if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
                    PathBuf::from(snap_user_data).join(".config/lens-for-gnome")
                } else {
                    self.home_dir.join(".config/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Flatpak => {
                if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
                    PathBuf::from(xdg_config).join("lens-for-gnome")
                } else {
                    self.home_dir.join(".config/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Host => self.home_dir.join(".config/lens-for-gnome"),
        }
    }

    /// Resolves the secure shared data path tracking directories
    pub fn data_dir(&self) -> PathBuf {
        match self.env_type {
            RuntimeEnvironment::Snap => {
                if let Ok(snap_user_common) = env::var("SNAP_USER_COMMON") {
                    PathBuf::from(snap_user_common).join(".local/share/lens-for-gnome")
                } else {
                    self.home_dir.join(".local/share/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Flatpak => {
                if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
                    PathBuf::from(xdg_data).join("lens-for-gnome")
                } else {
                    self.home_dir.join(".local/share/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Host => self.home_dir.join(".local/share/lens-for-gnome"),
        }
    }

    /// Resolves the secure runtime/state socket path boundaries
    pub fn state_dir(&self) -> PathBuf {
        match self.env_type {
            RuntimeEnvironment::Snap => {
                if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
                    PathBuf::from(snap_user_data).join(".local/state/lens-for-gnome")
                } else {
                    self.home_dir.join(".local/state/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Flatpak => {
                if let Ok(xdg_state) = env::var("XDG_STATE_HOME") {
                    PathBuf::from(xdg_state).join("lens-for-gnome")
                } else {
                    self.home_dir.join(".local/state/lens-for-gnome")
                }
            }
            RuntimeEnvironment::Host => self.home_dir.join(".local/state/lens-for-gnome"),
        }
    }

    /// Abstracts the building of system settings configuration utilities
    pub fn build_gsettings_cmd(&self) -> Command {
        let mut cmd = Command::new("gsettings");
        
        let real_home = if self.env_type == RuntimeEnvironment::Snap {
            env::var("SNAP_REAL_HOME").unwrap_or_else(|_| self.home_dir.to_string_lossy().to_string())
        } else {
            self.home_dir.to_string_lossy().to_string()
        };

        let ext_schema = PathBuf::from(real_home).join(".local/share/gnome-shell/extensions/lens-for-gnome@cwittenberg/schemas");

        if Path::new("schemas").exists() {
            cmd.env("GSETTINGS_SCHEMA_DIR", "schemas");
        } else if self.env_type == RuntimeEnvironment::Snap {
            let snap_dir = env::var("SNAP").unwrap_or_default();
            let bundled_schema = PathBuf::from(snap_dir).join("schemas");
            
            if bundled_schema.exists() {
                cmd.env("GSETTINGS_SCHEMA_DIR", bundled_schema);
            } else if ext_schema.exists() {
                cmd.env("GSETTINGS_SCHEMA_DIR", ext_schema);
            }
        } else if ext_schema.exists() {
            cmd.env("GSETTINGS_SCHEMA_DIR", ext_schema);
        }
        
        cmd
    }

    /// Secure execution wrapper ensuring commands run under flatpak sandboxing or host contexts smoothly
    pub fn create_system_command(&self, binary: &str) -> Command {
        match self.env_type {
            RuntimeEnvironment::Flatpak | RuntimeEnvironment::Snap | RuntimeEnvironment::Host => {
                Command::new(binary)
            }
        }
    }
}