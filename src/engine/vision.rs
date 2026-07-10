// src/engine/vision.rs

use std::path::Path;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use image::{DynamicImage, GenericImageView};

use super::smart_extract::SmartExtractor;
use super::RuntimeAdapter;

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
    runtime_adapter: Arc<RuntimeAdapter>,
}

impl VisionEngine {
    pub fn new(runtime_adapter: Arc<RuntimeAdapter>) -> Self {
        let debug_mode = env::var("DEBUG_VISION_OCR").unwrap_or_else(|_| "0".to_string()) == "1";
        
        if debug_mode {
            println!("[DEBUG] Vision OCR debugging is ENABLED. Raw text will be printed to stdout.");
        }
        
        Self {
            smart_extractor: SmartExtractor::new(),
            debug_mode,
            runtime_adapter,
        }
    }

    pub fn process_image(&self, path: &str) -> serde_json::Value {
        if !Path::new(path).exists() {
            return serde_json::json!({
                "type": "error",
                "text": "",
                "confidence": 0.0,
                "entities": []
            });
        }

        let qr_data = self.extract_qr(path);
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

        let scaled_img = if width < 1500 && height < 1500 {
            img.resize(width * 2, height * 2, image::imageops::FilterType::Triangle)
        } else {
            img
        };

        let rgba_img = scaled_img.to_rgba8();
        let mut rgb_img = image::RgbImage::new(scaled_img.width(), scaled_img.height());

        for (x, y, pixel) in rgba_img.enumerate_pixels() {
            let alpha = pixel[3] as f32 / 255.0;
            let r = ((1.0 - alpha) * 255.0 + alpha * pixel[0] as f32) as u8;
            let g = ((1.0 - alpha) * 255.0 + alpha * pixel[1] as f32) as u8;
            let b = ((1.0 - alpha) * 255.0 + alpha * pixel[2] as f32) as u8;
            rgb_img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
        
        let mut safe_img = image::DynamicImage::ImageRgb8(rgb_img);

        let brightness = self.calculate_mean_brightness(&safe_img);
        if brightness < 0.45 {
            safe_img.invert();
        }
        
        let temp_flattened = format!("/tmp/lens_for_gnome_flat_{}_{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        let _ = safe_img.save(&temp_flattened);

        let res1 = self.run_tesseract_pass(&temp_flattened, primary_psm);

        let mut accept = false;
        if let Some(ref r) = res1 {
            if r.word_count > 0 && r.char_count >= 2 && r.confidence >= 65.0 && r.garbage_ratio < 0.35 {
                accept = true;
            }
        }

        let mut final_res = res1.unwrap_or(TesseractResult {
            text: String::new(),
            confidence: 0.0,
            word_count: 0,
            char_count: 0,
            garbage_ratio: 0.0,
        });

        if !accept {
            let res2 = self.run_tesseract_pass(&temp_flattened, fallback_psm);
            
            let mut inverted_img = safe_img.clone();
            inverted_img.invert();
            let temp_inverted = format!("/tmp/lens_for_gnome_inv_{}_{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
            
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

        // --- LINE-BY-LINE SURGICAL QUALITY GATE ---
        let mut final_text = String::new();
        let mut is_gibberish = false;

        if let Some(cleaned) = self.sanitize_ocr_text(&final_res.text) {
            final_text = cleaned;
        } else {
            is_gibberish = true;
            if self.debug_mode {
                println!("[DEBUG] OCR REJECTED -> Entire block was structural noise or lacked sufficient proper text.");
            }
        }

        if is_gibberish {
            final_res.text = String::new();
            final_res.confidence = 0.0;
        } else {
            final_res.text = final_text;
        }

        if self.debug_mode && !final_res.text.is_empty() {
            println!("\n================================================================");
            println!("[DEBUG] RAW VISION OCR OUTPUT FOR: {}", path);
            println!("----------------------------------------------------------------");
            println!("{}", final_res.text.trim());
            println!("================================================================\n");
        }

        let entities = self.smart_extractor.extract_entities(&final_res.text);

        serde_json::json!({
            "type": "ocr",
            "text": final_res.text,
            "confidence": final_res.confidence / 100.0,
            "entities": entities
        })
    }

    /// Surgically evaluates OCR text strictly on a line-by-line basis.
    /// Deletes lines that fail structural entropy checks.
    /// Evaluates the surviving block to ensure a sufficient amount of proper text remains.
    fn sanitize_ocr_text(&self, text: &str) -> Option<String> {
        let mut cleaned_lines = Vec::new();
        let mut total_original_lines = 0;

        for line in text.lines() {
            let line_trim = line.trim();
            if line_trim.is_empty() { continue; }
            
            total_original_lines += 1;

            let total_chars = line_trim.chars().filter(|c| !c.is_whitespace()).count();
            if total_chars == 0 { continue; }

            let mut alnum_chars = 0;
            let mut cjk_chars = 0;
            let mut garbage_symbols = 0;
            let mut unique_alnum = std::collections::HashSet::new();

            for c in line_trim.chars() {
                if c.is_whitespace() { continue; }
                
                if c.is_alphanumeric() {
                    alnum_chars += 1;
                    unique_alnum.insert(c.to_lowercase().next().unwrap_or(c));
                    
                    let u = c as u32;
                    // Detect Logographic/CJK ranges (Kana, Hangul, CJK Ideographs)
                    if (u >= 0x2E80 && u <= 0x9FFF) || (u >= 0xAC00 && u <= 0xD7AF) || (u >= 0xF900 && u <= 0xFAFF) || (u >= 0x3040 && u <= 0x30FF) {
                        cjk_chars += 1;
                    }
                } else {
                    // Acceptable punctuation and math symbols
                    if !".,!?'\"()[]{}:;/%$€£+-=*&@#".contains(c) {
                        garbage_symbols += 1;
                    }
                }
            }

            let is_cjk = cjk_chars > 0 && (cjk_chars as f64 / alnum_chars as f64) > 0.3;
            
            // RULE 1: Alphanumeric Density (Is it mostly symbols?)
            if (alnum_chars as f64 / total_chars as f64) < 0.40 {
                continue;
            }

            // RULE 2: Excessive Garbage Symbols (Edge hallucination)
            if (garbage_symbols as f64 / total_chars as f64) > 0.15 {
                continue;
            }

            // RULE 3: Repetitive Spam (e.g. "111 11 111", "aaaaaa")
            if alnum_chars > 10 && unique_alnum.len() <= 3 {
                continue;
            }

            if !is_cjk {
                let mut alnum_words = 0;
                let mut tiny_alnum_words = 0; 
                let mut valid_multi_char_words = 0;
                let mut long_words_without_vowels = 0;
                
                let words: Vec<&str> = line_trim.split_whitespace().collect();
                for word in &words {
                    let clean_word: String = word.chars().filter(|c| c.is_alphabetic()).collect();
                    let len = clean_word.len();
                    
                    if len > 0 {
                        alnum_words += 1;
                        if len <= 2 { 
                            tiny_alnum_words += 1; 
                        } else {
                            let has_vowel = clean_word.to_lowercase().chars().any(|c| "aeiouyáéíóúäöüßàèìòùâêîôû".contains(c));
                            if !has_vowel {
                                long_words_without_vowels += 1;
                            } else {
                                valid_multi_char_words += 1;
                            }
                        }
                    }
                }

                // RULE 4: Long consonant strings with no vowels (Hallucinated textures)
                if long_words_without_vowels > 0 && valid_multi_char_words == 0 {
                    continue;
                }

                // RULE 5: Pure Fragmentation (Shattered noise e.g. "A b c D E f")
                if alnum_words >= 4 && (tiny_alnum_words as f64 / alnum_words as f64) >= 0.65 {
                    continue;
                }
            }

            // Line survived all checks, keep it
            cleaned_lines.push(line_trim.to_string());
        }
        
        // --- POST-SANITIZATION EVALUATION ---

        if cleaned_lines.is_empty() {
            return None;
        }

        let final_text = cleaned_lines.join("\n");
        let total_alnum = final_text.chars().filter(|c| c.is_alphanumeric()).count();
        let total_words = final_text.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphabetic())).count();

        // RULE 6: Minimum Proper Text Threshold
        // If the surviving text is just 1 or 2 random words, or very few characters, 
        // it is a false positive and not "a proper good amount of text".
        if total_alnum < 10 || total_words < 3 {
            return None;
        }
            
        // RULE 7: Document Collapse 
        // If the original document was noisy and we had to delete over 80% of the lines, 
        // the remaining lines are mathematically likely to just be a fluke that passed the gate.
        if total_original_lines > 5 {
            let survival_ratio = cleaned_lines.len() as f64 / total_original_lines as f64;
            if survival_ratio < 0.20 {
                return None; 
            }
        }

        Some(final_text)
    }

    fn extract_qr(&self, path: &str) -> String {
        if let Ok(img) = image::open(path) {
            let (width, height) = img.dimensions();
            
            // Protective bounds check to prevent rqrr from attempting to scan tiny/corrupted images.
            // Minimum QR version 1 size is 21x21 modules + quiet zone.
            if width < 50 || height < 50 {
                return String::new();
            }

            let gray_img = img.to_luma8();
            
            // The `rqrr` crate has an internal bug where specific noise patterns trigger 
            // an `assertion failed: scan >= 1` panic inside `grid.rs`. 
            // Because we run in a Rayon parallel worker thread pool, unhandled panics will 
            // tear down the worker and stall the entire ingestion batch. Catch it safely.
            let result = std::panic::catch_unwind(|| {
                let mut prepared = rqrr::PreparedImage::prepare(gray_img);
                let grids = prepared.detect_grids();
                for grid in grids {
                    if let Ok((_meta, decoded_content)) = grid.decode() {
                        return decoded_content;
                    }
                }
                String::new()
            });

            return result.unwrap_or_default();
        }
        String::new()
    }

    fn calculate_mean_brightness(&self, img: &DynamicImage) -> f64 {
        let luma = img.to_luma8();
        let pixels = luma.as_raw();
        if pixels.is_empty() {
            return 0.0;
        }
        let total: u64 = pixels.iter().map(|&p| p as u64).sum();
        (total as f64) / (pixels.len() as f64) / 255.0
    }

    fn run_tesseract_pass(&self, image_path: &str, psm: &str) -> Option<TesseractResult> {
        let tmp_prefix = format!("/tmp/tess_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        
        let output = self.runtime_adapter.create_system_command("tesseract")
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

        // Pre-clean formatting weirdness from tesseract
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
        
        while text.contains("\n\n\n") {
            text = text.replace("\n\n\n", "\n\n");
        }
        let text = text.trim().to_string();

        // Calculate confidence
        let mut total_conf = 0.0;
        let mut word_count = 0;

        for line in tsv.lines().skip(1) {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() >= 12 {
                if let Ok(conf) = cols[10].parse::<f64>() {
                    let word_text = cols[11].trim();
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

    fn calculate_score(res: &TesseractResult) -> f64 {
        res.confidence 
            + (res.word_count.min(20) as f64) * 0.5 
            + (res.char_count.min(160) as f64) * 0.03 
            - (res.garbage_ratio * 25.0)
    }
}