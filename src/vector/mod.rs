// src/vector/mod.rs
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::Mutex;
use rayon::prelude::*;
use crate::domain::SearchResult;
use std::path::Path;

pub struct VectorStore {
    conn: Mutex<Connection>,
}

impl VectorStore {
    pub fn new(db_path: &str) -> Self {
        let conn = Connection::open(db_path).expect("Failed to open SQLite database");
        
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA mmap_size = 10000000000;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -2000000;
            "
        ).expect("Failed to configure in-memory pragmas");

        // --- SCHEMA MIGRATION ---
        // Clean up the old bloated document_contents table to free up space
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
            
            // Reclaim disk space from dropped tables
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

        // --- FTS TOKENIZER MIGRATION ---
        let mut needs_fts_migration = false;
        let mut fts_exists = false;
        if let Ok(mut stmt) = conn.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='documents_fts'") {
            if let Ok(mut rows) = stmt.query([]) {
                if let Ok(Some(row)) = rows.next() {
                    fts_exists = true;
                    let sql: String = row.get(0).unwrap_or_default();
                    // We check if it is using the old trigram tokenizer
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
        
        // FIX: Force re-indexing of all existing documents so text content isn't lost during table drops
        if needs_migration || needs_fts_migration {
            println!("[Database] Resetting modification timestamps to rebuild dropped text indices...");
            conn.execute("UPDATE documents SET modified_at = 0", []).ok();
        }
        // --- END FTS MIGRATION ---

        Self { conn: Mutex::new(conn) }
    }

    /// Dynamically extracts all unique metadata keys present across the user's entire data corpus.
    /// This allows the LLM to know exactly what fields it can filter on without hardcoding logic.
    pub fn get_available_metadata_keys(&self) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let mut keys = Vec::new();
        
        // Use SQLite's native JSON tree walking to extract all unique top-level keys
        if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT key FROM documents, json_each(metadata)") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    if let Ok(key) = row.get::<_, String>(0) {
                        // Filter out internal system keys that the LLM shouldn't try to query semantically
                        if key != "shallow_index" && key != "created_at" && key != "indexed_at" {
                            keys.push(key);
                        }
                    }
                }
            }
        }
        
        keys
    }

    pub fn get_document_modified_at(&self, path: &str) -> Option<u64> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut stmt = conn.prepare("SELECT modified_at FROM documents WHERE id = ?1").ok()?;
        let mut rows = stmt.query(params![path]).ok()?;

        if let Ok(Some(row)) = rows.next() {
            row.get(0).ok()
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn get_shallow_documents(&self) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let mut results = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id FROM documents WHERE json_extract(metadata, '$.shallow_index') = 'true'") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    if let Ok(id) = row.get::<_, String>(0) {
                        results.push(id);
                    }
                }
            }
        }
        results
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
    }

    fn f32_to_bytes(vec: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(vec.len() * 4);
        for v in vec {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        bytes
    }

    fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
        let mut vec = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.chunks_exact(4) {
            vec.push(f32::from_ne_bytes(chunk.try_into().unwrap()));
        }
        vec
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

        // FIX: FTS5 does not allow filtering by unindexed columns in DELETE directly. Must subquery rowid.
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
    }

    #[inline(always)]
    fn fast_cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() { return 0.0; }
        let mut dot_product = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;
        for (val_a, val_b) in a.iter().zip(b.iter()) {
            dot_product += val_a * val_b;
            norm_a += val_a * val_b;
            norm_b += val_b * val_b;
        }
        if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot_product / (norm_a.sqrt() * norm_b.sqrt()) }
    }

    pub fn search(&self, target_embedding: &[f32], raw_query_text: &str, min_ts: Option<u64>, max_ts: Option<u64>, filters: &HashMap<String, String>, plugin_id: &str) -> Vec<SearchResult> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let mut fts_matches = HashMap::new();
        
        let stop_words = [
            "what", "how", "why", "who", "when", "where", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to", "on", "at", "by", "about",
            "show", "me", "find", "search", "get", "looking", "documents", "document", "files", "file", "pictures", "picture", "photos", "photo", "saying", "something", "mentioning", "mentions", "containing", "like", "anything",
            "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar", "op", "bij", "over", "documenten", "bestanden", "foto", "fotos",
            "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en", "documentos", "archivos", "fotos"
        ];

        // 1. Lexical Extraction Phase: Detect explicit exact phrases bound by quotes
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
        
        // CTO FIX: Retain hyphens (-) and underscores (_) for UUIDs, server names, and API keys.
        let clean_query_text: String = raw_query_text.to_lowercase().chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
            .collect();

        // Build semantic OR matches
        let semantic_query = clean_query_text
            .split_whitespace()
            .filter(|w| w.len() > 2 && !stop_words.contains(w) && !exact_phrases.contains(&w.to_string()))
            .map(|w| format!("\"{}\"", w))
            .collect::<Vec<_>>()
            .join(" OR ");

        // Build final FTS query merging semantic ORs with mandatory exact ANDs
        let mut safe_query = semantic_query;
        if !exact_phrases.is_empty() {
            let exact_fts: String = exact_phrases.iter()
                .map(|p| format!("\"{}\"", p.replace("\"", ""))) // Escape nested quotes just in case
                .collect::<Vec<_>>()
                .join(" AND ");
                
            if safe_query.is_empty() {
                safe_query = exact_fts;
            } else {
                safe_query = format!("({}) AND {}", safe_query, exact_fts);
            }
        }
            
        if !safe_query.is_empty() {
            if let Ok(mut fts_stmt) = conn.prepare(
                "SELECT id, snippet(documents_fts, 2, '<b>', '</b>', '...', 30) as snip
                 FROM documents_fts
                 WHERE documents_fts MATCH ?1
                 ORDER BY rank LIMIT 100"
            ) {
                if let Ok(mut rows) = fts_stmt.query(params![safe_query]) {
                    let mut rank = 1;
                    while let Ok(Some(row)) = rows.next() {
                        let id: String = row.get(0).unwrap();
                        let snippet: String = row.get(1).unwrap();
                        fts_matches.insert(id, (rank, snippet));
                        rank += 1;
                    }
                }
            }
        }

        // 2. Fetch all candidates for Vector Scoring
        let mut sql = String::from("SELECT id, embedding, metadata FROM documents WHERE 1=1");
        
        if let Some(min) = min_ts { sql.push_str(&format!(" AND modified_at >= {}", min)); }
        if let Some(max) = max_ts { sql.push_str(&format!(" AND modified_at <= {}", max)); }
        
        for (key, val) in filters {
            sql.push_str(&format!(" AND json_extract(metadata, '$.{}') = '{}'", key, val));
        }

        let mut stmt = conn.prepare(&sql).expect("Failed to prepare search statement");
        
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            let metadata_json: String = row.get(2)?;
            Ok((id, Self::bytes_to_f32(&blob), metadata_json))
        }).unwrap();

        let mut candidate_records = Vec::new();
        for record in rows.flatten() {
            candidate_records.push(record);
        }

        // 3. Score Vector Similarities
        let mut vector_scores: Vec<_> = candidate_records.par_iter().map(|(id, embedding, _)| {
            (id.clone(), Self::fast_cosine_similarity(target_embedding, embedding))
        }).collect();

        vector_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        let mut vector_ranks = HashMap::new();
        for (i, (id, _score)) in vector_scores.iter().enumerate() {
            vector_ranks.insert(id.clone(), i + 1);
        }

        let contains_specifics = !exact_phrases.is_empty() || raw_query_text.split_whitespace()
            .any(|w| w.chars().any(|c| c.is_ascii_digit()));

        // 4. Combine via Reciprocal Rank Fusion (RRF)
        let rrf_k = 60.0;
        
        let mut scored_candidates: Vec<_> = candidate_records.into_iter().map(|(id, _emb, metadata_json)| {
            let v_rank = *vector_ranks.get(&id).unwrap_or(&1000) as f32;
            
            let (f_rank, snippet) = if let Some((r, s)) = fts_matches.get(&id) {
                (*r as f32, s.clone())
            } else {
                (1000.0, String::new())
            };
            
            let v_score = if v_rank < 1000.0 { 1.0 / (rrf_k + v_rank) } else { 0.0 };
            let f_score = if f_rank < 1000.0 { 1.0 / (rrf_k + f_rank) } else { 0.0 };
            
            let mut rrf_score = v_score + f_score;
            let is_fts_match = f_rank < 1000.0;

            if contains_specifics {
                if is_fts_match {
                    // Heavily boost documents containing exact phrases or serial numbers
                    rrf_score *= 3.0; 
                } else {
                    rrf_score *= 0.05; 
                }
            }

            (id, rrf_score, metadata_json, is_fts_match, snippet, v_rank)
        })
        .filter(|&(_, score, _, _, _, v_rank)| score > 0.005 || v_rank <= 10.0)
        .collect();

        scored_candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        scored_candidates.truncate(15); 

        let mut results = Vec::new();
        for (id, score, metadata_json, is_fts_match, fts_snippet, _v_rank) in scored_candidates {
            let parsed_filename = Path::new(&id).file_name().unwrap_or_default().to_string_lossy().to_string();
            let mut metadata: HashMap<String, String> = serde_json::from_str(&metadata_json).unwrap_or_default();
            
            let created_at = metadata.remove("created_at").and_then(|v| v.parse::<u64>().ok());
            let indexed_at = metadata.remove("indexed_at").and_then(|v| v.parse::<u64>().ok());

            let mut text_stmt = conn.prepare("SELECT content_text FROM documents_fts WHERE id = ?1").unwrap();
            let full_text: String = text_stmt.query_row(params![&id], |row| row.get(0)).unwrap_or_default();

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

        results
    }
}