// src/vector/mod.rs
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use crate::domain::SearchResult;
use std::path::Path;

#[derive(Clone)]
pub struct CachedDoc {
    pub id: String,
    pub modified_at: u64,
    pub metadata: HashMap<String, String>,
    pub embedding: Vec<f32>,
}

pub struct VectorStore {
    conn: Mutex<Connection>,
    db_path: String,
    cache: Arc<RwLock<HashMap<String, CachedDoc>>>,
}

impl VectorStore {
    pub fn new(db_path: &str) -> Self {
        let conn = Connection::open(db_path).expect("Failed to open SQLite database");
        
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA mmap_size = 30000000000; 
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -2000000;
            "
        ).expect("Failed to configure in-memory pragmas");

        conn.execute("DROP TABLE IF EXISTS document_contents", []).ok();

        let mut needs_migration = false;
        if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(documents)") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    let name: String = row.get(1).unwrap_or_default();
                    if name == "content_zip" {
                        needs_migration = true;
                        break;
                    }
                }
            }
        }

        if needs_migration {
            println!("[Database] Migrating existing database schema to split tables...");
            conn.execute_batch(
                "
                BEGIN TRANSACTION;
                
                CREATE TABLE documents_new (
                    id TEXT PRIMARY KEY,
                    modified_at INTEGER NOT NULL,
                    metadata JSON NOT NULL,
                    embedding BLOB NOT NULL
                );
                
                INSERT INTO documents_new (id, modified_at, metadata, embedding)
                SELECT id, modified_at, metadata, embedding FROM documents;

                DROP TABLE documents;
                ALTER TABLE documents_new RENAME TO documents;
                
                COMMIT;
                "
            ).expect("Failed to run schema migration");
            
            conn.execute("VACUUM", []).ok();
        }

        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                modified_at INTEGER NOT NULL,
                metadata JSON NOT NULL,
                embedding BLOB NOT NULL
            )",
            [],
        ).expect("Failed to create base tables");

        println!("[Database] Proactively building performance indexes for JSON metadata and paths...");
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_documents_lower_id ON documents(LOWER(id));
            CREATE INDEX IF NOT EXISTS idx_docs_metadata_filetype ON documents(LOWER(json_extract(metadata, '$.filetype')));
            CREATE INDEX IF NOT EXISTS idx_docs_metadata_domain ON documents(LOWER(json_extract(metadata, '$.domain')));
            "
        ).expect("Failed to create performance expression indexes");

        let mut needs_fts_migration = false;
        let mut fts_exists = false;

        if let Ok(mut stmt) = conn.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='documents_fts'") {
            if let Ok(mut rows) = stmt.query([]) {
                if let Ok(Some(row)) = rows.next() {
                    fts_exists = true;
                    let sql: String = row.get(0).unwrap_or_default();
                    if sql.contains("trigram") {
                        needs_fts_migration = true;
                    }
                }
            }
        }

        if needs_fts_migration {
            println!("[Database] Upgrading FTS5 tokenizer to 'unicode61' for exact keyword/number matching...");
            conn.execute("DROP TABLE IF EXISTS documents_fts", []).ok();
            
            conn.execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
                    id UNINDEXED,
                    filename,
                    content_text,
                    tokenize='unicode61 remove_diacritics 2'
                )",
                [],
            ).expect("Failed to create FTS5 virtual table");
        } else if !fts_exists {
            conn.execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
                    id UNINDEXED,
                    filename,
                    content_text,
                    tokenize='unicode61 remove_diacritics 2'
                )",
                [],
            ).expect("Failed to create FTS5 virtual table");
        }
        
        if needs_migration || needs_fts_migration {
            println!("[Database] Resetting modification timestamps to rebuild dropped text indices...");
            conn.execute("UPDATE documents SET modified_at = 0", []).ok();
        }

        println!("[Database] Normalizing existing timestamps to stable UNIX seconds...");
        conn.execute("UPDATE documents SET modified_at = modified_at / 1000000000 WHERE modified_at > 1000000000000000", []).ok();
        conn.execute("UPDATE documents SET modified_at = modified_at / 1000 WHERE modified_at > 1000000000000", []).ok();

        println!("[Database] Running startup WAL checkpoint to ensure continuous disk read performance...");
        conn.execute("PRAGMA wal_checkpoint(PASSIVE)", []).ok();

        println!("[Database] Loading embeddings and metadata into RAM for instant search...");
        let mut cache = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, modified_at, metadata, embedding FROM documents") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    let id: String = row.get(0).unwrap_or_default();
                    let modified_at: u64 = row.get(1).unwrap_or(0);
                    let metadata_json: String = row.get(2).unwrap_or_else(|_| "{}".to_string());
                    let metadata: HashMap<String, String> = serde_json::from_str(&metadata_json).unwrap_or_default();

                    let mut embedding = Vec::new();
                    if let Ok(blob_ref) = row.get_ref(3) {
                        if let Ok(bytes) = blob_ref.as_blob() {
                            embedding.reserve_exact(bytes.len() / 4);
                            for chunk in bytes.chunks_exact(4) {
                                embedding.push(f32::from_ne_bytes(chunk.try_into().unwrap_or([0; 4])));
                            }
                        }
                    }
                    cache.insert(id.clone(), CachedDoc { id, modified_at, metadata, embedding });
                }
            }
        }
        println!("[Database] Successfully cached {} documents in RAM.", cache.len());

        Self { 
            conn: Mutex::new(conn),
            db_path: db_path.to_string(),
            cache: Arc::new(RwLock::new(cache)),
        }
    }

    pub fn get_db_stats(&self) -> (usize, u64) {
        let count = self.cache.read().unwrap().len();
        
        let mut total_size = 0;
        if let Ok(meta) = std::fs::metadata(&self.db_path) {
            total_size += meta.len();
        }
        if let Ok(meta) = std::fs::metadata(format!("{}-wal", self.db_path)) {
            total_size += meta.len();
        }
        if let Ok(meta) = std::fs::metadata(format!("{}-shm", self.db_path)) {
            total_size += meta.len();
        }

        (count, total_size)
    }

    pub fn force_reindex_all(&self) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        conn.execute("UPDATE documents SET modified_at = 0", []).ok();
        
        let mut cache_guard = self.cache.write().unwrap();
        for doc in cache_guard.values_mut() {
            doc.modified_at = 0;
        }
    }

    pub fn prune_orphans(&self) {
        let mut ids = Vec::new();
        {
            let cache_guard = self.cache.read().unwrap();
            for id in cache_guard.keys() {
                ids.push(id.clone());
            }
        }

        let mut orphans = Vec::new();
        for id in ids {
            if !Path::new(&id).exists() {
                orphans.push(id);
            }
        }

        if !orphans.is_empty() {
            let conn = match self.conn.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            conn.execute("BEGIN TRANSACTION", []).ok();
            for orphan in &orphans {
                conn.execute("DELETE FROM documents WHERE id = ?1", params![orphan]).ok();
                conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![orphan]).ok();
            }
            conn.execute("COMMIT", []).ok();
            
            conn.execute("VACUUM", []).ok();
            
            let mut cache_guard = self.cache.write().unwrap();
            for orphan in &orphans {
                cache_guard.remove(orphan);
            }
        }
    }

    pub fn get_available_metadata_keys(&self) -> Vec<String> {
        let cache_guard = self.cache.read().unwrap();
        let mut keys = std::collections::HashSet::new();
        
        for doc in cache_guard.values() {
            for key in doc.metadata.keys() {
                if key != "shallow_index" && key != "created_at" && key != "indexed_at" {
                    keys.insert(key.clone());
                }
            }
        }
        
        keys.into_iter().collect()
    }

    pub fn get_all_document_timestamps(&self) -> HashMap<String, u64> {
        let cache_guard = self.cache.read().unwrap();
        cache_guard.values().map(|doc| (doc.id.clone(), doc.modified_at)).collect()
    }

    pub fn get_document_modified_at(&self, path: &str) -> Option<u64> {
        let cache_guard = self.cache.read().unwrap();
        cache_guard.get(path).map(|doc| doc.modified_at)
    }

    pub fn delete_document(&self, path: &str) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        conn.execute("BEGIN TRANSACTION", []).ok();
        conn.execute("DELETE FROM documents WHERE id = ?1", params![path]).ok();
        conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![path]).ok();
        conn.execute("COMMIT", []).ok();

        let mut cache_guard = self.cache.write().unwrap();
        cache_guard.remove(path);
    }

    fn f32_to_bytes(vec: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(vec.len() * 4);
        for v in vec {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        bytes
    }

    pub fn insert_document(&self, path: String, content: String, embedding: Vec<f32>, modified_at: u64, metadata: HashMap<String, String>) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
        let embedding_blob = Self::f32_to_bytes(&embedding);
        
        let filename = Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();

        conn.execute("BEGIN TRANSACTION", []).ok();

        if let Err(e) = conn.execute(
            "INSERT INTO documents (id, modified_at, metadata, embedding) 
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET 
                 modified_at=excluded.modified_at,
                 metadata=excluded.metadata,
                 embedding=excluded.embedding",
            params![&path, modified_at, metadata_json, embedding_blob],
        ) {
            eprintln!("[Database] Failed to insert document metadata for {}: {}", path, e);
            conn.execute("ROLLBACK", []).ok();
            return;
        }

        conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![&path]).ok();
        
        if let Err(e) = conn.execute(
            "INSERT INTO documents_fts (id, filename, content_text) VALUES (?1, ?2, ?3)",
            params![&path, filename, content],
        ) {
            eprintln!("[Database] Failed to update FTS5 index for {}: {}", path, e);
            conn.execute("ROLLBACK", []).ok();
            return;
        }

        conn.execute("COMMIT", []).ok();

        let mut cache_guard = self.cache.write().unwrap();
        cache_guard.insert(path.clone(), CachedDoc {
            id: path,
            modified_at,
            metadata,
            embedding,
        });
    }

    pub fn insert_documents(&self, documents: Vec<(String, String, Vec<f32>, u64, HashMap<String, String>)>) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        conn.execute("BEGIN TRANSACTION", []).ok();

        for (path, content, embedding, modified_at, metadata) in &documents {
            let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
            let embedding_blob = Self::f32_to_bytes(embedding);
            
            let filename = Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

            if let Err(e) = conn.execute(
                "INSERT INTO documents (id, modified_at, metadata, embedding) 
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET 
                     modified_at=excluded.modified_at,
                     metadata=excluded.metadata,
                     embedding=excluded.embedding",
                params![path, *modified_at, metadata_json, embedding_blob],
            ) {
                eprintln!("[Database] Failed to insert document metadata for {}: {}", path, e);
                continue;
            }

            conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![path]).ok();
            
            if let Err(e) = conn.execute(
                "INSERT INTO documents_fts (id, filename, content_text) VALUES (?1, ?2, ?3)",
                params![path, filename, content],
            ) {
                eprintln!("[Database] Failed to update FTS5 index for {}: {}", path, e);
            }
        }

        conn.execute("COMMIT", []).ok();

        let mut cache_guard = self.cache.write().unwrap();
        for (path, _content, embedding, modified_at, metadata) in documents {
            cache_guard.insert(path.clone(), CachedDoc {
                id: path,
                modified_at,
                metadata,
                embedding,
            });
        }
    }

    pub fn search(
        &self, 
        target_embedding: &[f32], 
        raw_query_text: &str, 
        min_ts: Option<u64>, 
        max_ts: Option<u64>, 
        filters: &HashMap<String, String>, 
        directory_filter: Option<&String>, 
        plugin_id: &str,
        prioritize_folders: bool
    ) -> Vec<SearchResult> {
        let mut fts_matches = HashMap::new();
        
        let stop_words = [
            "what", "how", "why", "who", "when", "where", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to", "on", "at", "by", "about",
            "show", "me", "find", "search", "get", "looking", "documents", "document", "files", "file", "pictures", "picture", "photos", "photo", "saying", "something", "mentioning", "mentions", "containing", "like", "anything",
            "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar", "op", "bij", "over", "documenten", "bestanden", "foto", "fotos",
            "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en", "documentos", "archivos", "fotos"
        ];

        let mut exact_phrases = Vec::new();
        let mut in_quotes = false;
        let mut current_phrase = String::new();

        for c in raw_query_text.chars() {
            if c == '"' {
                if in_quotes && !current_phrase.trim().is_empty() {
                    exact_phrases.push(current_phrase.clone());
                    current_phrase.clear();
                }
                in_quotes = !in_quotes;
            } else if in_quotes {
                current_phrase.push(c);
            }
        }
        
        let clean_query_text: String = raw_query_text.to_lowercase().chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
            .collect();

        let semantic_query = clean_query_text
            .split_whitespace()
            .filter(|w| w.len() > 2 && !stop_words.contains(w) && !exact_phrases.contains(&w.to_string()))
            .map(|w| {
                if w.len() >= 4 {
                    format!("\"{}\"*", w)
                } else {
                    format!("\"{}\"", w)
                }
            }) 
            .collect::<Vec<_>>()
            .join(" OR ");

        let mut safe_query = semantic_query;

        if !exact_phrases.is_empty() {
            let exact_fts: String = exact_phrases.iter()
                .map(|p| format!("\"{}\"", p.replace("\"", ""))) 
                .collect::<Vec<_>>()
                .join(" AND ");
                
            if safe_query.is_empty() {
                safe_query = exact_fts;
            } else {
                safe_query = format!("({}) AND {}", safe_query, exact_fts);
            }
        }
            
        let has_fts_query = !safe_query.is_empty();
        let clean_q_for_like = raw_query_text.to_lowercase().trim().to_string();

        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut sql_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut sql_base = String::new();

        if has_fts_query {
            sql_base.push_str("SELECT fts.id, snippet(documents_fts, 2, '<b>', '</b>', '...', 30) as snip, docs.modified_at as rank_val 
                               FROM documents_fts fts
                               JOIN documents docs ON fts.id = docs.id
                               WHERE documents_fts MATCH ?");
            sql_params.push(Box::new(safe_query.clone()));

            if let Some(dir) = directory_filter {
                sql_base.push_str(" AND LOWER(fts.id) LIKE LOWER(?)");
                sql_params.push(Box::new(format!("{}%", dir)));
            }
            if let Some(min) = min_ts {
                sql_base.push_str(" AND docs.modified_at >= ?");
                sql_params.push(Box::new(min as i64));
            }
            if let Some(max) = max_ts {
                sql_base.push_str(" AND docs.modified_at <= ?");
                sql_params.push(Box::new(max as i64));
            }
            for (key, val) in filters {
                sql_base.push_str(" AND LOWER(json_extract(docs.metadata, ?)) = LOWER(?)");
                sql_params.push(Box::new(format!("$.{}", key)));
                sql_params.push(Box::new(val.clone()));
            }

            sql_base.push_str("\nUNION\n");
        }

        sql_base.push_str("SELECT docs.id, '' as snip, docs.modified_at as rank_val 
                           FROM documents docs
                           WHERE 1=1");

        if let Some(dir) = directory_filter {
            if !clean_q_for_like.is_empty() {
                sql_base.push_str(" AND (LOWER(docs.id) LIKE LOWER(?) OR LOWER(docs.id) LIKE LOWER(?))");
                sql_params.push(Box::new(format!("{}%{}", dir, clean_q_for_like))); 
                sql_params.push(Box::new(format!("{}%{}%", dir, clean_q_for_like))); 
            } else {
                sql_base.push_str(" AND LOWER(docs.id) LIKE LOWER(?)");
                sql_params.push(Box::new(format!("{}%", dir)));
            }
        } else if !clean_q_for_like.is_empty() {
            sql_base.push_str(" AND LOWER(docs.id) LIKE LOWER(?)");
            sql_params.push(Box::new(format!("%{}%", clean_q_for_like)));
        }

        if let Some(min) = min_ts {
            sql_base.push_str(" AND docs.modified_at >= ?");
            sql_params.push(Box::new(min as i64));
        }
        if let Some(max) = max_ts {
            sql_base.push_str(" AND docs.modified_at <= ?");
            sql_params.push(Box::new(max as i64));
        }
        for (key, val) in filters {
            sql_base.push_str(" AND LOWER(json_extract(docs.metadata, ?)) = LOWER(?)");
            sql_params.push(Box::new(format!("$.{}", key)));
            sql_params.push(Box::new(val.clone()));
        }

        sql_base.push_str(" ORDER BY rank_val DESC");

        if let Ok(mut fts_stmt) = conn.prepare(&sql_base) {
            let ref_params: Vec<&dyn rusqlite::ToSql> = sql_params.iter().map(|b| b.as_ref()).collect();
            if let Ok(mut rows) = fts_stmt.query(&ref_params[..]) {
                let mut rank = 1;
                while let Ok(Some(row)) = rows.next() {
                    let id: String = row.get(0).unwrap_or_default();
                    let snippet: String = row.get(1).unwrap_or_default();
                    fts_matches.insert(id, (rank, snippet));
                    rank += 1;
                }
            }
        } else {
            eprintln!("[Database] Failed to prepare SQL push-down query:\n{}", sql_base);
        }
        drop(conn);
        println!("[Database] Exhaustive SQL push-down index filter yielded {} row targets.", fts_matches.len());

        let is_dummy_vector = target_embedding.is_empty() || target_embedding.iter().all(|&v| v == 0.0);
        let norm_a: f32 = if !is_dummy_vector {
            target_embedding.iter().map(|v| v * v).sum::<f32>().sqrt().max(0.0001)
        } else {
            1.0
        };

        let cache_guard = self.cache.read().unwrap();

        let mut candidate_scores: Vec<(String, f32, Option<String>, Option<String>)> = cache_guard.iter()
            .filter(|(_, doc)| {
                // If semantic search is running, do not drop files unless we have too many candidates
                if is_dummy_vector && !fts_matches.contains_key(&doc.id) {
                    return false;
                }
                
                if let Some(min) = min_ts { if doc.modified_at < min { return false; } }
                if let Some(max) = max_ts { if doc.modified_at > max { return false; } }
                if let Some(dir) = directory_filter {
                    if !doc.id.to_lowercase().starts_with(&dir.to_lowercase()) { return false; }
                }
                for (key, val) in filters {
                    if let Some(doc_val) = doc.metadata.get(key) {
                        if doc_val.to_lowercase() != val.to_lowercase() {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            })
            .map(|(_, doc)| {
                let v_score = if is_dummy_vector {
                    let rank = fts_matches.get(&doc.id).unwrap().0;
                    1.0 / (rank as f32)
                } else {
                    let mut dot_product = 0.0;
                    let mut norm_b = 0.0;
                    for (i, &val_b) in doc.embedding.iter().enumerate() {
                        if i >= target_embedding.len() { break; }
                        let val_a = target_embedding[i];
                        dot_product += val_a * val_b;
                        norm_b += val_b * val_b;
                    }
                    if norm_b == 0.0 { 0.0 } else { dot_product / (norm_a * norm_b.sqrt()) }
                };
                
                let is_shallow = doc.metadata.get("shallow_index").cloned();
                let filetype = doc.metadata.get("filetype").cloned();
                
                (doc.id.clone(), v_score, is_shallow, filetype)
            })
            .collect();

        println!("[Database] Retained {} candidates after cache threshold and metadata bounds.", candidate_scores.len());

        candidate_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        let mut vector_ranks = HashMap::new();
        for (i, (id, _, _, _)) in candidate_scores.iter().enumerate() {
            vector_ranks.insert(id.clone(), i + 1);
        }

        let contains_specifics = !exact_phrases.is_empty() || raw_query_text.split_whitespace().any(|w| w.chars().any(|c| c.is_ascii_digit()));
        let rrf_k = 60.0;
        
        let mut scored_candidates: Vec<_> = candidate_scores.into_iter().map(|(id, _v_score, is_shallow_opt, filetype_opt)| {
            let v_rank = *vector_ranks.get(&id).unwrap_or(&1000) as f32;
            let (f_rank, snippet) = if let Some((r, s)) = fts_matches.get(&id) {
                (*r as f32, s.clone())
            } else {
                (1000.0, String::new())
            };
            
            let mut rrf_score = if is_dummy_vector {
                if f_rank < 1000.0 { 1.0 / (rrf_k + f_rank) } else { 0.0 }
            } else {
                let v_rrf = if v_rank < 1000.0 { 1.0 / (rrf_k + v_rank) } else { 0.0 };
                let f_rrf = if f_rank < 1000.0 { 1.0 / (rrf_k + f_rank) } else { 0.0 };
                v_rrf + (f_rrf * 0.5) 
            };

            let is_fts_match = f_rank < 1000.0;

            if contains_specifics {
                if is_fts_match {
                    rrf_score *= 1.5; 
                } else {
                    rrf_score *= 0.5; 
                }
            }

            let parsed_filename = Path::new(&id).file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let q_lower = raw_query_text.to_lowercase().trim().to_string();
            
            let is_exact = parsed_filename == q_lower || parsed_filename.starts_with(&format!("{}.", q_lower));
            
            if is_exact {
                rrf_score += 0.05_f32;
            } else {
                let terms: Vec<&str> = q_lower.split_whitespace().filter(|w| w.len() > 1).collect();
                let mut all_match = !terms.is_empty();
                let mut any_match = false;
                
                for term in &terms {
                    if parsed_filename.contains(term) {
                        any_match = true;
                    } else {
                        all_match = false;
                    }
                }
                
                if all_match {
                    rrf_score += 0.02_f32;
                } else if any_match {
                    rrf_score += 0.005_f32;
                }
                
                if (all_match || any_match) && is_shallow_opt.as_deref() == Some("true") {
                    rrf_score += 0.01_f32;
                }
            }

            (id, rrf_score, filetype_opt, is_fts_match, snippet, v_rank)
        })
        .filter(|&(_, score, _, _, _, v_rank)| score > 0.005 || (!is_dummy_vector && v_rank <= 10.0))
        .collect();

        scored_candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        // RE-INTRODUCED UPPER THRESHOLD BATCH CAP FOR RAG CONTEXT PURITY:
        // We allow infinite results when executing custom scripts, but for general vector searches or 
        // synthesis prompts, capping the pipeline at 100 high-confidence entries prevents the LLM 
        // context sliding window from choking on noise or system file garbage.
        scored_candidates.truncate(100);

        if prioritize_folders {
            let mut top_folders = Vec::new();
            let mut remaining = Vec::new();
            for cand in scored_candidates {
                if cand.2.as_deref() == Some("directory") && top_folders.len() < 3 {
                    top_folders.push(cand);
                } else {
                    remaining.push(cand);
                }
            }
            scored_candidates = top_folders;
            scored_candidates.extend(remaining);
        }

        let mut results = Vec::new();
        let mut ghosts_to_heal = Vec::new();

        let mut full_texts = HashMap::new();
        if !scored_candidates.is_empty() {
            let conn = match self.conn.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let id_list = scored_candidates.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("SELECT id, content_text FROM documents_fts WHERE id IN ({})", id_list);
            
            if let Ok(mut text_stmt) = conn.prepare(&sql) {
                let params_vec: Vec<&dyn rusqlite::ToSql> = scored_candidates.iter().map(|(id, ..)| id as &dyn rusqlite::ToSql).collect();
                if let Ok(mut rows) = text_stmt.query(rusqlite::params_from_iter(params_vec)) {
                    while let Ok(Some(row)) = rows.next() {
                        let id: String = row.get(0).unwrap_or_default();
                        let text: String = row.get(1).unwrap_or_default();
                        full_texts.insert(id, text);
                    }
                }
            }
            drop(conn);
        }

        for (id, score, _filetype, is_fts_match, fts_snippet, _v_rank) in scored_candidates {
            if !Path::new(&id).exists() {
                ghosts_to_heal.push(id.clone());
                continue; 
            }

            let parsed_filename = Path::new(&id).file_name().unwrap_or_default().to_string_lossy().to_string();
            let mut metadata: HashMap<String, String> = cache_guard.get(&id)
                .map(|doc| doc.metadata.clone())
                .unwrap_or_default();

            let full_text = full_texts.remove(&id).unwrap_or_default();

            let is_eml = metadata.get("filetype").map(|s| s.as_str()) == Some("eml") || parsed_filename.ends_with(".eml");
            if is_eml {
                for line in full_text.lines().take(50) {
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
                
                if let Some(msg_id) = metadata.get("message_id").cloned() {
                    let clean_id = msg_id.trim_matches(|c| c == '<' || c == '>');
                    let encoded_id = clean_id.replace("@", "%40").replace("+", "%2B");
                    metadata.insert("gmail_url".to_string(), format!("https://mail.google.com/mail/u/0/#search/rfc822msgid%3A{}", encoded_id));
                }
            }

            let created_at = metadata.remove("created_at").and_then(|v| v.parse::<u64>().ok());
            let indexed_at = metadata.remove("indexed_at").and_then(|v| v.parse::<u64>().ok());

            let final_snippet = if is_fts_match {
                fts_snippet
            } else {
                if full_text.is_empty() {
                    "Content snippet unavailable.".to_string()
                } else {
                    full_text.chars().take(200).collect::<String>()
                }
            };

            results.push(SearchResult {
                id: id.clone(),
                title: parsed_filename.clone(),
                snippet: final_snippet,
                plugin_id: plugin_id.to_string(),
                score,
                filename: Some(parsed_filename),
                filepath: Some(id),
                metadata,
                created_at,
                indexed_at,
                full_context: Some(full_text.chars().take(2500).collect::<String>()),
                ai_matched: None,
                ai_reasoning: None,
            });
        }

        drop(cache_guard);

        if !ghosts_to_heal.is_empty() {
            let conn = match self.conn.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            conn.execute("BEGIN TRANSACTION", []).ok();
            for ghost in &ghosts_to_heal {
                conn.execute("DELETE FROM documents WHERE id = ?1", params![ghost]).ok();
                conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![ghost]).ok();
            }
            conn.execute("COMMIT", []).ok();
            
            let mut cache_write = self.cache.write().unwrap();
            for ghost in ghosts_to_heal {
                cache_write.remove(&ghost);
            }
        }
        
        println!("[Database] Vector/Hybrid search finalized {} results to return to router.", results.len());

        results
    }
    
    pub fn browse_directory(&self, path: &str) -> Vec<SearchResult> {
        let cache_guard = self.cache.read().unwrap();
        let mut results = Vec::new();
        
        let mut dir_prefix = path.to_string();
        if !dir_prefix.ends_with('/') {
            dir_prefix.push('/');
        }
        let dir_prefix_lower = dir_prefix.to_lowercase();
        let prefix_char_count = dir_prefix.chars().count();

        for doc in cache_guard.values() {
            let doc_id_lower = doc.id.to_lowercase();
            if doc_id_lower.starts_with(&dir_prefix_lower) && doc_id_lower != dir_prefix_lower {
                let remainder: String = doc.id.chars().skip(prefix_char_count).collect();
                if !remainder.is_empty() && !remainder.contains('/') {
                    let parsed_filename = Path::new(&doc.id).file_name().unwrap_or_default().to_string_lossy().to_string();
                    let is_dir = doc.metadata.get("filetype").map(|s| s.as_str()) == Some("directory");
                    
                    results.push(SearchResult {
                        id: doc.id.clone(),
                        title: parsed_filename.clone(),
                        snippet: if is_dir { "Directory".to_string() } else { "File".to_string() },
                        plugin_id: if is_dir { "plugin:directory".to_string() } else { "plugin:vector_db".to_string() },
                        score: if is_dir { 1.0 } else { 0.5 },
                        filename: Some(parsed_filename),
                        filepath: Some(doc.id.clone()),
                        metadata: doc.metadata.clone(),
                        created_at: Some(doc.modified_at),
                        indexed_at: None,
                        full_context: None,
                        ai_matched: None,
                        ai_reasoning: None,
                    });
                }
            }
        }

        results.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });

        results
    }
}