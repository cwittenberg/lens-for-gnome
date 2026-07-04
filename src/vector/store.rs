use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::path::Path;
use crate::domain::SearchResult;
use crate::vector::models::CachedDoc;
use crate::vector::db::DatabaseFactory;
use crate::vector::search::HybridSearchEngine;

/// Repository / Facade Pattern: Unified gateway interfacing with the application layer
/// to coordinate the DB, memory cache, and search algorithms seamlessly.
pub struct VectorStore {
    conn: Mutex<Connection>,
    db_path: String,
    cache: Arc<RwLock<HashMap<String, CachedDoc>>>,
}

impl VectorStore {
    pub fn new(db_path: &str) -> Self {
        let (conn, cache) = DatabaseFactory::create(db_path);
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

    pub fn update_document_timestamp(&self, path: &str, modified_at: u64) {
        let conn = match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        conn.execute("UPDATE documents SET modified_at = ?1 WHERE id = ?2", rusqlite::params![modified_at, path]).ok();
        
        let mut cache_guard = self.cache.write().unwrap();
        if let Some(doc) = cache_guard.get_mut(path) {
            doc.modified_at = modified_at;
        }
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
        HybridSearchEngine::search(
            &self.conn,
            &self.cache,
            target_embedding,
            raw_query_text,
            min_ts,
            max_ts,
            filters,
            directory_filter,
            plugin_id,
            prioritize_folders
        )
    }
    
    pub fn browse_directory(&self, path: &str) -> Vec<SearchResult> {
        HybridSearchEngine::browse_directory(&self.cache, path)
    }
}