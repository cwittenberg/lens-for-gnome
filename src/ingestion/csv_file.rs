use std::path::Path;
use super::FileExtractor;

pub struct CsvExtractor;

impl FileExtractor for CsvExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        extension == "csv"
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let mut rdr = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
        let mut content = String::new();
        for result in rdr.records() {
            if let Ok(record) = result {
                let row: Vec<&str> = record.iter().collect();
                content.push_str(&row.join(" "));
                content.push('\n');
            }
        }
        Ok(content)
    }
}