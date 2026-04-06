// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;
use tianshu::spawn::{ChildHandle, ChildStatus, ChildrenResult, SpawnConfig};
use tianshu::Case;

#[test]
fn spawn_config_construction() {
    let config = SpawnConfig {
        workflow_code: "child_wf".into(),
        resource_data: Some(json!({"key": "value"})),
        case_key: Some("custom_key".into()),
    };
    assert_eq!(config.workflow_code, "child_wf");
    assert_eq!(config.case_key.as_deref(), Some("custom_key"));
    assert!(config.resource_data.is_some());
}

#[test]
fn child_handle_serde_roundtrip() {
    let handle = ChildHandle {
        case_key: "child_1".into(),
        workflow_code: "wf_sub".into(),
    };
    let serialized = serde_json::to_string(&handle).unwrap();
    let deserialized: ChildHandle = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.case_key, "child_1");
    assert_eq!(deserialized.workflow_code, "wf_sub");
}

#[test]
fn child_status_variants() {
    let running = ChildStatus::Running;
    assert!(matches!(running, ChildStatus::Running));

    let waiting = ChildStatus::Waiting;
    assert!(matches!(waiting, ChildStatus::Waiting));

    let finished = ChildStatus::Finished {
        finished_type: "SUCCESS".into(),
        finished_description: "done".into(),
        resource_data: Some(json!({"result": 42})),
    };
    assert!(matches!(finished, ChildStatus::Finished { .. }));

    let failed = ChildStatus::Failed {
        error: "boom".into(),
    };
    assert!(matches!(failed, ChildStatus::Failed { .. }));
}

#[test]
fn children_result_all_done() {
    let handle = ChildHandle {
        case_key: "c1".into(),
        workflow_code: "wf".into(),
    };
    let status = ChildStatus::Finished {
        finished_type: "OK".into(),
        finished_description: "complete".into(),
        resource_data: None,
    };
    let result = ChildrenResult::AllDone(vec![(handle, status)]);
    match result {
        ChildrenResult::AllDone(items) => assert_eq!(items.len(), 1),
        ChildrenResult::Pending(_) => panic!("expected AllDone"),
    }
}

#[test]
fn children_result_pending() {
    let result = ChildrenResult::Pending(3);
    match result {
        ChildrenResult::Pending(n) => assert_eq!(n, 3),
        ChildrenResult::AllDone(_) => panic!("expected Pending"),
    }
}

#[test]
fn case_has_child_keys_field() {
    let case = Case::new("k".into(), "s".into(), "wf".into());
    // child_keys should be an empty Vec by default
    assert!(case.child_keys.is_empty());
}