// src/ingestion/mod.rs
pub mod plaintext;
pub mod csv_file;
pub mod pdf;
pub mod office;
pub mod spreadsheet;
pub mod legacy;
pub mod image;
pub mod video; 

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;
use rayon::prelude::*;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use crate::vector::VectorStore;

// Configurable global limit for text extraction per document
const MAX_DOC_BYTES: usize = 256_000;

pub trait FileExtractor: Send + Sync {
    fn can_handle(&self, extension: &str) -> bool;
    fn extract(&self, path: &Path) -> Result<String, String>;
}

pub struct IngestionPipeline {
    store: Arc<VectorStore>,
    ai_model: Mutex<TextEmbedding>,
    extractors: Vec<Box<dyn FileExtractor>>,
    domain_keywords: HashMap<String, Vec<String>>,
    blacklist: Vec<String>,
}

impl IngestionPipeline {
    pub fn new(store: Arc<VectorStore>, config_dir: &str, blacklist: Vec<String>) -> Self {
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::ParaphraseMLMiniLML12V2;
        options.show_download_progress = true;

        let ai_model = TextEmbedding::try_new(options)
            .expect("Failed to initialize Multi-lingual AI Model");

        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_or_create_config(&config_path);

        Self {
            store,
            ai_model: Mutex::new(ai_model),
            extractors: vec![
                Box::new(plaintext::TxtExtractor),
                Box::new(csv_file::CsvExtractor),
                Box::new(pdf::PdfExtractor::new(MAX_DOC_BYTES)), 
                Box::new(office::ModernOfficeExtractor),
                Box::new(spreadsheet::SpreadsheetExtractor),
                Box::new(legacy::LegacyDocExtractor),
                Box::new(image::ImageExtractor::new()),
                Box::new(video::VideoExtractor::new()),
            ],
            domain_keywords,
            blacklist,
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
        let path_str = path.to_string_lossy().to_string();
        self.store.delete_document(&path_str);
        if let Some(name) = path.file_name() {
            println!("[Indexer] Removed from index: {:?}", name);
        }
    }

    pub fn index_file(&self, path: &Path) {
        if !path.exists() { return; }
        let is_dir = path.is_dir();
        if !is_dir && !path.is_file() { return; }
        
        let path_str = path.to_string_lossy().to_string();

        // Enforce nanosecond precision. SQLite uses i64 which safely holds nanosecond timestamps until the year 2262.
        // This permanently fixes the collision bug where files saved by GTK/GNOME within the same millisecond were dropped.
        let modified_at = path.metadata()
            .ok()
            .and_then(|m| m.modified().ok().or_else(|| m.created().ok()))
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64);

        if let Some(db_modified) = self.store.get_document_modified_at(&path_str) {
            if modified_at <= db_modified {
                return; // Skip reindexing, file/folder is unchanged
            }
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let ext = if is_dir {
            "directory".to_string()
        } else {
            path.extension().and_then(|e| e.to_str()).unwrap_or("unknown").to_lowercase()
        };

        // Inject the target keywords directly into the FTS engine so it naturally matches lexical "folder" queries
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

        // CRITICAL FIX: The embedding model fails silently and aborts the function returning `None` if fed an empty string. 
        // This forces empty screenshots to embed gracefully as their title.
        if content.trim().is_empty() {
            content = format!("Filename: {}", path.file_name().unwrap_or_default().to_string_lossy());
            is_shallow = true;
        }

        let mut metadata = self.extract_entities(&content, &ext);
        metadata.insert("created_at".to_string(), modified_at.to_string());
        metadata.insert("indexed_at".to_string(), now.to_string());
        metadata.insert("shallow_index".to_string(), is_shallow.to_string());

        let vector_option = if is_shallow {
            Some(vec![0.0; 384])
        } else {
            let safe_content = content.chars().take(8000).collect::<String>();
            let mut model = self.ai_model.lock().unwrap();
            match model.embed(vec![safe_content], None) {
                Ok(mut embeddings) => embeddings.pop(),
                Err(_) => None,
            }
        };

        if let Some(vector) = vector_option {
            self.store.insert_document(
                path_str,
                content, 
                vector,
                modified_at,
                metadata
            );
            if is_shallow {
                println!("[Indexer] Shallow Tracked: {:?}", path.file_name().unwrap_or_default());
            } else {
                println!("[Indexer] Deep Processed: {:?}", path.file_name().unwrap_or_default());
            }
        }
    }

    fn is_ignored_directory(&self, name: &str) -> bool {
        name.starts_with('.') || self.blacklist.contains(&name.to_string())
    }

    pub fn run_indexer(&self, target_dirs: Vec<String>, max_depth: usize) {
        println!("[Indexer] Fetching existing index state for reconciliation...");
        let db_state = self.store.get_all_document_timestamps();
        let mut missing_or_modified = Vec::new();

        for target_dir in target_dirs {
            println!("Scanning for missing or modified files in: {} (Max Depth: {})", target_dir, max_depth);
            
            for entry in WalkDir::new(&target_dir)
                .max_depth(max_depth)
                .follow_links(true) // Required for explicit mount points or custom user symlinks
                .into_iter()
                .filter_entry(|e| {
                    let fname = e.file_name().to_string_lossy();
                    !self.is_ignored_directory(&fname)
                }) 
            {
                match entry {
                    Ok(e) => {
                        if e.file_type().is_file() || e.file_type().is_dir() {
                            let path_str = e.path().to_string_lossy().to_string();
                            
                            let disk_mod = e.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok().or_else(|| m.created().ok()))
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0);

                            let needs_index = match db_state.get(&path_str) {
                                Some(&db_mod) => disk_mod > db_mod || db_mod == 0,
                                None => true, // Missing from index!
                            };

                            if needs_index {
                                missing_or_modified.push(e);
                            }
                        }
                    },
                    Err(err) => {
                        let is_permission_denied = err.io_error().map_or(false, |io| io.kind() == std::io::ErrorKind::PermissionDenied);
                        if !is_permission_denied {
                            eprintln!("[Indexer] Directory traversal error in {}: {}", target_dir, err);
                        }
                    }
                }
            }
        }
        
        if missing_or_modified.is_empty() {
            println!("[Indexer] Reconciliation complete. No missing files found from offline period.");
        } else {
            println!("[Indexer] Reconciliation found {} missing or modified files. Indexing now...", missing_or_modified.len());
            missing_or_modified.par_iter().for_each(|entry| {
                self.index_file(entry.path());
            });
        }
        
        println!("Full Ingestion sweep complete.");
    }
}