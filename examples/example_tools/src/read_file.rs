use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};

use crate::tool::{Tool, ToolSafety};

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path"
    }

    fn safety(&self) -> ToolSafety {
        ToolSafety::ConcurrentSafe
    }

    fn input_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: JsonValue) -> Result<String> {
        let path = input["path"]
            .as_str()
            .context("missing required field: path")?;
        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read file: {path}"))?;
        Ok(contents)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[tokio::test]
    async fn read_file_reads_tempfile() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "hello from tempfile").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tool = ReadFileTool;
        let result = tool.execute(json!({"path": path})).await.unwrap();
        assert_eq!(result, "hello from tempfile");
    }

    #[tokio::test]
    async fn read_file_missing_path_field_returns_error() {
        let tool = ReadFileTool;
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing required field"));
    }

    #[tokio::test]
    async fn read_file_nonexistent_path_returns_error() {
        let tool = ReadFileTool;
        let err = tool
            .execute(json!({"path": "/nonexistent/path/that/does/not/exist.txt"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("failed to read file"));
    }
}
