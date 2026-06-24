// src/vector/mod.rs
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::Mutex;
use crate::domain::SearchResult;
use std::path::Path;

pub struct VectorStore {
    conn: Mutex<Connection>,
    db_path: String,
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

        Self { 
            conn: Mutex::new(conn),
            db_path: db_path.to_string(),
        }
    }

    pub fn get_db_stats(&self) -> (usize, u64) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let count: usize = conn.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0)).unwrap_or(0);
        
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
    }

    pub fn prune_orphans(&self) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut ids = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id FROM documents") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    if let Ok(id) = row.get::<_, String>(0) {
                        ids.push(id);
                    }
                }
            }
        }

        let mut orphans = Vec::new();
        for id in ids {
            if !Path::new(&id).exists() {
                orphans.push(id);
            }
        }

        if !orphans.is_empty() {
            conn.execute("BEGIN TRANSACTION", []).ok();
            for orphan in &orphans {
                conn.execute("DELETE FROM documents WHERE id = ?1", params![orphan]).ok();
                conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![orphan]).ok();
            }
            conn.execute("COMMIT", []).ok();
            
            conn.execute("VACUUM", []).ok();
        }
    }

    pub fn get_available_metadata_keys(&self) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let mut keys = Vec::new();
        
        if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT key FROM documents, json_each(metadata)") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    if let Ok(key) = row.get::<_, String>(0) {
                        if key != "shallow_index" && key != "created_at" && key != "indexed_at" {
                            keys.push(key);
                        }
                    }
                }
            }
        }
        
        keys
    }

    pub fn get_all_document_timestamps(&self) -> HashMap<String, u64> {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let mut map = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, modified_at FROM documents") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(row)) = rows.next() {
                    if let (Ok(id), Ok(ts)) = (row.get::<_, String>(0), row.get::<_, u64>(1)) {
                        map.insert(id, ts);
                    }
                }
            }
        }
        map
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
            .map(|w| format!("\"{}\"*", w)) 
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
                        let id: String = row.get(0).unwrap_or_default();
                        let snippet: String = row.get(1).unwrap_or_default();
                        fts_matches.insert(id, (rank, snippet));
                        rank += 1;
                    }
                }
            }
        }

        let is_dummy_vector = target_embedding.is_empty() || target_embedding.iter().all(|&v| v == 0.0);
        let mut candidate_scores = Vec::new();

        if is_dummy_vector {
            for (id, (rank, _)) in &fts_matches {
                let mut sql = String::from("SELECT json_extract(metadata, '$.shallow_index'), json_extract(metadata, '$.filetype') FROM documents WHERE id = ?1");
                if let Some(min) = min_ts { sql.push_str(&format!(" AND modified_at >= {}", min)); }
                if let Some(max) = max_ts { sql.push_str(&format!(" AND modified_at <= {}", max)); }
                if let Some(dir) = directory_filter {
                    let safe_dir = dir.replace("'", "''");
                    sql.push_str(&format!(" AND id LIKE '{}%'", safe_dir));
                }
                for (key, val) in filters {
                    let safe_val = val.replace("'", "''");
                    sql.push_str(&format!(" AND json_extract(metadata, '$.{}') = '{}'", key, safe_val));
                }
                
                if let Ok(mut check_stmt) = conn.prepare(&sql) {
                    if let Ok(row) = check_stmt.query_row(params![id], |r| {
                        let s: Option<String> = r.get(0).unwrap_or(None);
                        let f: Option<String> = r.get(1).unwrap_or(None);
                        Ok((s, f))
                    }) {
                        let base_score = 1.0 / (*rank as f32);
                        candidate_scores.push((id.clone(), base_score, row.0, row.1));
                    }
                }
            }
        } else {
            let mut sql = String::from("SELECT id, embedding, json_extract(metadata, '$.shallow_index'), json_extract(metadata, '$.filetype') FROM documents WHERE 1=1");
            if let Some(min) = min_ts { sql.push_str(&format!(" AND modified_at >= {}", min)); }
            if let Some(max) = max_ts { sql.push_str(&format!(" AND modified_at <= {}", max)); }
            if let Some(dir) = directory_filter {
                let safe_dir = dir.replace("'", "''");
                sql.push_str(&format!(" AND id LIKE '{}%'", safe_dir));
            }
            for (key, val) in filters {
                let safe_val = val.replace("'", "''");
                sql.push_str(&format!(" AND json_extract(metadata, '$.{}') = '{}'", key, safe_val));
            }

            if let Ok(mut stmt) = conn.prepare(&sql) {
                let norm_a: f32 = target_embedding.iter().map(|v| v * v).sum::<f32>().sqrt().max(0.0001);
                if let Ok(mut rows) = stmt.query([]) {
                    while let Ok(Some(row)) = rows.next() {
                        let id: String = row.get(0).unwrap_or_default();
                        let v_score = if let Ok(blob_ref) = row.get_ref(1) {
                            if let Ok(bytes) = blob_ref.as_blob() {
                                let mut dot_product = 0.0;
                                let mut norm_b = 0.0;
                                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                                    if i >= target_embedding.len() { break; }
                                    let val_b = f32::from_ne_bytes(chunk.try_into().unwrap_or([0; 4]));
                                    let val_a = target_embedding[i];
                                    dot_product += val_a * val_b;
                                    norm_b += val_b * val_b;
                                }
                                if norm_b == 0.0 { 0.0 } else { dot_product / (norm_a * norm_b.sqrt()) }
                            } else { 0.0 }
                        } else { 0.0 };
                        
                        let is_shallow_opt: Option<String> = row.get(2).unwrap_or(None);
                        let filetype_opt: Option<String> = row.get(3).unwrap_or(None);
                        candidate_scores.push((id, v_score, is_shallow_opt, filetype_opt));
                    }
                }
            }
        }

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
                v_rrf + f_rrf
            };

            let is_fts_match = f_rank < 1000.0;

            if contains_specifics {
                if is_fts_match {
                    rrf_score *= 3.0; 
                } else {
                    rrf_score *= 0.05; 
                }
            }

            let parsed_filename = Path::new(&id).file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let q_lower = raw_query_text.to_lowercase().trim().to_string();
            
            let is_exact = parsed_filename == q_lower || parsed_filename.starts_with(&format!("{}.", q_lower));
            
            if is_exact {
                rrf_score += 15.0_f32;
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
                    rrf_score += 5.0_f32;
                } else if any_match {
                    rrf_score += 1.5_f32;
                }
                
                if (all_match || any_match) && is_shallow_opt.as_deref() == Some("true") {
                    rrf_score += 3.0_f32;
                }
            }

            (id, rrf_score, filetype_opt, is_fts_match, snippet, v_rank)
        })
        .filter(|&(_, score, _, _, _, v_rank)| score > 0.005 || (!is_dummy_vector && v_rank <= 10.0))
        .collect();

        scored_candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
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
            remaining.truncate(100 - top_folders.len());
            scored_candidates = top_folders;
            scored_candidates.extend(remaining);
        } else {
            scored_candidates.truncate(100); 
        }

        let mut results = Vec::new();
        let mut ghosts_to_heal = Vec::new();

        if let Ok(mut meta_stmt) = conn.prepare("SELECT metadata FROM documents WHERE id = ?1") {
            if let Ok(mut text_stmt) = conn.prepare("SELECT content_text FROM documents_fts WHERE id = ?1") {
                for (id, score, _filetype, is_fts_match, fts_snippet, _v_rank) in scored_candidates {
                    if !Path::new(&id).exists() {
                        ghosts_to_heal.push(id.clone());
                        continue; 
                    }

                    let parsed_filename = Path::new(&id).file_name().unwrap_or_default().to_string_lossy().to_string();
                    let metadata_json: String = meta_stmt.query_row(params![&id], |row| row.get(0)).unwrap_or_else(|_| "{}".to_string());
                    let mut metadata: HashMap<String, String> = serde_json::from_str(&metadata_json).unwrap_or_default();
                    let full_text: String = text_stmt.query_row(params![&id], |row| row.get(0)).unwrap_or_default();

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
            }
        }

        if !ghosts_to_heal.is_empty() {
            conn.execute("BEGIN TRANSACTION", []).ok();
            for ghost in ghosts_to_heal {
                conn.execute("DELETE FROM documents WHERE id = ?1", params![&ghost]).ok();
                conn.execute("DELETE FROM documents_fts WHERE rowid IN (SELECT rowid FROM documents_fts WHERE id = ?1)", params![&ghost]).ok();
            }
            conn.execute("COMMIT", []).ok();
        }

        results
    }
}