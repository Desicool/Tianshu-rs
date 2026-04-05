use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};

use crate::tool::{Tool, ToolSafety};

pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files at the given path (like grep -rn)"
    }

    fn safety(&self) -> ToolSafety {
        ToolSafety::ConcurrentSafe
    }

    fn input_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in"
                }
            },
            "required": ["pattern", "path"]
        })
    }

    async fn execute(&self, input: JsonValue) -> Result<String> {
        let pattern = input["pattern"]
            .as_str()
            .context("missing required field: pattern")?;
        let path = input["path"]
            .as_str()
            .context("missing required field: path")?;

        let output = tokio::process::Command::new("grep")
            .args(["-rn", "--include=*", pattern, path])
            .output()
            .await
            .context("failed to run grep")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            Ok("No matches found".to_string())
        } else {
            Ok(stdout.into_owned())
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[tokio::test]
    async fn search_files_finds_pattern_in_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "hello world\nfoo bar\nbaz hello").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tool = SearchFilesTool;
        let result = tool
            .execute(json!({"pattern": "hello", "path": path}))
            .await
            .unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn search_files_returns_no_matches_when_pattern_absent() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "nothing here matching xyz").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tool = SearchFilesTool;
        let result = tool
            .execute(json!({"pattern": "NOTFOUND_UNIQUE_STRING_12345", "path": path}))
            .await
            .unwrap();
        assert_eq!(result, "No matches found");
    }

    #[tokio::test]
    async fn search_files_missing_pattern_returns_error() {
        let tool = SearchFilesTool;
        let err = tool
            .execute(json!({"path": "/tmp"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing required field"));
    }
}
