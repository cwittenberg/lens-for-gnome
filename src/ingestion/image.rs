// src/ingestion/image.rs
use std::path::Path;
use crate::engine::vision::VisionEngine;
use super::FileExtractor;

// 0.5 MB limit to prevent excessive CPU utilization on high-res photos
const MAX_IMAGE_SIZE_BYTES: u64 = 500*1024; 

pub struct ImageExtractor {
    vision: VisionEngine,
}

impl ImageExtractor {
    pub fn new() -> Self {
        Self {
            vision: VisionEngine::new(),
        }
    }
}

impl FileExtractor for ImageExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "png" | "jpg" | "jpeg" | "bmp" | "webp")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        // Enforce the 1MB file size limit before attempting any OCR
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > MAX_IMAGE_SIZE_BYTES {
                return Err(format!(
                    "Image exceeds 1MB limit ({} bytes). Skipping deep OCR extraction.", 
                    metadata.len()
                ));
            }
        }

        let path_str = path.to_string_lossy().to_string();
        
        // Route the file through the new Vision Engine
        let result = self.vision.process_image(&path_str);
        
        // Direct indexing allows precise type inference for serde_json
        if let Some(text) = result["text"].as_str() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Err("No identifiable text or QR matrix found in image".to_string())
            } else {
                Ok(trimmed.to_string())
            }
        } else {
            Err("Vision engine failed to process the image".to_string())
        }
    }
}