// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use tianshu::{
    case::{Case, ExecutionState},
    store::{InMemoryCaseStore, InMemoryStateStore},
    WorkflowContext,
};

fn make_case(case_key: &str, workflow_code: &str) -> Case {
    Case::new(case_key.into(), "sess_integ".into(), workflow_code.into())
}

fn make_ctx(case: Case) -> WorkflowContext {
    WorkflowContext::new(
        case,
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

#[test]
fn test_workflow_context_api() {
    let ctx = make_ctx(make_case("test_case_456", "test_workflow"));
    assert_eq!(ctx.case.execution_state, ExecutionState::Running);
    assert!(ctx.case.finished_type.is_none());
}

#[test]
fn test_multiple_contexts_independent() {
    let ctx1 = make_ctx(make_case("case_1", "workflow_1"));
    let ctx2 = make_ctx(make_case("case_2", "workflow_2"));
    assert_ne!(ctx1.case.case_key, ctx2.case.case_key);
}

#[test]
fn test_context_case_key_format() {
    let ctx = make_ctx(make_case("session_123_code_5", "code_5"));
    assert_eq!(ctx.case.case_key, "session_123_code_5");
    assert_eq!(ctx.case.workflow_code, "code_5");
}