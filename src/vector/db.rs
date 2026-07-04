use rusqlite::Connection;
use std::collections::HashMap;
use crate::vector::models::CachedDoc;
use crate::vector::security::SecurityManager;

/// Factory: Abstracts away SQLite initialization, pragmas, and schema migrations.
pub struct DatabaseFactory;

impl DatabaseFactory {
    pub fn create(db_path: &str) -> (Connection, HashMap<String, CachedDoc>) {
        let db_key = SecurityManager::resolve_or_generate_key(db_path);
        let mut conn = Connection::open(db_path).expect("Failed to open SQLite database");

        let init_db = |c: &Connection, k: &Option<String>| -> Result<(), rusqlite::Error> {
            if let Some(key) = k {
                c.execute_batch(&format!("PRAGMA key = '{}';", key))?;
            }
            c.execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA mmap_size = 30000000000; 
                PRAGMA temp_store = MEMORY;
                PRAGMA cache_size = -2000000;
                "
            )?;
            // Force a read to immediately trigger SQLCipher decryption failure if the key is wrong or file is plaintext.
            let _: i64 = c.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?; 
            Ok(())
        };

        if let Err(e) = init_db(&conn, &db_key) {
            if e.to_string().contains("not a database") {
                eprintln!("[Security] Existing database is unencrypted or uses an invalid key. Purging to rebuild securely...");
                drop(conn); // Release the file lock
                let _ = std::fs::remove_file(db_path);
                let _ = std::fs::remove_file(format!("{}-wal", db_path));
                let _ = std::fs::remove_file(format!("{}-shm", db_path));
                
                conn = Connection::open(db_path).expect("Failed to recreate SQLite database");
                init_db(&conn, &db_key).expect("Failed to initialize encrypted pragmas on fresh database");
            } else {
                panic!("Failed to configure SQLite database: {}", e);
            }
        }

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

        (conn, cache)
    }
}