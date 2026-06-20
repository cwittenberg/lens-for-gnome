use std::path::Path;
use std::io::Read;
use super::FileExtractor;

pub struct LegacyDocExtractor;

impl FileExtractor for LegacyDocExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        extension == "doc"
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).map_err(|e| e.to_string())?;

        let mut content = String::new();
        let mut current_string = String::new();

        for &byte in &buffer {
            if byte.is_ascii_graphic() || byte == b' ' {
                current_string.push(byte as char);
            } else {
                if current_string.len() > 4 {
                    content.push_str(&current_string);
                    content.push(' ');
                }
                current_string.clear();
            }
        }
        Ok(content)
    }
}