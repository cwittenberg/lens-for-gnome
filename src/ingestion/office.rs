// src/ingestion/office.rs
use std::path::Path;
use std::io::Read;
use dotext::{Docx, Pptx, Odt, Odp, MsDoc};
use dotext::doc::OpenOfficeDoc;
use super::FileExtractor;

pub struct ModernOfficeExtractor;

impl FileExtractor for ModernOfficeExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "docx" | "pptx" | "odt" | "odp")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let mut content = String::new();
        
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext.to_lowercase().as_str() {
                "docx" => {
                    let mut doc = Docx::open(path).map_err(|e| e.to_string())?;
                    doc.read_to_string(&mut content).map_err(|e| e.to_string())?;
                },
                "pptx" => {
                    let mut doc = Pptx::open(path).map_err(|e| e.to_string())?;
                    doc.read_to_string(&mut content).map_err(|e| e.to_string())?;
                },
                "odt" => {
                    let mut doc = Odt::open(path).map_err(|e| e.to_string())?;
                    doc.read_to_string(&mut content).map_err(|e| e.to_string())?;
                },
                "odp" => {
                    let mut doc = Odp::open(path).map_err(|e| e.to_string())?;
                    doc.read_to_string(&mut content).map_err(|e| e.to_string())?;
                },
                _ => return Err("Unsupported modern office format".to_string()),
            }
        } else {
            return Err("File has no extension".to_string());
        }
        
        Ok(content)
    }
}