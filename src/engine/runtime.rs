// src/engine/runtime.rs

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEnvironment {
    Host,
    Flatpak,
}

pub struct RuntimeAdapter {
    env_type: RuntimeEnvironment,
    home_dir: PathBuf,
}

impl RuntimeAdapter {
    pub fn detect() -> Self {
        let home_dir = PathBuf::from(env::var("HOME").expect("HOME environment variable must be set"));
        
        // Detect if running inside a Flatpak Sandbox environment
        let env_type = if Path::new("/.flatpak-info").exists() {
            RuntimeEnvironment::Flatpak
        } else {
            RuntimeEnvironment::Host
        };

        Self { env_type, home_dir }
    }

    /// Resolves the secure configuration path depending on the host sandbox rules
    pub fn config_dir(&self) -> PathBuf {
        match self.env_type {
            RuntimeEnvironment::Flatpak => {
                // Inside Flatpak, XDG_CONFIG_HOME is redirected to sandboxed app state
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
        
        // Check for development schemas relative to the deployment binary workspace path
        if Path::new("schemas").exists() {
            cmd.env("GSETTINGS_SCHEMA_DIR", "schemas");
        } else if self.env_type == RuntimeEnvironment::Host {
            let ext_schema = self.home_dir.join(".local/share/gnome-shell/extensions/lens-for-gnome@cwittenberg/schemas");
            if ext_schema.exists() {
                cmd.env("GSETTINGS_SCHEMA_DIR", ext_schema);
            }
        }
        
        cmd
    }

    /// Secure execution wrapper ensuring commands run under flatpak sandboxing or host contexts smoothly
    pub fn create_system_command(&self, binary: &str) -> Command {
        match self.env_type {
            RuntimeEnvironment::Flatpak => {
                // If executing within Flatpak container context, some host utilities (like tool portals) 
                // might need to fall-through via portal invocation if they are not explicitly packed in the manifest.
                // However, bundling ffmpeg/tesseract inside the flatpak-build manifest ensures absolute isolation.
                // We default to local container execution first.
                Command::new(binary)
            }
            RuntimeEnvironment::Host => Command::new(binary),
        }
    }
}