// src/ingestion/mod.rs

pub mod plaintext;
pub mod csv_file;
pub mod pdf;
pub mod office;
pub mod spreadsheet;
pub mod legacy;
pub mod image;
pub mod video; 
pub mod eml;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::{Read, Write};

use walkdir::WalkDir;
use rayon::prelude::*;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

use crate::vector::VectorStore;
use crate::engine::RuntimeAdapter;

const MAX_DOC_BYTES: usize = 256_000;

pub trait FileExtractor: Send + Sync {
    fn can_handle(&self, extension: &str) -> bool;
    fn extract(&self, path: &Path) -> Result<String, String>;
}

pub struct IndexerProgressState {
    pub is_running: Arc<AtomicBool>,
    pub current_target: Arc<Mutex<String>>,
    pub deep_processed: Arc<AtomicUsize>,
    pub shallow_processed: Arc<AtomicUsize>,
    pub total_files: Arc<AtomicUsize>,
    config_dir: String,
    write_lock: Arc<Mutex<()>>,
}

impl IndexerProgressState {
    pub fn new(config_dir: &str) -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            current_target: Arc::new(Mutex::new(String::new())),
            deep_processed: Arc::new(AtomicUsize::new(0)),
            shallow_processed: Arc::new(AtomicUsize::new(0)),
            total_files: Arc::new(AtomicUsize::new(0)),
            config_dir: config_dir.to_string(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn write_flush(&self) {
        let _lock = match self.write_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let file_path = Path::new(&self.config_dir).join("indexer_state.json");
        let temp_path = Path::new(&self.config_dir).join(format!("indexer_state_{}.tmp", std::process::id()));
        
        let current_target = match self.current_target.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };

        let payload = serde_json::json!({
            "is_running": self.is_running.load(Ordering::Relaxed),
            "current_target": current_target,
            "deep_processed": self.deep_processed.load(Ordering::Relaxed),
            "shallow_processed": self.shallow_processed.load(Ordering::Relaxed),
            "total_files": self.total_files.load(Ordering::Relaxed)
        });
        
        if std::fs::write(&temp_path, payload.to_string()).is_ok() {
            let _ = std::fs::rename(&temp_path, &file_path);
        }
    }
}

pub struct IngestionPipeline {
    store: Arc<VectorStore>,
    ai_model: Mutex<TextEmbedding>,
    extractors: Vec<Box<dyn FileExtractor>>,
    domain_keywords: HashMap<String, Vec<String>>,
    blacklist: Vec<String>,
    pub progress: IndexerProgressState,
    mail_dir: String,
    thread_pool: rayon::ThreadPool,
}

impl IngestionPipeline {
    pub fn new(store: Arc<VectorStore>, config_dir: &str, blacklist: Vec<String>, runtime_adapter: Arc<RuntimeAdapter>) -> Self {
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::ParaphraseMLMiniLML12V2;
        options.show_download_progress = true;
        options.cache_dir = runtime_adapter.data_dir().join("fastembed_cache");

        let ai_model = TextEmbedding::try_new(options)
            .expect("Failed to initialize Multi-lingual AI Model");

        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_or_create_config(&config_path);
        
        let progress = IndexerProgressState::new(config_dir);
        
        let mail_dir = runtime_adapter.data_dir().join("mail").to_string_lossy().to_string();

        // 1. Detect logical cores available to the application
        let available_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        
        // 2. Reserve at least 2 cores strictly for GNOME/OS to prevent desktop thrashing.
        // On a 4-core machine, it uses 2. On a 16-core machine, it uses 14. 
        let worker_threads = available_cores.saturating_sub(2).max(1);
        
        // 3. Build an isolated thread pool so we don't pollute the global Rayon state
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_threads)
            .thread_name(|i| format!("lens-worker-{}", i))
            .build()
            .expect("Failed to build isolated ingestion thread pool");

