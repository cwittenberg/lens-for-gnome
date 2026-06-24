// src/ingestion/video.rs
use std::path::Path;
use std::fs;
use std::process::Command;
use super::FileExtractor;

pub struct VideoExtractor;

impl VideoExtractor {
    pub fn new() -> Self {
        Self
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

fn generate_video_thumbnail(path: &Path) {
    if let Some(home) = std::env::var_os("HOME") {
        let large_dir = Path::new(&home).join(".cache/thumbnails/large");
        let uri = generate_freedesktop_uri(path);
        let hash = format!("{:x}", md5::compute(uri.as_bytes()));
        let thumb_path = large_dir.join(format!("{}.png", hash));
        
        if thumb_path.exists() {
            return;
        }

        let _ = fs::create_dir_all(&large_dir);
        let temp_path = large_dir.join(format!("{}.tmp.png", hash));
        let path_str = path.to_string_lossy().to_string();
        
        // Spawn FFMPEG to seek to 0.5s and extract a 256x256 thumbnail frame
        let status = Command::new("ffmpeg")
            .args(&[
                "-y", 
                "-i", &path_str, 
                "-ss", "00:00:00.500", 
                "-vframes", "1", 
                "-vf", "thumbnail,scale=256:256:force_original_aspect_ratio=decrease", 
                temp_path.to_str().unwrap()
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if let Ok(st) = status {
            if st.success() {
                let _ = fs::rename(&temp_path, &thumb_path);
            } else {
                let _ = fs::remove_file(&temp_path);
            }
        }
    }
}

impl FileExtractor for VideoExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        // Fire-and-forget thumbnail frame extraction into the OS Cache
        generate_video_thumbnail(path);

        let path_str = path.to_string_lossy().to_string();
        let mut extracted_content = String::new();

        if let Ok(probe_output) = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(&path_str)
            .output()
        {
            if probe_output.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&probe_output.stdout) {
                    extracted_content.push_str("--- VIDEO METADATA ---\n");
                    
                    if let Some(format) = json.get("format") {
                        if let Some(duration) = format.get("duration").and_then(|d| d.as_str()) {
                            extracted_content.push_str(&format!("Duration: {} seconds\n", duration));
                        }
                        if let Some(tags) = format.get("tags").and_then(|t| t.as_object()) {
                            for (k, v) in tags {
                                if let Some(val_str) = v.as_str() {
                                    extracted_content.push_str(&format!("Tag {}: {}\n", k, val_str));
                                }
                            }
                        }
                    }

                    if let Some(streams) = json.get("streams").and_then(|s| s.as_array()) {
                        let mut v_codecs = Vec::new();
                        let mut a_codecs = Vec::new();
                        let mut resolutions = Vec::new();

                        for stream in streams {
                            if let Some(codec_type) = stream.get("codec_type").and_then(|c| c.as_str()) {
                                if let Some(codec_name) = stream.get("codec_name").and_then(|c| c.as_str()) {
                                    match codec_type {
                                        "video" => {
                                            v_codecs.push(codec_name.to_string());
                                            let width = stream.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
                                            let height = stream.get("height").and_then(|h| h.as_i64()).unwrap_or(0);
                                            if width > 0 && height > 0 {
                                                resolutions.push(format!("{}x{}", width, height));
                                            }
                                        },
                                        "audio" => a_codecs.push(codec_name.to_string()),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        
                        if !v_codecs.is_empty() { extracted_content.push_str(&format!("Video Codecs: {}\n", v_codecs.join(", "))); }
                        if !a_codecs.is_empty() { extracted_content.push_str(&format!("Audio Codecs: {}\n", a_codecs.join(", "))); }
                        if !resolutions.is_empty() { extracted_content.push_str(&format!("Resolutions: {}\n", resolutions.join(", "))); }
                    }
                    extracted_content.push_str("----------------------\n\n");
                }
            }
        }

        if let Ok(ffmpeg_output) = Command::new("ffmpeg")
            .arg("-v")
            .arg("quiet")
            .arg("-i")
            .arg(&path_str)
            .arg("-map")
            .arg("0:s:0")
            .arg("-c:s")
            .arg("text")
            .arg("-f")
            .arg("srt")
            .arg("-")
            .output()
        {
            if ffmpeg_output.status.success() {
                let subtitles = String::from_utf8_lossy(&ffmpeg_output.stdout);
                if !subtitles.trim().is_empty() {
                    extracted_content.push_str("--- EMBEDDED DIALOGUE / SUBTITLES ---\n");
                    
                    for line in subtitles.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.contains("-->") && !trimmed.chars().all(|c| c.is_ascii_digit()) {
                            extracted_content.push_str(trimmed);
                            extracted_content.push(' ');
                        }
                    }
                    extracted_content.push_str("\n-------------------------------------\n");
                }
            }
        }

        if extracted_content.trim().is_empty() {
            Err("Failed to extract any metadata or subtitles from the video container.".to_string())
        } else {
            Ok(extracted_content)
        }
    }
}