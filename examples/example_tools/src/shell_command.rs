use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};
use std::time::Duration;

use crate::tool::{Tool, ToolSafety};

pub struct ShellCommandTool {
    pub timeout_secs: u64,
}

impl Default for ShellCommandTool {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[async_trait]
impl Tool for ShellCommandTool {
    fn name(&self) -> &str {
        "shell_command"
    }

    fn description(&self) -> &str {
        "Run a shell command and return its stdout/stderr output"
    }

    fn safety(&self) -> ToolSafety {
        ToolSafety::Exclusive
    }

    fn input_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: JsonValue) -> Result<String> {
        let command = input["command"]
            .as_str()
            .context("missing required field: command")?;

        let timeout = Duration::from_secs(self.timeout_secs);

        let output = tokio::time::timeout(
            timeout,
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("command timed out after {} seconds", self.timeout_secs))?
        .with_context(|| format!("failed to run command: {command}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }
        if combined.is_empty() {
            combined.push_str("(no output)");
        }
        Ok(combined)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn shell_command_echo_returns_output() {
        let tool = ShellCommandTool::default();
        let result = tool
            .execute(json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert!(result.trim() == "hello");
    }

    #[tokio::test]
    async fn shell_command_stderr_is_captured() {
        let tool = ShellCommandTool::default();
        let result = tool
            .execute(json!({"command": "echo error_output >&2"}))
            .await
            .unwrap();
        assert!(result.contains("error_output"));
    }

    #[tokio::test]
    async fn shell_command_missing_command_field_returns_error() {
        let tool = ShellCommandTool::default();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing required field"));
    }

    #[tokio::test]
    async fn shell_command_timeout_returns_error() {
        let tool = ShellCommandTool { timeout_secs: 1 };
        let err = tool
            .execute(json!({"command": "sleep 10"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }
}