        Self {
            store,
            ai_model: Mutex::new(ai_model),
            extractors: vec![
                Box::new(plaintext::TxtExtractor),
                Box::new(csv_file::CsvExtractor),
                Box::new(pdf::PdfExtractor::new(MAX_DOC_BYTES, Arc::clone(&runtime_adapter))), 
                Box::new(office::ModernOfficeExtractor),
                Box::new(spreadsheet::SpreadsheetExtractor),
                Box::new(legacy::LegacyDocExtractor),
                Box::new(image::ImageExtractor::new(Arc::clone(&runtime_adapter))),
                Box::new(video::VideoExtractor::new()),
                Box::new(eml::EmlExtractor),
            ],
            domain_keywords,
            blacklist,
            progress,
            mail_dir,
            thread_pool,
        }
    }

    fn load_or_create_config(path: &str) -> HashMap<String, Vec<String>> {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(parsed) = serde_json::from_str(&content) {
                return parsed;
            }
        }
        
        let mut defaults = HashMap::new();
        defaults.insert("financial".to_string(), vec!["financial".to_string(), "invoice".to_string(), "receipt".to_string(), "tax".to_string(), "billing".to_string(), "factuur".to_string(), "rechnung".to_string(), "statement".to_string()]);
        defaults.insert("cad".to_string(), vec!["cad".to_string(), "dwg".to_string(), "blueprint".to_string(), "autocad".to_string(), "solidworks".to_string(), "schematic".to_string()]);
        defaults.insert("legal".to_string(), vec!["legal".to_string(), "contract".to_string(), "agreement".to_string(), "liability".to_string(), "lawsuit".to_string(), "court".to_string(), "compliance".to_string(), "terms".to_string()]);
        defaults.insert("software".to_string(), vec!["software".to_string(), "code".to_string(), "development".to_string(), "programming".to_string(), "api".to_string(), "json".to_string(), "rust".to_string(), "javascript".to_string(), "compile".to_string(), "deploy".to_string()]);
        defaults.insert("medical".to_string(), vec!["medical".to_string(), "patient".to_string(), "diagnosis".to_string(), "clinical".to_string(), "hospital".to_string(), "prescription".to_string(), "therapy".to_string()]);
        defaults.insert("hr".to_string(), vec!["hr".to_string(), "payroll".to_string(), "employee".to_string(), "employer".to_string(), "onboarding".to_string(), "resume".to_string(), "interview".to_string()]);

        if let Ok(json) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(path, json);
        }
        
        defaults
    }

    fn extract_entities(&self, content: &str, file_ext: &str) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        metadata.insert("filetype".to_string(), file_ext.to_string());

        let tokens: Vec<String> = content
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect();

        let mut domain_scores: HashMap<&String, usize> = HashMap::new();
        
        for (domain_tag, keywords) in &self.domain_keywords {
            let mut score = 0;
            for token in &tokens {
                if keywords.iter().any(|k| k == token) {
                    score += 1;
                }
            }
            domain_scores.insert(domain_tag, score);
        }

        if let Some((&best_domain, &best_score)) = domain_scores.iter().max_by_key(|&(_, score)| score) {
            if best_score > 0 {
                metadata.insert("domain".to_string(), best_domain.clone());
            } else {
                metadata.insert("domain".to_string(), "general".to_string());
            }
        } else {
            metadata.insert("domain".to_string(), "general".to_string());
        }

        metadata
    }

    fn smart_truncate(&self, content: &str, max_bytes: usize) -> String {
        if content.len() <= max_bytes {
            return content.to_string();
        }

        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        if paragraphs.is_empty() {
            let mut end = max_bytes;
            while !content.is_char_boundary(end) { end -= 1; }
            return content[..end].to_string();
        }

        let mut total_bytes = 0;
        let mut selected_indices = HashSet::new();

        let head_limit = max_bytes / 5;
        let mut head_bytes = 0;
        for (i, p) in paragraphs.iter().enumerate() {
            let p_len = p.len();
            if head_bytes + p_len > head_limit { break; }
            selected_indices.insert(i);
            head_bytes += p_len;
            total_bytes += p_len;
        }

        let tail_limit = max_bytes / 5;
        let mut tail_bytes = 0;
        for i in (0..paragraphs.len()).rev() {
            if selected_indices.contains(&i) { break; }
            let p_len = paragraphs[i].len();
            if tail_bytes + p_len > tail_limit { break; }
            selected_indices.insert(i);
            tail_bytes += p_len;
            total_bytes += p_len;
        }

        let mut middle_scored: Vec<(usize, usize, usize)> = Vec::new();
        for (i, p) in paragraphs.iter().enumerate() {
            if selected_indices.contains(&i) { continue; }
            
            let mut score = 0;
            let lower_p = p.to_lowercase();
            
            for word in p.split_whitespace() {
                if word.chars().any(|c| c.is_ascii_digit()) { score += 2; }
                if word.chars().next().map_or(false, |c| c.is_uppercase()) { score += 1; }
            }
            
            for (_, keywords) in &self.domain_keywords {
                for kw in keywords {
                    if lower_p.contains(kw) { score += 5; }
                }
            }
            
            let normalized_score = if !p.is_empty() { (score * 1000) / p.len() } else { 0 };
            middle_scored.push((i, p.len(), normalized_score));
        }

        middle_scored.sort_by(|a, b| b.2.cmp(&a.2));

        for (i, len, _) in middle_scored {
            if total_bytes + len > max_bytes { continue; }
            selected_indices.insert(i);
            total_bytes += len;
            if total_bytes >= max_bytes - 1000 { break; }
        }

        let mut final_indices: Vec<usize> = selected_indices.into_iter().collect();
        final_indices.sort_unstable();

        let mut result = String::with_capacity(max_bytes);
        for i in final_indices {
            result.push_str(paragraphs[i]);
            result.push_str("\n\n");
        }
        
        if result.len() > max_bytes {
            let mut end = max_bytes;
            while !result.is_char_boundary(end) { end -= 1; }
            result.truncate(end);
        }

        result
    }

    pub fn remove_file(&self, path: &Path) {
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = canonical_path.to_string_lossy().to_string();
        self.store.delete_document(&path_str);
        if let Some(name) = path.file_name() {
            println!("[Indexer] Removed from index: {:?}", name);
        }
    }

    fn prepare_document(&self, path: &Path) -> Option<(String, String, bool, u64, HashMap<String, String>)> {
        if !path.exists() { return None; }
        
        let is_dir = path.is_dir();
        if !is_dir && !path.is_file() { return None; }
        
        let ext = if is_dir {
            "directory".to_string()
        } else {
            path.extension().and_then(|e| e.to_str()).unwrap_or("unknown").to_lowercase()
        };

        if path.is_file() && ext == "eml" {
            let mut buffer = [0; 23];
            if let Ok(mut f) = std::fs::File::open(path) {
                if f.read_exact(&mut buffer).is_ok() && &buffer == b"[LENS_SECURE_TOMBSTONE]" {
                    return None;
                }
            }
        }

        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = canonical_path.to_string_lossy().to_string();

        let modified_at = path.metadata()
            .ok()
            .and_then(|m| m.modified().ok().or_else(|| m.created().ok()))
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as u64)
            .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64);

        if let Some(db_modified) = self.store.get_document_modified_at(&path_str) {
            if modified_at <= db_modified && db_modified != 0 {
                return None; 
            }
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;

        let (mut content, mut is_shallow) = if is_dir {
            ("Folder Directory".to_string(), true)
        } else if let Some(extractor) = self.extractors.iter().find(|e| e.can_handle(&ext)) {
            match extractor.extract(path) {
                Ok(extracted_text) => (self.smart_truncate(&extracted_text, MAX_DOC_BYTES), false),
                Err(_) => (String::new(), true),
            }
        } else {
            (String::new(), true)
        };

        if content.trim().is_empty() {
            content = format!("Filename: {}", path.file_name().unwrap_or_default().to_string_lossy());
            is_shallow = true;
        }

        let mut metadata = self.extract_entities(&content, &ext);
        
        if ext == "eml" {
            for line in content.lines().take(50) {
                let lower = line.to_lowercase();
                if lower.starts_with("subject: ") && !metadata.contains_key("subject") {
                    metadata.insert("subject".to_string(), line[9..].trim().to_string());
                } else if lower.starts_with("from: ") && !metadata.contains_key("from") {
                    let mut from_val = line[6..].trim().to_string();
                    if let Some(idx) = from_val.find('<') {
                        let name = from_val[..idx].trim();
                        if !name.is_empty() {
                            from_val = name.replace("\"", "");
                        }
                    }
                    metadata.insert("from".to_string(), from_val);
                } else if lower.starts_with("date: ") && !metadata.contains_key("date") {
                    metadata.insert("date".to_string(), line[6..].trim().to_string());
                } else if lower.starts_with("message-id: ") && !metadata.contains_key("message_id") {
                    metadata.insert("message_id".to_string(), line[12..].trim().to_string());
                }
            }
        }

        metadata.insert("created_at".to_string(), modified_at.to_string());
        metadata.insert("indexed_at".to_string(), now.to_string());
        metadata.insert("shallow_index".to_string(), is_shallow.to_string());

        Some((path_str, content, is_shallow, modified_at, metadata))
    }

    pub fn index_file(&self, path: &Path) {
        if let Some((path_str, content, is_shallow, modified_at, metadata)) = self.prepare_document(path) {
            let vector_option = if is_shallow {
                Some(vec![0.0; 384])
            } else {
                let safe_content = content.chars().take(8000).collect::<String>();
                let mut model = match self.ai_model.lock() {
                    Ok(m) => m,
                    Err(p) => p.into_inner(),
                };
                match model.embed(vec![safe_content], None) {
                    Ok(mut embeddings) => embeddings.pop(),
                    Err(_) => None,
                }
            };

            if let Some(vector) = vector_option {
                self.store.insert_document(
                    path_str.clone(),
                    content, 
                    vector,
                    modified_at,
                    metadata
                );

                if path_str.starts_with(&self.mail_dir) && path_str.ends_with(".eml") && !is_shallow {
                    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).truncate(true).open(&path_str) {
                        let _ = f.write_all(b"[LENS_SECURE_TOMBSTONE]");
                        if let Ok(new_meta) = std::fs::metadata(&path_str) {
                            if let Ok(sys_time) = new_meta.modified() {
                                if let Ok(dur) = sys_time.duration_since(UNIX_EPOCH) {
                                    self.store.update_document_timestamp(&path_str, dur.as_secs());
                                }
                            }
                        }
                    }
                }

                let filename = path.file_name().unwrap_or_default();
                if is_shallow {
                    self.progress.shallow_processed.fetch_add(1, Ordering::Relaxed);
                    println!("[Indexer] Shallow Tracked: {:?}", filename);
                } else {
                    self.progress.deep_processed.fetch_add(1, Ordering::Relaxed);
                    println!("[Indexer] Deep Processed: {:?}", filename);
                }
                
                self.progress.write_flush();
            }
        }
    }

    fn is_ignored_directory(&self, name: &str) -> bool {
        name.starts_with('.') || self.blacklist.contains(&name.to_string())
    }

    fn notify_user(title: &str, body: &str) {
        let icon_path = std::env::var("SNAP")
            .map(|snap| format!("{}/usr/share/pixmaps/lens-for-gnome.svg", snap))
            .unwrap_or_else(|_| {
                let local_path = std::env::current_dir()
                    .unwrap_or_default()
                    .join("metadata/io.github.cwittenberg.Lens.icon.svg");
                if local_path.exists() {
                    local_path.canonicalize().unwrap_or(local_path).to_string_lossy().to_string()
                } else {
                    "lens-for-gnome".to_string()
                }
            });

        let mut gdbus = std::process::Command::new("gdbus");
        gdbus.args(&[
            "call", "--session",
            "--dest", "org.freedesktop.Notifications",
            "--object-path", "/org/freedesktop/Notifications",
            "--method", "org.freedesktop.Notifications.Notify",
            "--",
            &format!("'{}'", "Lens for GNOME"),
            "uint32 0",
            &format!("'{}'", icon_path),
            &format!("'{}'", title.replace('\'', "")),
            &format!("'{}'", body.replace('\'', "")),
            "@as []",
            "@a{sv} {}",
            "int32 -1"
        ]);
        
        let _ = gdbus.spawn();
    }

    pub fn run_indexer(&self, target_dirs: Vec<String>, max_depth: usize) {
        self.progress.is_running.store(true, Ordering::Relaxed);
        self.progress.deep_processed.store(0, Ordering::Relaxed);
        self.progress.shallow_processed.store(0, Ordering::Relaxed);
        self.progress.total_files.store(0, Ordering::Relaxed);
        
        println!("[Indexer] Fetching existing index state for reconciliation...");
        let db_state = self.store.get_all_document_timestamps();

        let mut missing_or_modified = Vec::new();

        for target_dir in &target_dirs {
            {
                let mut tgt = match self.progress.current_target.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                *tgt = target_dir.clone();
            }
            self.progress.write_flush();
            
            println!("Scanning for missing or modified files in: {} (Max Depth: {})", target_dir, max_depth);
            
            for entry in WalkDir::new(target_dir)
                .max_depth(max_depth)
                .follow_links(true) 
                .into_iter()
                .filter_entry(|e| {
                    let fname = e.file_name().to_string_lossy();
                    !self.is_ignored_directory(&fname)
                }) 
            {
                match entry {
                    Ok(e) => {
                        if e.file_type().is_file() || e.file_type().is_dir() {
                            let canonical_path = e.path().canonicalize().unwrap_or_else(|_| e.path().to_path_buf());
                            let path_str = canonical_path.to_string_lossy().to_string();
                            
                            let disk_mod = e.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok().or_else(|| m.created().ok()))
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as u64)
                                .unwrap_or(0);

                            let needs_index = match db_state.get(&path_str) {
                                Some(&db_mod) => disk_mod > db_mod || db_mod == 0,
                                None => true,
                            };

                            if needs_index {
                                missing_or_modified.push(e);
                            }
                        }
                    },
                    Err(err) => {
                        let io_kind = err.io_error().map(|io| io.kind());
                        let is_permission_denied = io_kind == Some(std::io::ErrorKind::PermissionDenied);
                        let is_not_found = io_kind == Some(std::io::ErrorKind::NotFound);
                        
                        if !is_permission_denied && !is_not_found {
                            eprintln!("[Indexer] Directory traversal error in {}: {}", target_dir, err);
                        }
                    }
                }
            }
        }
        
        if missing_or_modified.is_empty() {
            println!("[Indexer] Reconciliation complete. No missing files found from offline period.");
        } else {
            let file_count = missing_or_modified.len();
            let dir_count = target_dirs.len();
            
            println!("[Indexer] Reconciliation found {} missing or modified files. Indexing now in sequential batches...", file_count);
            
            let should_notify = dir_count == 1 || file_count > 25;
            
            if should_notify {
                let start_msg = if dir_count == 1 {
                    format!("Started indexation for '{}' ({} items).", target_dirs[0], file_count)
                } else {
                    format!("Started bulk indexation across {} directories ({} items).", dir_count, file_count)
                };
                Self::notify_user("Lens Indexer", &start_msg);
            }
            
            self.progress.total_files.store(file_count, Ordering::Relaxed);
            self.progress.write_flush();
            
            // Dynamic Batch Sizing: Scales memory ingestion based on the safe thread count
            // Cap it between 32 and 128 to prevent memory spikes while still feeding the fastembed matrix effectively.
            let batch_size = (self.thread_pool.current_num_threads() * 8).clamp(32, 128);
            let total_batches = (file_count as f64 / batch_size as f64).ceil() as usize;

            for (batch_idx, chunk) in missing_or_modified.chunks(batch_size).enumerate() {
                if !self.progress.is_running.load(Ordering::Relaxed) {
                    println!("[Indexer] Sweep aborted by system.");
                    break;
                }

                println!("[Indexer] Processing batch {}/{} ({} files)...", batch_idx + 1, total_batches, chunk.len());

                // Execute the par_iter exclusively inside our isolated safe thread pool
                let prepared_docs: Vec<_> = self.thread_pool.install(|| {
                    chunk.par_iter().filter_map(|entry| {
                        let filename = entry.path().file_name().unwrap_or_default().to_string_lossy().to_string();
                        
                        let doc_opt = self.prepare_document(entry.path());
                        
                        if let Some(ref doc) = doc_opt {
                            if doc.2 {
                                self.progress.shallow_processed.fetch_add(1, Ordering::Relaxed);
                                println!("[Indexer] Tracked (Shallow): {}", filename);
                            } else {
                                println!("[Indexer] Extracted (Deep/AI Queued): {}", filename);
                            }
                        } else {
                            self.progress.shallow_processed.fetch_add(1, Ordering::Relaxed);
                            println!("[Indexer] Skipped (Unmodified/Already Indexed): {}", filename);
                        }
                        
                        doc_opt.map(|d| (d, filename))
                    }).collect()
                });

                self.progress.write_flush();

                if prepared_docs.is_empty() { 
                    continue; 
                }

                let mut texts_to_embed = Vec::new();
                for (doc, _) in &prepared_docs {
                    if !doc.2 {
                        texts_to_embed.push(doc.1.chars().take(8000).collect::<String>());
                    }
                }

                let mut embeddings_result = Vec::new();
                if !texts_to_embed.is_empty() {
                    println!("[Indexer] Generating AI embeddings for {} documents...", texts_to_embed.len());
                    let mut model = match self.ai_model.lock() {
                        Ok(m) => m,
                        Err(p) => p.into_inner(),
                    };

                    if let Ok(embs) = model.embed(texts_to_embed, None) {
                        embeddings_result = embs;
                    } else {
                        println!("[Indexer Warning] AI Embedding failed for batch, falling back to zero-vectors.");
                    }
                }

                let mut final_docs = Vec::new();
                let mut embed_idx = 0;
                for (doc, filename) in prepared_docs {
                    let vector = if doc.2 {
                        vec![0.0; 384]
                    } else {
                        if embed_idx < embeddings_result.len() {
                            let v = embeddings_result[embed_idx].clone();
                            embed_idx += 1;
                            self.progress.deep_processed.fetch_add(1, Ordering::Relaxed);
                            println!("[Indexer] Embedded (Deep): {}", filename);
                            v
                        } else {
                            self.progress.deep_processed.fetch_add(1, Ordering::Relaxed);
                            println!("[Indexer] Processed (Deep/Fallback): {}", filename);
                            vec![0.0; 384]
                        }
                    };

                    final_docs.push((doc.0, doc.1, vector, doc.3, doc.4));
                }

                let eml_paths: Vec<String> = final_docs.iter()
                    .filter(|(path, _, vector, _, _)| {
                        let is_shallow = vector.iter().all(|&v| v == 0.0);
                        path.starts_with(&self.mail_dir) && path.ends_with(".eml") && !is_shallow
                    })
                    .map(|(path, _, _, _, _)| path.clone())
                    .collect();

                self.store.insert_documents(final_docs);

                for path_str in eml_paths {
                    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).truncate(true).open(&path_str) {
                        let _ = f.write_all(b"[LENS_SECURE_TOMBSTONE]");
                        if let Ok(new_meta) = std::fs::metadata(&path_str) {
                            if let Ok(sys_time) = new_meta.modified() {
                                if let Ok(dur) = sys_time.duration_since(UNIX_EPOCH) {
                                    self.store.update_document_timestamp(&path_str, dur.as_secs());
                                }
                            }
                        }
                    }
                }

                self.progress.write_flush();
            }
            
            if should_notify {
                let end_msg = if dir_count == 1 {
                    format!("Finished indexing '{}'.", target_dirs[0])
                } else {
                    format!("Finished bulk indexation of {} items.", file_count)
                };
                Self::notify_user("Lens Indexer", &end_msg);
            }
        }
        
        self.progress.is_running.store(false, Ordering::Relaxed);
        {
            let mut tgt = match self.progress.current_target.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *tgt = String::new();
        }
        self.progress.write_flush();
        
        println!("Full Ingestion sweep complete.");
    }
}