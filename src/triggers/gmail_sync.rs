use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use native_tls::TlsConnector;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub struct GmailSyncDaemon {
    config_path: PathBuf,
    mail_dir: PathBuf,
    state_file: PathBuf,
    secret_file: PathBuf,
}

impl GmailSyncDaemon {
    pub fn new(config_dir: &str, data_dir: &str) -> Self {
        let mail_dir = Path::new(data_dir).join("mail");
        if !mail_dir.exists() {
            let _ = fs::create_dir_all(&mail_dir);
            #[cfg(unix)]
            {
                if let Ok(mut perms) = fs::metadata(&mail_dir).map(|m| m.permissions()) {
                    perms.set_mode(0o700);
                    let _ = fs::set_permissions(&mail_dir, perms);
                }
            }
        }

        Self {
            config_path: Path::new(config_dir).join("gmail.json"),
            mail_dir,
            state_file: Path::new(config_dir).join("gmail_state.json"),
            secret_file: Path::new(config_dir).join("gmail_secret.key"),
        }
    }

    pub fn start(&self) {
        let config_path = self.config_path.clone();
        let mail_dir = self.mail_dir.clone();
        let state_file = self.state_file.clone();
        let secret_file = self.secret_file.clone();

        thread::spawn(move || {
            let mut last_sync = std::time::Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(std::time::Instant::now);
            let mut last_config_mtime = SystemTime::UNIX_EPOCH;
            let mut last_state_mtime = SystemTime::UNIX_EPOCH;

            loop {
                let current_config_mtime = fs::metadata(&config_path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                let current_state_mtime = fs::metadata(&state_file).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);

                // Instantly break the 60-second interval if the UI modified the config or requested a forced resync
                let force_sync = current_config_mtime > last_config_mtime || current_state_mtime > last_state_mtime;

                if force_sync || last_sync.elapsed() >= Duration::from_secs(60) {
                    if let Ok(config_data) = fs::read_to_string(&config_path) {
                        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&config_data) {
                            if let Some(raw_email) = config["email"].as_str() {
                                let email = raw_email.trim();
                                if email == "your_email@gmail.com" || email.is_empty() {
                                    Self::write_state(&state_file, 0, false, 0, 0, "Awaiting user configuration in gmail.json.", false, vec![]);
                                } else {
                                    println!("[Gmail Sync] Reading config. Attempting Keyring lookup for email: '{}'", email);
                                    
                                    let mut password_opt = None;
                                    let entry_res = keyring::Entry::new("lens_for_gnome_gmail", email);
                                    
                                    if let Ok(entry) = entry_res {
                                        println!("[Gmail Sync] Keyring entry struct initialized. Querying Secret Service D-Bus...");
                                        match entry.get_password() {
                                            Ok(password) => {
                                                println!("[Gmail Sync] SUCCESS: Password retrieved from GNOME Keyring (length: {}).", password.len());
                                                password_opt = Some(password);
                                            }
                                            Err(e) => {
                                                eprintln!("[Gmail Sync ERROR] Failed to retrieve password for '{}'. Exact D-Bus/Keyring error: {:?}", email, e);
                                            }
                                        }
                                    } else {
                                        eprintln!("[Gmail Sync ERROR] Failed to initialize keyring API binding for '{}'.", email);
                                    }

                                    // Trigger Fallback Mechanics if Keyring dropped the D-Bus write
                                    if password_opt.is_none() {
                                        println!("[Gmail Sync] Attempting fallback to secure local key file...");
                                        if let Ok(password) = fs::read_to_string(&secret_file) {
                                            let cleaned = password.trim().to_string();
                                            if !cleaned.is_empty() {
                                                println!("[Gmail Sync] SUCCESS: Password retrieved from secure fallback file.");
                                                password_opt = Some(cleaned);
                                            }
                                        }
                                    }

                                    let mut has_backlog = false;
                                    match password_opt {
                                        Some(password) => {
                                            let clean_password: String = password.chars().filter(|c| !c.is_whitespace() && *c != '\r' && *c != '\n').collect();
                                            let history_years = config["history_years"].as_u64().unwrap_or(1).clamp(1, 5) as u32;
                                            has_backlog = Self::sync_inbox(email, &clean_password, &mail_dir, &state_file, history_years);
                                        }
                                        None => {
                                            eprintln!("[Gmail Sync ERROR] Failed to retrieve password for '{}' from both Keyring and fallback.", email);
                                            Self::write_state(&state_file, 0, false, 0, 0, "Email password not found in GNOME Keyring or secure fallback file.", true, vec![]);
                                        }
                                    }
                                    
                                    // Force immediate next tick by backdating the last_sync timer if there's a backlog
                                    if has_backlog {
                                        last_sync = std::time::Instant::now().checked_sub(Duration::from_secs(65)).unwrap_or_else(std::time::Instant::now);
                                    }
                                }
                            } else {
                                Self::write_state(&state_file, 0, false, 0, 0, "Missing email in configuration.", true, vec![]);
                            }
                        }
                    } else {
                        let dummy_config = serde_json::json!({
                            "email": "your_email@gmail.com",
                            "history_years": 1
                        });
                        if let Ok(json_str) = serde_json::to_string_pretty(&dummy_config) {
                            let _ = fs::write(&config_path, json_str);
                            #[cfg(unix)]
                            {
                                if let Ok(mut perms) = fs::metadata(&config_path).map(|m| m.permissions()) {
                                    perms.set_mode(0o600);
                                    let _ = fs::set_permissions(&config_path, perms);
                                }
                            }
                        }
                        Self::write_state(&state_file, 0, false, 0, 0, "Awaiting user configuration. App password must be added to keyring securely.", false, vec![]);
                    }

                    // Update trackers *after* we've done our own file modifications so we don't infinitely re-trigger
                    if last_sync.elapsed() < Duration::from_secs(60) {
                        last_sync = std::time::Instant::now();
                    }
                    last_config_mtime = fs::metadata(&config_path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    last_state_mtime = fs::metadata(&state_file).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                }
                
                // Tight, lightweight poll loop for maximum UI responsiveness
                thread::sleep(Duration::from_millis(1500));
            }
        });
    }

