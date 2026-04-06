use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};

use crate::tool::{Tool, ToolSafety};

pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories at the given path"
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
                    "description": "Directory path to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: JsonValue) -> Result<String> {
        let path = input["path"]
            .as_str()
            .context("missing required field: path")?;

        let mut rd = tokio::fs::read_dir(path)
            .await
            .with_context(|| format!("failed to list directory: {path}"))?;

        let mut lines = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .context("error reading directory entry")?
        {
            let file_type = entry.file_type().await.context("error getting file type")?;
            let kind = if file_type.is_dir() {
                "dir"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "file"
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            lines.push(format!("{name} ({kind})"));
        }

        if lines.is_empty() {
            Ok("(empty directory)".to_string())
        } else {
            lines.sort();
            Ok(lines.join("\n"))
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn list_directory_lists_contents() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir_path = tmp_dir.path().to_str().unwrap().to_string();

        // Create some entries.
        tokio::fs::write(tmp_dir.path().join("file_a.txt"), "a")
            .await
            .unwrap();
        tokio::fs::write(tmp_dir.path().join("file_b.txt"), "b")
            .await
            .unwrap();
        tokio::fs::create_dir(tmp_dir.path().join("subdir"))
            .await
            .unwrap();

        let tool = ListDirectoryTool;
        let result = tool.execute(json!({"path": dir_path})).await.unwrap();

        assert!(result.contains("file_a.txt (file)"));
        assert!(result.contains("file_b.txt (file)"));
        assert!(result.contains("subdir (dir)"));
    }

    #[tokio::test]
    async fn list_directory_empty_dir_returns_message() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir_path = tmp_dir.path().to_str().unwrap().to_string();

        let tool = ListDirectoryTool;
        let result = tool.execute(json!({"path": dir_path})).await.unwrap();
        assert_eq!(result, "(empty directory)");
    }

    #[tokio::test]
    async fn list_directory_missing_path_returns_error() {
        let tool = ListDirectoryTool;
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing required field"));
    }

    #[tokio::test]
    async fn list_directory_nonexistent_returns_error() {
        let tool = ListDirectoryTool;
        let err = tool
            .execute(json!({"path": "/nonexistent/dir/that/does/not/exist"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("failed to list directory"));
    }
}
