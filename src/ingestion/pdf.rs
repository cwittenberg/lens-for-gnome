use std::path::Path;
use super::FileExtractor;

pub struct PdfExtractor;

impl FileExtractor for PdfExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        extension == "pdf"
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        pdf_extract::extract_text(path).map_err(|e| format!("PDF Error: {:?}", e))
    }
}