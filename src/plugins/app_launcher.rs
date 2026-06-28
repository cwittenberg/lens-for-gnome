// src/plugins/app_launcher.rs
use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub id: String,
}

pub struct AppLauncherPlugin {
    apps: Vec<DesktopApp>,
}

impl AppLauncherPlugin {
    pub fn new() -> Self {
        let mut apps = Vec::new();
        let mut seen_execs = std::collections::HashSet::new();

        let home = std::env::var("HOME").unwrap_or_default();
        
        // Expanded to include Flatpak and Snap system installations
        // as well as Flatpak /var/run/host sandbox mounts for correct containerized execution
        let paths = vec![
            "/usr/share/applications".to_string(),
            format!("{}/.local/share/applications", home),
            "/var/lib/flatpak/exports/share/applications".to_string(),
            format!("{}/.local/share/flatpak/exports/share/applications", home),
            "/var/lib/snapd/desktop/applications".to_string(),
            "/var/run/host/usr/share/applications".to_string(),
            "/var/run/host/var/lib/flatpak/exports/share/applications".to_string(),
            "/var/run/host/var/lib/snapd/desktop/applications".to_string(),
        ];

        for path in paths {
            if let Ok(entries) = fs::read_dir(Path::new(&path)) {
                for entry in entries.flatten() {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_file() {
                            let path_buf = entry.path();
                            if path_buf.extension().and_then(|s| s.to_str()) == Some("desktop") {
                                if let Some(app) = Self::parse_desktop_file(&path_buf) {
                                    // Deduplicate by the base executable to prevent terminal vs GUI duplicates
                                    let base_exec = app.exec.split_whitespace().next().unwrap_or(&app.exec).to_string();
                                    if !seen_execs.contains(&base_exec) {
                                        seen_execs.insert(base_exec);
                                        apps.push(app);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Self { apps }
    }

    fn parse_desktop_file(path: &Path) -> Option<DesktopApp> {
        let content = fs::read_to_string(path).ok()?;
        let mut name = String::new();
        let mut exec = String::new();
        let mut icon = None;
        let mut no_display = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Name=") && name.is_empty() {
                name = trimmed[5..].to_string();
            } else if trimmed.starts_with("Exec=") && exec.is_empty() {
                exec = trimmed[5..].to_string();
            } else if trimmed.starts_with("Icon=") && icon.is_none() {
                icon = Some(trimmed[5..].to_string());
            } else if trimmed.starts_with("NoDisplay=true") {
                no_display = true;
            }
        }

        if name.is_empty() || exec.is_empty() || no_display {
            return None;
        }

        Some(DesktopApp {
            id: path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            name,
            exec,
            icon,
        })
    }
}

impl PluginTool for AppLauncherPlugin {
    fn id(&self) -> &'static str { "plugin:app_launcher" }
    fn name(&self) -> &'static str { "App Launcher" }
    
    fn can_fast_handle(&self, query: &SearchQuery) -> bool {
        let q = query.raw_text.to_lowercase();
        let clean_q = q.replace("open ", "").replace("launch ", "").replace("app ", "").trim().to_string();
        
        if clean_q.len() < 2 {
            return false;
        }

        // Claim the fast-pass if the query matches an installed application name, id, or exec
        self.apps.iter().any(|app| {
            app.name.to_lowercase().contains(&clean_q) || 
            app.id.to_lowercase().contains(&clean_q) ||
            app.exec.to_lowercase().contains(&clean_q)
        })
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let q = query.raw_text.to_lowercase();
        let clean_q = q.replace("open ", "").replace("launch ", "").replace("app ", "").trim().to_string();
        
        let mut matches: Vec<_> = self.apps.iter()
            .filter(|app| {
                app.name.to_lowercase().contains(&clean_q) || 
                app.id.to_lowercase().contains(&clean_q) ||
                app.exec.to_lowercase().contains(&clean_q)
            })
            .collect();

        // Sort so exact/prefix matches bubble to the top of the fast-pass payload
        matches.sort_by(|a, b| {
            let a_exact = a.name.to_lowercase() == clean_q;
            let b_exact = b.name.to_lowercase() == clean_q;
            if a_exact && !b_exact { return std::cmp::Ordering::Less; }
            if !a_exact && b_exact { return std::cmp::Ordering::Greater; }

            let a_starts = a.name.to_lowercase().starts_with(&clean_q);
            let b_starts = b.name.to_lowercase().starts_with(&clean_q);
            match (a_starts, b_starts) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        // Uncapped mapping to allow all matches to flow to the UI results engine
        matches.into_iter().map(|app| {
            let mut metadata = HashMap::new();
            metadata.insert("exec".to_string(), app.exec.clone());
            if let Some(ico) = &app.icon {
                metadata.insert("icon".to_string(), ico.clone());
            }

            SearchResult {
                id: format!("app_{}", app.id),
                title: format!("Launch {}", app.name),
                snippet: app.exec.clone(),
                plugin_id: self.id().to_string(),
                score: 1.0,
                filename: None,
                filepath: None,
                metadata,
                created_at: None,
                indexed_at: None,
                full_context: None,
                ai_matched: None,
                ai_reasoning: None,
            }
        }).collect()
    }
}