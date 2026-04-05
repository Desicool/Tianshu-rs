use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};

use crate::tool::{Tool, ToolSafety};

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path (creates or overwrites)"
    }

    fn safety(&self) -> ToolSafety {
        ToolSafety::Exclusive
    }

    fn input_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, input: JsonValue) -> Result<String> {
        let path = input["path"]
            .as_str()
            .context("missing required field: path")?;
        let content = input["content"]
            .as_str()
            .context("missing required field: content")?;
        // Create parent directories if they don't already exist.
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("failed to create parent directories for: {path}"))?;
            }
        }
        tokio::fs::write(path, content)
            .await
            .with_context(|| format!("failed to write file: {path}"))?;
        Ok(format!("Written {} bytes to {path}", content.len()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn write_file_creates_file_with_content() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("output.txt");
        let path_str = path.to_str().unwrap().to_string();

        let tool = WriteFileTool;
        let result = tool
            .execute(json!({"path": path_str, "content": "test content"}))
            .await
            .unwrap();

        assert!(result.contains("12")); // "test content" is 12 bytes
        assert!(result.contains(&path_str));

        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(written, "test content");
    }

    #[tokio::test]
    async fn write_file_overwrites_existing_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("overwrite.txt");
        let path_str = path.to_str().unwrap().to_string();

        let tool = WriteFileTool;
        tool.execute(json!({"path": path_str, "content": "original"}))
            .await
            .unwrap();
        tool.execute(json!({"path": path_str, "content": "replaced"}))
            .await
            .unwrap();

        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(written, "replaced");
    }

    #[tokio::test]
    async fn write_file_creates_parent_directories() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("nested/deep/file.txt");
        let path_str = path.to_str().unwrap().to_string();

        let tool = WriteFileTool;
        let result = tool
            .execute(json!({"path": path_str, "content": "nested content"}))
            .await
            .unwrap();

        assert!(result.contains(&path_str));
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(written, "nested content");
    }

    #[tokio::test]
    async fn write_file_missing_path_returns_error() {
        let tool = WriteFileTool;
        let err = tool.execute(json!({"content": "hello"})).await.unwrap_err();
        assert!(err.to_string().contains("missing required field"));
    }
}
