// src/ingestion/eml.rs
use std::path::Path;
use mailparse::*;

use super::FileExtractor;

pub struct EmlExtractor;

impl FileExtractor for EmlExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "eml")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        println!("[Indexer] [EML] Opening local mailbox file for semantic extraction: {}", file_name);

        let raw_mail = std::fs::read(path).map_err(|e| {
            let err_msg = format!("File read failure: {}", e);
            eprintln!("[Indexer Error] [EML] {}", err_msg);
            err_msg
        })?;

        let parsed_mail = parse_mail(&raw_mail).map_err(|e| {
            let err_msg = format!("MIME structural layout parsing error: {}", e);
            eprintln!("[Indexer Error] [EML] {}", err_msg);
            err_msg
        })?;

        let mut content = String::new();
        let mut log_subject = String::from("Unknown Subject");

        // 1. Extract Rich Headers for LLM Context
        content.push_str("--- EMAIL METADATA ---\n");
        for header in parsed_mail.get_headers() {
            let key = header.get_key();
            let key_lower = key.to_lowercase();

            if matches!(key_lower.as_str(), "subject" | "from" | "to" | "cc" | "date" | "message-id") {
                let val = header.get_value().replace("\r\n", " ");
                content.push_str(&format!("{}: {}\n", key, val));
                
                if key_lower == "subject" {
                    log_subject = val.clone();
                }
            }
        }
        content.push_str("----------------------\n\n");

        println!("[Indexer] [EML] Parsing content layer for email: \"{}\"", log_subject);

        // 2. Extract Body (Prefer Plaintext, fallback to HTML)
        let body = Self::extract_best_body(&parsed_mail);
        
        let clean_body = if body.contains("<html") || body.contains("<body") {
            Self::strip_html(&body)
        } else {
            body
        };

        content.push_str(clean_body.trim());

        if content.trim().is_empty() {
            let err_msg = "Extracted email is empty".to_string();
            eprintln!("[Indexer Warning] [EML] {} for {}", err_msg, file_name);
            Err(err_msg)
        } else {
            println!(
                "[Indexer] [EML] Successfully extracted {} characters of text/metadata context from {}",
                content.len(),
                file_name
            );
            Ok(content)
        }
    }
}

impl EmlExtractor {
    fn extract_best_body(parsed: &ParsedMail) -> String {
        let mut best_body = String::new();

        if parsed.subparts.is_empty() {
            if parsed.ctype.mimetype == "text/plain" || parsed.ctype.mimetype == "text/html" {
                return parsed.get_body().unwrap_or_default();
            }
        } else {
            for subpart in &parsed.subparts {
                if subpart.ctype.mimetype == "text/plain" {
                    return subpart.get_body().unwrap_or_default();
                } else if subpart.ctype.mimetype == "text/html" {
                    best_body = subpart.get_body().unwrap_or_default();
                } else if subpart.ctype.mimetype.starts_with("multipart/") {
                    let sub_body = Self::extract_best_body(subpart);
                    if !sub_body.is_empty() {
                        return sub_body;
                    }
                }
            }
        }

        best_body
    }

    fn strip_html(html: &str) -> String {
        let mut result = String::with_capacity(html.len());
        let mut in_tag = false;
        let mut in_style_or_script = false;
        let mut chars = html.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '<' {
                in_tag = true;
                let mut tag_name = String::new();
                let mut peek_chars = chars.clone();
                while let Some(&p) = peek_chars.peek() {
                    if p == '>' || p == ' ' { break; }
                    tag_name.push(p);
                    peek_chars.next();
                }
                let tag_lower = tag_name.to_lowercase();
                if tag_lower == "script" || tag_lower == "style" {
                    in_style_or_script = true;
                } else if tag_lower == "/script" || tag_lower == "/style" {
                    in_style_or_script = false;
                }
                continue;
            }

            if c == '>' {
                in_tag = false;
                result.push(' ');
                continue;
            }

            if !in_tag && !in_style_or_script {
                result.push(c);
            }
        }

        result.split_whitespace().collect::<Vec<&str>>().join(" ")
    }
}