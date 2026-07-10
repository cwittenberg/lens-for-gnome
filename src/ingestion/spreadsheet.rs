// src/ingestion/spreadsheet.rs

use std::path::Path;
use calamine::{Reader, open_workbook_auto, DataType};
use super::FileExtractor;

pub struct SpreadsheetExtractor;

impl FileExtractor for SpreadsheetExtractor {
    fn can_handle(&self, extension: &str) -> bool {
        matches!(extension, "xlsx" | "xls" | "ods")
    }

    fn extract(&self, path: &Path) -> Result<String, String> {
        let mut workbook = open_workbook_auto(path).map_err(|e| e.to_string())?;
        let mut content = String::new();

        if let Some(sheet_names) = Some(workbook.sheet_names().to_owned()) {
            for sheet_name in sheet_names {
                if let Some(Ok(range)) = workbook.worksheet_range(&sheet_name) {
                    for row in range.rows() {
                        for cell in row {
                            match cell {
                                DataType::String(s) => { content.push_str(&s); content.push(' '); },
                                DataType::Float(f) => { content.push_str(&f.to_string()); content.push(' '); },
                                DataType::Int(i) => { content.push_str(&i.to_string()); content.push(' '); },
                                // Type inference is now correct: 'b' is '&bool', so '*b' evaluates safely
                                DataType::Bool(b) => { content.push_str(if *b { "true " } else { "false " }); },
                                _ => {}
                            }
                        }
                        content.push('\n');
                    }
                }
            }
        }
        
        Ok(content)
    }
}