use std::path::Path;
use super::FileExtractor;

pub struct TxtExtractor;

impl FileExtractor for TxtExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "txt" | "md" | "rs" | "js" | "json" | "xml" | "html")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}