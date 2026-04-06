// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};
use std::sync::Arc;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::mpsc, sync::Notify};
use tianshu::observe::{LlmCallRecord, Observer, StepRecord, WorkflowRecord};

enum Event {
    Record(String),
    Flush(Arc<Notify>),
}

/// An observer that writes every event as a JSON line to a file.
///
/// Each event is tagged with a `"type"` field:
/// - `"step"` — from `on_step()`
/// - `"workflow_complete"` — from `on_workflow_complete()`
/// - `"llm_call"` — from `on_llm_call()`
///
/// `on_step/on_workflow_complete/on_llm_call` are non-blocking (send to channel).
/// Call `flush()` to drain the buffer before reading the file.
///
/// # Example
///
/// ```rust,ignore
/// let obs = JsonlObserver::new("/tmp/traces.jsonl").await?;
/// scheduler.set_observer(Arc::new(obs));
/// // ... run workflows ...
/// // Flush is called automatically by WorkflowContext::finish(),
/// // but you can also call it manually.
/// ```
pub struct JsonlObserver {
    sender: mpsc::UnboundedSender<Event>,
}

impl JsonlObserver {
    /// Create a new JSONL observer writing to `path`.
    /// The file is created or appended to.
    pub async fn new(path: &str) -> Result<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Event>();
        let path = path.to_string();

        tokio::spawn(async move {
            let mut file = match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("JsonlObserver: failed to open {}: {}", path, e);
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Event::Record(line) => {
                        let mut bytes = line.into_bytes();
                        bytes.push(b'\n');
                        if let Err(e) = file.write_all(&bytes).await {
                            tracing::error!("JsonlObserver: write error: {}", e);
                        }
                    }
                    Event::Flush(notify) => {
                        if let Err(e) = file.flush().await {
                            tracing::error!("JsonlObserver: flush error: {}", e);
                        }
                        notify.notify_one();
                    }
                }
            }
        });

        Ok(Self { sender })
    }

    fn send_record(&self, value: JsonValue) {
        if let Ok(line) = serde_json::to_string(&value) {
            let _ = self.sender.send(Event::Record(line));
        }
    }
}

#[async_trait]
impl Observer for JsonlObserver {
    async fn on_step(&self, record: &StepRecord) {
        let mut value = serde_json::to_value(record).unwrap_or(json!({}));
        if let Some(obj) = value.as_object_mut() {
            obj.insert("type".to_string(), json!("step"));
        }
        self.send_record(value);
    }

    async fn on_workflow_complete(&self, record: &WorkflowRecord) {
        let mut value = serde_json::to_value(record).unwrap_or(json!({}));
        if let Some(obj) = value.as_object_mut() {
            obj.insert("type".to_string(), json!("workflow_complete"));
        }
        self.send_record(value);
    }

    async fn on_llm_call(&self, record: &LlmCallRecord) {
        let mut value = serde_json::to_value(record).unwrap_or(json!({}));
        if let Some(obj) = value.as_object_mut() {
            obj.insert("type".to_string(), json!("llm_call"));
        }
        self.send_record(value);
    }

    /// Flush buffered writes. Waits until all previously sent events are written.
    async fn flush(&self) {
        let notify = Arc::new(Notify::new());
        let _ = self.sender.send(Event::Flush(Arc::clone(&notify)));
        notify.notified().await;
    }
}