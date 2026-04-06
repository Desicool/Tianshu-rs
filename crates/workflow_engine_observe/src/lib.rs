// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

//! Observer implementations for the Tianshu workflow engine.
//!
//! This crate provides ready-made `Observer` implementations and dataset
//! export utilities to complement `workflow_engine`'s observer trait.
//!
//! # Observers
//!
//! | Type | Description |
//! |------|-------------|
//! | [`InMemoryObserver`] | Accumulates records in memory. For testing and programmatic access. |
//! | [`JsonlObserver`] | Appends one JSON line per event to a file. For RLHF dataset building. |
//! | [`CompositeObserver`] | Fans out events to multiple child observers simultaneously. |
//!
//! # Dataset export
//!
//! The [`dataset`] module provides functions to extract RLHF-ready
//! input/output pairs from collected records:
//!
//! ```rust,ignore
//! use tianshu_observe::{InMemoryObserver, dataset::step_dataset};
//!
//! let obs = Arc::new(InMemoryObserver::new());
//! scheduler.set_observer(obs.clone());
//!
//! // ... run workflows ...
//!
//! // Get only original (non-cached) step executions for fine-tuning
//! let entries = step_dataset(&obs.step_records());
//! for entry in &entries {
//!     println!("{} -> {}", entry.input, entry.output);
//! }
//! ```

pub mod composite;
pub mod dataset;
pub mod jsonl;
pub mod memory;

pub use composite::CompositeObserver;
pub use jsonl::JsonlObserver;
pub use memory::InMemoryObserver;