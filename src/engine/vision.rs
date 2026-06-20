// src/engine/vision.rs
use std::path::Path;
use std::process::Command;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use image::{DynamicImage, GenericImageView};
use super::smart_extract::SmartExtractor;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TesseractResult {
    text: String,
    confidence: f64,
    word_count: usize,
    char_count: usize,
    garbage_ratio: f64,
}

pub struct VisionEngine {
    smart_extractor: SmartExtractor,
    debug_mode: bool,
}

impl VisionEngine {
    pub fn new() -> Self {
        let debug_mode = env::var("DEBUG_VISION_OCR").unwrap_or_else(|_| "0".to_string()) == "1";
        if debug_mode {
            println!("[DEBUG] Vision OCR debugging is ENABLED. Raw text will be printed to stdout.");
        }

        Self {
            smart_extractor: SmartExtractor::new(),
            debug_mode,
        }
    }

    /// Primary entry point for analyzing images and extracting embedded text or data natively.
    pub fn process_image(&self, path: &str) -> serde_json::Value {
        if !Path::new(path).exists() {
            return serde_json::json!({
                "type": "error",
                "text": "",
                "confidence": 0.0,
                "entities": []
            });
        }

        // 1. Native QR Code Scanning pass (100% Pure Rust via rqrr)
        let qr_data = self.extract_qr(path);
        // rqrr can sometimes return 1-2 char false positives on checkered patterns.
        // We enforce a minimum length to ensure it's a valid QR payload.
        if qr_data.len() > 3 {
            if self.debug_mode {
                println!("\n=======================================================================================");
                println!("[DEBUG] RAW VISION QR OUTPUT FOR: {}", path);
                println!("---------------------------------------------------------------------------------------");
                println!("{}", qr_data.trim());
                println!("=======================================================================================\n");
            }

            return serde_json::json!({
                "type": "qr",
                "text": qr_data,
                "confidence": 1.0,
                "entities": self.smart_extractor.extract_entities(&qr_data)
            });
        }

        // 2. Load and analyze the image matrix for visual characteristics
        let img = match image::open(path) {
            Ok(opened) => opened,
            Err(_) => {
                return serde_json::json!({
                    "type": "error",
                    "text": "Failed to open or parse image matrix",
                    "confidence": 0.0,
                    "entities": []
                });
            }
        };

        // Determine optimum Page Segmentation Mode (PSM) for Tesseract based on dimensions
        let (width, height) = img.dimensions();
        let aspect_ratio = if height > 0 { width as f32 / height as f32 } else { 1.0 };
        
        let mut primary_psm = "6";
        let mut fallback_psm = "11";

        if height <= 90 && aspect_ratio >= 4.0 {
            primary_psm = "7"; fallback_psm = "13";
        } else if width <= 220 && height <= 100 {
            primary_psm = "8"; fallback_psm = "7";
        } else if width >= 900 && height >= 900 {
            primary_psm = "3"; fallback_psm = "6";
        }

        // Tesseract accuracy tanks on small bounding boxes. Upscale natively by 300% if needed.
        let scaled_img = if width < 1500 && height < 1500 {
            img.resize(width * 3, height * 3, image::imageops::FilterType::CatmullRom)
        } else {
            img
        };

        // CRITICAL FIX: Screenshots often have an alpha channel (transparent background).
        // Tesseract interprets transparency as black, making black text on transparent backgrounds invisible.
        // We must flatten the alpha channel onto a solid white background before OCR.
        let rgba_img = scaled_img.to_rgba8();
        let mut rgb_img = image::RgbImage::new(scaled_img.width(), scaled_img.height());
        for (x, y, pixel) in rgba_img.enumerate_pixels() {
            let alpha = pixel[3] as f32 / 255.0;
            // Mix the pixel color with a white (255) background based on alpha density
            let r = ((1.0 - alpha) * 255.0 + alpha * pixel[0] as f32) as u8;
            let g = ((1.0 - alpha) * 255.0 + alpha * pixel[1] as f32) as u8;
            let b = ((1.0 - alpha) * 255.0 + alpha * pixel[2] as f32) as u8;
            rgb_img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
        
        let mut safe_img = image::DynamicImage::ImageRgb8(rgb_img);
        let brightness = self.calculate_mean_brightness(&safe_img);

        // Dark Mode Fix: Pre-invert colors if the image is mostly dark
        if brightness < 0.45 {
            safe_img.invert();
        }
        
        let temp_flattened = format!("/tmp/gnome_lens_flat_{}_{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        let _ = safe_img.save(&temp_flattened);

        // 3. Process Primary OCR
        let res1 = self.run_tesseract_pass(&temp_flattened, primary_psm);

        let mut accept = false;
        if let Some(ref r) = res1 {
            if r.word_count > 0 && r.char_count >= 2 && r.confidence >= 65.0 && r.garbage_ratio < 0.35 {
                accept = true; // Results are excellent, skip fallback passes
            }
        }

        let mut final_res = res1.unwrap_or(TesseractResult {
            text: String::new(),
            confidence: 0.0,
            word_count: 0,
            char_count: 0,
            garbage_ratio: 0.0,
        });

        // 4. Fallback Validation Passes
        if !accept {
            let res2 = self.run_tesseract_pass(&temp_flattened, fallback_psm);
            
            // Execute a third negated pass to handle hollow/meme text (e.g. white text with dark outline)
            let mut inverted_img = safe_img.clone();
            inverted_img.invert();
            let temp_inverted = format!("/tmp/gnome_lens_inv_{}_{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
            
            let mut res3 = None;
            if inverted_img.save(&temp_inverted).is_ok() {
                res3 = self.run_tesseract_pass(&temp_inverted, primary_psm);
                let _ = std::fs::remove_file(&temp_inverted);
            }

            let score1 = Self::calculate_score(&final_res);
            let score2 = res2.as_ref().map(|r| Self::calculate_score(r)).unwrap_or(-9999.0);
            let score3 = res3.as_ref().map(|r| Self::calculate_score(r)).unwrap_or(-9999.0);

            let max_score = score1.max(score2).max(score3);

            if max_score == score3 && score3 > -999.0 {
                final_res = res3.unwrap();
            } else if max_score == score2 && score2 > -999.0 {
                final_res = res2.unwrap();
            }
        }

        let _ = std::fs::remove_file(&temp_flattened);

        if self.debug_mode {
            println!("\n=======================================================================================");
            println!("[DEBUG] RAW VISION OCR OUTPUT FOR: {}", path);
            println!("---------------------------------------------------------------------------------------");
            println!("{}", final_res.text.trim());
            println!("=======================================================================================\n");
        }

        let entities = self.smart_extractor.extract_entities(&final_res.text);

        serde_json::json!({
            "type": "ocr",
            "text": final_res.text,
            // Convert native 0-100 confidence scale to 0.0-1.0 representation for IPC
            "confidence": final_res.confidence / 100.0,
            "entities": entities
        })
    }

    /// Scans the image for valid QR matrix specifications using pure Rust decoding
    fn extract_qr(&self, path: &str) -> String {
        if let Ok(img) = image::open(path) {
            let gray_img = img.to_luma8();
            let mut prepared = rqrr::PreparedImage::prepare(gray_img);
            let grids = prepared.detect_grids();
            for grid in grids {
                if let Ok((_meta, decoded_content)) = grid.decode() {
                    return decoded_content;
                }
            }
        }
        String::new()
    }

    /// Evaluates the mean light level of the pixel buffer to detect dark themes or screenshots
    fn calculate_mean_brightness(&self, img: &DynamicImage) -> f64 {
        let luma = img.to_luma8();
        let pixels = luma.as_raw();
        if pixels.is_empty() {
            return 0.0;
        }
        let total: u64 = pixels.iter().map(|&p| p as u64).sum();
        (total as f64) / (pixels.len() as f64) / 255.0
    }

    /// Executes Tesseract natively from the Rust Daemon using std::process and parses TSV metrics
    fn run_tesseract_pass(&self, image_path: &str, psm: &str) -> Option<TesseractResult> {
        let tmp_prefix = format!("/tmp/tess_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        
        let output = Command::new("tesseract")
            .arg(image_path)
            .arg(&tmp_prefix)
            .arg("-l")
            .arg("eng")
            .arg("--dpi")
            .arg("300")
            .arg("--oem")
            .arg("1")
            .arg("--psm")
            .arg(psm)
            .arg("txt")
            .arg("tsv")
            .output().ok()?;

        if !output.status.success() {
            return None;
        }

        let txt_path = format!("{}.txt", tmp_prefix);
        let tsv_path = format!("{}.tsv", tmp_prefix);

        let mut text = std::fs::read_to_string(&txt_path).unwrap_or_default();
        let tsv = std::fs::read_to_string(&tsv_path).unwrap_or_default();

        let _ = std::fs::remove_file(&txt_path);
        let _ = std::fs::remove_file(&tsv_path);

        // CRITICAL FIX: Tesseract frequently hallucinates the letter 'e' 
        // for standard bullet points. We must convert these back to proper 
        // markdown lists so the LLM understands it's reading items.
        let mut cleaned = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("e ") && trimmed.len() > 2 {
                cleaned.push(format!("- {}", &trimmed[2..].trim()));
            } else if trimmed == "e" {
                continue;
            } else {
                cleaned.push(line.to_string());
            }
        }
        text = cleaned.join("\n");

        // Strip excessive hallucinated newlines from empty areas
        while text.contains("\n\n\n") {
            text = text.replace("\n\n\n", "\n\n");
        }
        let text = text.trim().to_string();

        // Calculate Quality Metrics via TSV Output
        let mut total_conf = 0.0;
        let mut word_count = 0;

        // Parse Tesseract TSV Headers: level, page_num, block_num, par_num, line_num, word_num, left, top, width, height, conf, text
        for line in tsv.lines().skip(1) {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() >= 12 {
                if let Ok(conf) = cols[10].parse::<f64>() {
                    let word_text = cols[11].trim();
                    // Tesseract TSV uses -1 conf for block/paragraph container rows; we only want valid text words
                    if !word_text.is_empty() && conf >= 0.0 {
                        total_conf += conf;
                        word_count += 1;
                    }
                }
            }
        }

        let confidence = if word_count > 0 { total_conf / (word_count as f64) } else { 0.0 };
        let char_count = text.len();

        let garbage_count = text.chars().filter(|c| {
            !c.is_alphanumeric() && !c.is_whitespace() && !".,!?@/:-'\"()[]{}_+=$%".contains(*c)
        }).count();

        let garbage_ratio = if char_count > 0 { (garbage_count as f64) / (char_count as f64) } else { 0.0 };

        Some(TesseractResult {
            text,
            confidence,
            word_count,
            char_count,
            garbage_ratio,
        })
    }

    /// Determines which OCR result is better when multiple passes were executed.
    fn calculate_score(res: &TesseractResult) -> f64 {
        res.confidence 
            + (res.word_count.min(20) as f64) * 0.5 
            + (res.char_count.min(160) as f64) * 0.03 
            - (res.garbage_ratio * 25.0)
    }
}