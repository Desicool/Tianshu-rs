// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use serde_json::json;
use workflow_engine::case::{Case, ExecutionState};
use workflow_engine::context::WorkflowContext;
use workflow_engine::spawn::{ChildrenResult, SpawnConfig};
use workflow_engine::store::{CaseStore, InMemoryCaseStore, InMemoryStateStore};

fn make_ctx(case_key: &str) -> (WorkflowContext, Arc<InMemoryCaseStore>) {
    let case = Case::new(case_key.into(), "sess_1".into(), "parent_wf".into());
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let ctx = WorkflowContext::new(case, case_store.clone(), state_store);
    (ctx, case_store)
}

#[tokio::test]
async fn spawn_child_creates_case_in_store() {
    let (mut ctx, case_store) = make_ctx("parent_1");

    let config = SpawnConfig {
        workflow_code: "child_wf".into(),
        resource_data: Some(json!({"input": "data"})),
        case_key: Some("child_case_1".into()),
    };

    let handle = ctx.spawn_child(config).await.unwrap();
    assert_eq!(handle.case_key, "child_case_1");
    assert_eq!(handle.workflow_code, "child_wf");

    // The child case should exist in the store
    let child = case_store
        .get_by_key("child_case_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.workflow_code, "child_wf");
    assert_eq!(child.parent_key.as_deref(), Some("parent_1"));
    assert_eq!(child.resource_data, Some(json!({"input": "data"})));
    assert_eq!(child.execution_state, ExecutionState::Running);
}

#[tokio::test]
async fn spawn_children_creates_multiple() {
    let (mut ctx, case_store) = make_ctx("parent_2");

    let configs = vec![
        SpawnConfig {
            workflow_code: "wf_a".into(),
            resource_data: None,
            case_key: Some("child_a".into()),
        },
        SpawnConfig {
            workflow_code: "wf_b".into(),
            resource_data: None,
            case_key: Some("child_b".into()),
        },
        SpawnConfig {
            workflow_code: "wf_c".into(),
            resource_data: None,
            case_key: Some("child_c".into()),
        },
    ];

    let handles = ctx.spawn_children(configs).await.unwrap();
    assert_eq!(handles.len(), 3);

    // All three should exist in the store
    for key in &["child_a", "child_b", "child_c"] {
        let child = case_store.get_by_key(key).await.unwrap();
        assert!(child.is_some(), "child {} should exist", key);
    }

    // Parent should track all child keys
    assert_eq!(ctx.case.child_keys.len(), 3);
}

#[tokio::test]
async fn child_status_running() {
    let (mut ctx, _case_store) = make_ctx("parent_3");

    let config = SpawnConfig {
        workflow_code: "sub_wf".into(),
        resource_data: None,
        case_key: Some("running_child".into()),
    };

    let handle = ctx.spawn_child(config).await.unwrap();
    let status = ctx.child_status(&handle).await.unwrap();
    assert!(
        matches!(status, workflow_engine::spawn::ChildStatus::Running),
        "newly spawned child should be Running"
    );
}

#[tokio::test]
async fn child_status_finished() {
    let (mut ctx, case_store) = make_ctx("parent_4");

    let config = SpawnConfig {
        workflow_code: "sub_wf".into(),
        resource_data: Some(json!({"output": "result"})),
        case_key: Some("finish_child".into()),
    };

    let handle = ctx.spawn_child(config).await.unwrap();

    // Manually finish the child
    let mut child = case_store
        .get_by_key("finish_child")
        .await
        .unwrap()
        .unwrap();
    child.mc_finish("SUCCESS".into(), "all done".into());
    child.resource_data = Some(json!({"output": "final"}));
    case_store.upsert(&child).await.unwrap();

    let status = ctx.child_status(&handle).await.unwrap();
    match status {
        workflow_engine::spawn::ChildStatus::Finished {
            finished_type,
            finished_description,
            resource_data,
        } => {
            assert_eq!(finished_type, "SUCCESS");
            assert_eq!(finished_description, "all done");
            assert_eq!(resource_data, Some(json!({"output": "final"})));
        }
        other => panic!("expected Finished, got {:?}", other),
    }
}

#[tokio::test]
async fn await_children_pending_when_any_running() {
    let (mut ctx, _case_store) = make_ctx("parent_5");

    let configs = vec![
        SpawnConfig {
            workflow_code: "wf".into(),
            resource_data: None,
            case_key: Some("ac_child_1".into()),
        },
        SpawnConfig {
            workflow_code: "wf".into(),
            resource_data: None,
            case_key: Some("ac_child_2".into()),
        },
    ];

    let handles = ctx.spawn_children(configs).await.unwrap();

    // Both children are still Running
    let result = ctx.await_children(&handles).await.unwrap();
    match result {
        ChildrenResult::Pending(n) => assert_eq!(n, 2),
        ChildrenResult::AllDone(_) => panic!("expected Pending"),
    }
}

#[tokio::test]
async fn await_children_all_done_when_finished() {
    let (mut ctx, case_store) = make_ctx("parent_6");

    let configs = vec![
        SpawnConfig {
            workflow_code: "wf".into(),
            resource_data: None,
            case_key: Some("ad_child_1".into()),
        },
        SpawnConfig {
            workflow_code: "wf".into(),
            resource_data: None,
            case_key: Some("ad_child_2".into()),
        },
    ];

    let handles = ctx.spawn_children(configs).await.unwrap();

    // Finish both children
    for key in &["ad_child_1", "ad_child_2"] {
        let mut child = case_store.get_by_key(key).await.unwrap().unwrap();
        child.mc_finish("OK".into(), "done".into());
        case_store.upsert(&child).await.unwrap();
    }

    let result = ctx.await_children(&handles).await.unwrap();
    match result {
        ChildrenResult::AllDone(items) => {
            assert_eq!(items.len(), 2);
            for (_, status) in &items {
                assert!(matches!(
                    status,
                    workflow_engine::spawn::ChildStatus::Finished { .. }
                ));
            }
        }
        ChildrenResult::Pending(_) => panic!("expected AllDone"),
    }
}