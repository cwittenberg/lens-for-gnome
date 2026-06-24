// src/ingestion/image.rs
use std::path::Path;
use std::fs;
use crate::engine::vision::VisionEngine;
use super::FileExtractor;

// 0.5 MB limit to prevent excessive CPU utilization on high-res photos
const MAX_IMAGE_SIZE_BYTES: u64 = 500 * 1024; 

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

fn generate_freedesktop_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    let path_str = path.to_string_lossy();
    for b in path_str.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                uri.push(b as char);
            }
            _ => {
                uri.push_str(&format!("%{:02X}", b));
            }
        }
    }
    uri
}

fn generate_image_thumbnail(path: &Path) {
    if let Some(home) = std::env::var_os("HOME") {
        let large_dir = Path::new(&home).join(".cache/thumbnails/large");
        let uri = generate_freedesktop_uri(path);
        let hash = format!("{:x}", md5::compute(uri.as_bytes()));
        let thumb_path = large_dir.join(format!("{}.png", hash));
        
        // Fast exit if thumbnail already exists in OS cache
        if thumb_path.exists() {
            return;
        }

        let _ = fs::create_dir_all(&large_dir);
        let temp_path = large_dir.join(format!("{}.tmp.png", hash));

        if let Ok(img) = image::open(path) {
            // Resize to standard 256x256 bounding box
            let thumbnail = img.thumbnail(256, 256);
            if thumbnail.save(&temp_path).is_ok() {
                // Atomic rename prevents corrupted reads by the GNOME UI
                let _ = fs::rename(&temp_path, &thumb_path);
            } else {
                let _ = fs::remove_file(&temp_path);
            }
        }
    }
}

impl FileExtractor for ImageExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "png" | "jpg" | "jpeg" | "bmp" | "webp")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        // Fire-and-forget thumbnail generation into the OS Cache.
        // By placing this BEFORE the size check, the UI gets a preview for 
        // every single image, regardless of how large the file is.
        generate_image_thumbnail(path);

        // Enforce the 500KB file size limit before attempting any heavy OCR
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > MAX_IMAGE_SIZE_BYTES {
                return Err(format!(
                    "Image exceeds 500KB limit ({} bytes). Skipping deep OCR extraction.", 
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