use std::path::Path;
use std::process::Command;
use crate::engine::vision::VisionEngine;
use super::FileExtractor;

pub struct PdfExtractor {
    vision: VisionEngine,
    max_extraction_bytes: usize,
}

impl PdfExtractor {
    pub fn new(max_extraction_bytes: usize) -> Self {
        Self {
            vision: VisionEngine::new(),
            max_extraction_bytes,
        }
    }
}

impl FileExtractor for PdfExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        extension == "pdf"
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        // Attempt native embedded text extraction first
        let mut text = pdf_extract::extract_text(path).unwrap_or_default();

        // If no text layer is detected (e.g. scanned document), fallback to Vision Engine pipeline
        if text.trim().is_empty() {
            let temp_prefix = format!("/tmp/gnome_lens_pdf_{}", std::process::id());
            
            // Use pdftoppm (poppler-utils) to rasterize PDF pages into temporary image buffers
            if let Ok(output) = Command::new("pdftoppm")
                .arg("-png")
                .arg(path)
                .arg(&temp_prefix)
                .output()
            {
                if output.status.success() {
                    if let Ok(entries) = std::fs::read_dir("/tmp") {
                        let mut png_pages = Vec::new();
                        let prefix_matcher = format!("gnome_lens_pdf_{}", std::process::id());
                        
                        // Gather all generated page buffers
                        for entry in entries.flatten() {
                            let file_name = entry.file_name().to_string_lossy().to_string();
                            if file_name.starts_with(&prefix_matcher) && file_name.ends_with(".png") {
                                png_pages.push(entry.path());
                            }
                        }
                        
                        // Sort alphabetically to maintain correct document chronological order
                        png_pages.sort();
                        
                        // Pass each rasterized page matrix through the Vision Engine OCR pipeline
                        for img_path in png_pages {
                            // Only run expensive OCR if we are under the extraction size limit
                            if text.len() < self.max_extraction_bytes {
                                let result = self.vision.process_image(&img_path.to_string_lossy());
                                if let Some(ocr_text) = result["text"].as_str() {
                                    text.push_str(ocr_text);
                                    text.push_str("\n\n");
                                }
                            }
                            // Clean up buffer instantly to prevent disk bloat (even if skipped)
                            let _ = std::fs::remove_file(img_path);
                        }
                    }
                }
            }
        }

        if text.trim().is_empty() {
            Err("No identifiable text found in PDF (even after VisionEngine OCR fallback)".to_string())
        } else {
            Ok(text)
        }
    }
}