    fn write_state(state_file: &Path, last_uid: u32, is_syncing: bool, total: usize, current: usize, message: &str, is_error: bool, uncommitted: Vec<u32>) {
        let state = serde_json::json!({
            "last_uid": last_uid,
            "is_syncing": is_syncing,
            "total_emails": total,
            "synced_emails": current,
            "message": message,
            "is_error": is_error,
            "uncommitted_backlog": uncommitted
        });
        let _ = fs::write(state_file, serde_json::to_string(&state).unwrap());
    }

    fn sync_inbox(email: &str, password: &str, mail_dir: &Path, state_file: &Path, history_years: u32) -> bool {
        let mut last_uid = 0;
        let mut uncommitted_backlog = Vec::new();
        
        if let Ok(state_data) = fs::read_to_string(state_file) {
            if let Ok(state_json) = serde_json::from_str::<serde_json::Value>(&state_data) {
                last_uid = state_json["last_uid"].as_u64().unwrap_or(0) as u32;
                if let Some(arr) = state_json["uncommitted_backlog"].as_array() {
                    uncommitted_backlog = arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect();
                }
            }
        }

        println!("[Gmail Sync] Initiating IMAP connection sequence to imap.gmail.com:993");
        Self::write_state(state_file, last_uid, true, 0, 0, "Connecting to IMAP securely...", false, uncommitted_backlog.clone());

        let domain = "imap.gmail.com";
        let tls_res = TlsConnector::builder().build();
        if let Err(e) = tls_res {
            eprintln!("[Gmail Sync ERROR] TLS Connector Build Failed: {:?}", e);
            Self::write_state(state_file, last_uid, false, 0, 0, "TLS connection generation failed", true, uncommitted_backlog);
            return false;
        }
        let tls = tls_res.unwrap();
        
        let client = match imap::connect((domain, 993), domain, &tls) {
            Ok(c) => {
                println!("[Gmail Sync] Network stream connected. Negotiating TLS...");
                c
            },
            Err(e) => {
                eprintln!("[Gmail Sync ERROR] TCP/TLS Connection to {} failed: {}", domain, e);
                let err_msg = format!("Network Connection Error: {}", e);
                Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
                return false;
            }
        };

        println!("[Gmail Sync] Executing secure IMAP login for '{}'...", email);
        let mut session = match client.login(email, password) {
            Ok(s) => {
                println!("[Gmail Sync] SUCCESS: Google accepted credentials and authenticated IMAP session.");
                s
            },
            Err((e, _)) => {
                eprintln!("[Gmail Sync ERROR] Google REJECTED credentials for '{}'. Response: {}", email, e);
                let err_msg = format!("Login Rejected: {}", e);
                Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
                return false;
            }
        };

        println!("[Gmail Sync] Selecting 'INBOX' mailbox...");
        if let Err(e) = session.select("INBOX") {
            eprintln!("[Gmail Sync ERROR] Failed to select INBOX: {}", e);
            let err_msg = format!("Folder Selection Error: {}", e);
            Self::write_state(state_file, last_uid, false, 0, 0, &err_msg, true, uncommitted_backlog);
            return false;
        }

        let mut uids = if !uncommitted_backlog.is_empty() {
            println!("[Gmail Sync] Resuming from uncommitted backlog of {} emails.", uncommitted_backlog.len());
            uncommitted_backlog.clone()
        } else {
            let query = if last_uid == 0 {
                let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                let current_year = 1970 + (secs / 31556926); 
                let target_year = current_year - history_years as u64;
                println!("[Gmail Sync] Initial sync requested. Searching for emails since 01-Jan-{}.", target_year);
                format!("SINCE 01-Jan-{}", target_year)
            } else {
                println!("[Gmail Sync] Incremental sync requested. Searching for emails with UID > {}.", last_uid);
                format!("UID {}:*", last_uid + 1)
            };

            match session.uid_search(&query) {
                Ok(fetched) => {
                    let mut fetched_uids: Vec<u32> = fetched.into_iter().collect();
                    fetched_uids.sort_unstable();
                    fetched_uids.retain(|&u| u > last_uid);
                    println!("[Gmail Sync] Discovered {} new emails matching query.", fetched_uids.len());
                    fetched_uids
                },
                Err(e) => {
                    eprintln!("[Gmail Sync ERROR] IMAP SEARCH command failed: {}", e);
                    return false;
                }
            }
        };

        let total_unindexed_count = uids.len();
        if total_unindexed_count == 0 {
            println!("[Gmail Sync] Inbox is completely up-to-date.");
            Self::write_state(state_file, last_uid, false, 0, 0, "Inbox is fully synchronized.", false, vec![]);
            let _ = session.logout();
            return false;
        }

        let processing_backlog = total_unindexed_count > 500;
        if processing_backlog && uncommitted_backlog.is_empty() {
            println!("[Gmail Sync] Large backlog detected. Processing in chunks. Taking first 500 UIDs.");
            uids = uids.into_iter().take(500).collect();
        }

        let mut active_uncommitted = uids.clone();
        Self::write_state(state_file, last_uid, true, total_unindexed_count, 0, "Synchronizing messages...", false, active_uncommitted.clone());

        let mut downloaded_count = 0;
        let mut tracking_highest_uid = last_uid;

        for chunk in uids.chunks(50) {
            let fetch_query = chunk.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
            println!("[Gmail Sync] Fetching batch of {} emails...", chunk.len());
            
            match session.uid_fetch(&fetch_query, "RFC822") {
                Ok(messages) => {
                    let mut batch_downloaded = 0;
                    for msg in messages.iter() {
                        let uid = msg.uid.unwrap_or(0);
                        
                        if let Some(body) = msg.body() {
                            let file_path = mail_dir.join(format!("{}.eml", uid));

                            if let Err(e) = fs::write(&file_path, body) {
                                eprintln!("[Gmail Sync ERROR] Failed to write {} to disk: {}", file_path.display(), e);
                            } else {
                                #[cfg(unix)]
                                {
                                    if let Ok(mut perms) = fs::metadata(&file_path).map(|m| m.permissions()) {
                                        perms.set_mode(0o600);
                                        let _ = fs::set_permissions(&file_path, perms);
                                    }
                                }
                                downloaded_count += 1;
                                batch_downloaded += 1;
                            }
                        } else {
                            eprintln!("[Gmail Sync Warning] Message UID {} returned empty body from server.", uid);
                        }
                    }

                    // Authoritatively remove all requested block items to advance the loop indices cleanly
                    for &uid in chunk {
                        active_uncommitted.retain(|&x| x != uid);
                        if uid > tracking_highest_uid {
                            tracking_highest_uid = uid;
                        }
                    }
                    
                    println!(
                        "[Gmail Sync Progress] Successfully wrote {}/{} emails from this sub-chunk. (Total parsed: {} / {})",
                        batch_downloaded, chunk.len(), downloaded_count, total_unindexed_count
                    );
                }
                Err(e) => {
                    eprintln!("[Gmail Sync ERROR] Mime Batch Fetch Error: {}", e);
                    let err_msg = format!("Mime Batch Fetch Error: {}", e);
                    Self::write_state(state_file, last_uid, false, total_unindexed_count, downloaded_count, &err_msg, true, active_uncommitted);
                    return processing_backlog;
                }
            }

            Self::write_state(
                state_file,
                if active_uncommitted.is_empty() && !processing_backlog { tracking_highest_uid } else { last_uid },
                true,
                total_unindexed_count,
                downloaded_count,
                &format!("Syncing: pulled {} / {} entries", downloaded_count, total_unindexed_count),
                false,
                active_uncommitted.clone()
            );
        }

        let final_committed_uid = if active_uncommitted.is_empty() { tracking_highest_uid } else { last_uid };
        
        println!("[Gmail Sync] Sync batch complete. Downloaded: {}. Updating state to UID: {}", downloaded_count, final_committed_uid);
        Self::write_state(
            state_file,
            final_committed_uid,
            false,
            total_unindexed_count,
            downloaded_count,
            if processing_backlog { "Catching up backlog window..." } else { "Inbox is fully synchronized." },
            false,
            active_uncommitted
        );

        let _ = session.logout();
        println!("[Gmail Sync] Logged out successfully.");
        processing_backlog
    }
}