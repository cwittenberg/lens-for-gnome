use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use std::path::Path;

/// CoR: Sequentially resolves secure DB keys.
pub struct SecurityManager;

impl SecurityManager {
    pub fn resolve_or_generate_key(db_path: &str) -> Option<String> {
        let db_key_path = Path::new(db_path).with_extension("key");
        let mut saved_key = String::new();

        // 1. Prioritize stable persistent key file if the GNOME Keyring is volatile or inaccessible
        if db_key_path.exists() {
            if let Ok(key) = std::fs::read_to_string(&db_key_path) {
                let cleaned = key.trim().to_string();
                if !cleaned.is_empty() {
                    saved_key = cleaned;
                }
            }
        }

        // 2. Fallback to querying the GNOME DBus Keyring
        if saved_key.is_empty() {
            for attempt in 1..=10 {
                if let Ok(entry) = keyring::Entry::new("lens_for_gnome_db", "sqlcipher_key") {
                    match entry.get_password() {
                        Ok(k) => {
                            saved_key = k;
                            break;
                        }
                        Err(keyring::Error::NoEntry) => {
                            break; // Key definitively does not exist
                        }
                        Err(e) => {
                            eprintln!("[Security] Transient Keyring error or DBus locked: {:?}. Retrying... (Attempt {}/10)", e, attempt);
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                } else {
                    eprintln!("[Security] Waiting for GNOME Keyring DBus... (Attempt {}/10)", attempt);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }

        // 3. Generate a new key if none exists, and persist it locally to survive volatile keyrings
        if saved_key.is_empty() {
            saved_key = thread_rng()
                .sample_iter(&Alphanumeric)
                .take(64)
                .map(char::from)
                .collect();
                
            // Best effort save to DBus
            if let Ok(entry) = keyring::Entry::new("lens_for_gnome_db", "sqlcipher_key") {
                let _ = entry.set_password(&saved_key);
            }
            
            // Force save to local file with strict 600 permissions as a persistent fallback
            let _ = std::fs::write(&db_key_path, saved_key.as_bytes());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(mut perms) = std::fs::metadata(&db_key_path).map(|m| m.permissions()) {
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&db_key_path, perms);
                }
            }
        }

        if !saved_key.is_empty() {
            Some(saved_key)
        } else {
            None
        }
    }
}