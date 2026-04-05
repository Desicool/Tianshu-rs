/// Tests for OpenAI-compatible LlmProvider adapter.
///
/// These tests verify construction, configuration, and the LlmProvider
/// trait contract WITHOUT making real HTTP calls.
use std::sync::Arc;
use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmTool, ToolCall, ToolResult};
use workflow_engine_llm_openai::OpenAiProvider;

#[test]
fn provider_default_base_url() {
    let p = OpenAiProvider::new("sk-test-key", "gpt-4o");
    assert_eq!(p.base_url(), "https://api.openai.com/v1");
}

#[test]
fn provider_custom_base_url() {
    let p = OpenAiProvider::builder("sk-test", "my-model")
        .base_url("http://localhost:11434/v1")
        .build();
    assert_eq!(p.base_url(), "http://localhost:11434/v1");
}

#[test]
fn provider_strips_trailing_slash_from_base_url() {
    let p = OpenAiProvider::builder("key", "model")
        .base_url("https://api.openai.com/v1/")
        .build();
    assert_eq!(p.base_url(), "https://api.openai.com/v1");
}

#[test]
fn provider_model_name_accessible() {
    let p = OpenAiProvider::new("k", "claude-3-haiku");
    assert_eq!(p.model(), "claude-3-haiku");
}

#[test]
fn provider_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OpenAiProvider>();
}

#[test]
fn provider_implements_llm_provider_trait() {
    // Verify it can be used as Arc<dyn LlmProvider>
    let p: Arc<dyn LlmProvider> = Arc::new(OpenAiProvider::new("key", "model"));
    let _ = p;
}

#[test]
fn llm_request_construction() {
    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: Some("You are helpful".into()),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "Hello".into(),
            tool_calls: None,
            tool_call_id: None,
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),
        tools: None,
    };
    assert_eq!(req.model, "gpt-4o");
    assert_eq!(req.messages.len(), 1);
}

// ── Tool serialization tests ──────────────────────────────────────────────────

#[test]
fn wire_request_includes_tools_when_some() {
    use serde_json::json;

    let tool = LlmTool {
        name: "get_weather".into(),
        description: "Returns weather data".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
    };
    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: None,
        messages: vec![LlmMessage::user("What is the weather?")],
        temperature: None,
        max_tokens: None,
        tools: Some(vec![tool]),
    };

    // Verify that tools are present in the wire request JSON produced by the provider
    let wire_json = workflow_engine_llm_openai::build_wire_request_json(&req);
    assert!(wire_json["tools"].is_array());
    assert_eq!(wire_json["tools"][0]["type"], "function");
    assert_eq!(wire_json["tools"][0]["function"]["name"], "get_weather");
    assert_eq!(
        wire_json["tools"][0]["function"]["description"],
        "Returns weather data"
    );
    assert_eq!(
        wire_json["tools"][0]["function"]["parameters"]["type"],
        "object"
    );
}

#[test]
fn wire_request_omits_tools_when_none() {
    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: None,
        messages: vec![LlmMessage::user("hello")],
        temperature: None,
        max_tokens: None,
        tools: None,
    };
    let wire_json = workflow_engine_llm_openai::build_wire_request_json(&req);
    assert!(wire_json.get("tools").is_none());
}

#[test]
fn tool_results_serialize_to_role_tool_messages() {
    use serde_json::json;

    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: None,
        messages: vec![
            LlmMessage::user("What is the weather in London?"),
            LlmMessage::assistant_with_tool_calls(
                "",
                vec![ToolCall {
                    id: "call_abc".into(),
                    name: "get_weather".into(),
                    input: json!({"city": "London"}),
                }],
            ),
            LlmMessage::tool_results(vec![ToolResult {
                tool_use_id: "call_abc".into(),
                content: "15°C, cloudy".into(),
                is_error: false,
            }]),
        ],
        temperature: None,
        max_tokens: None,
        tools: None,
    };

    let wire_json = workflow_engine_llm_openai::build_wire_request_json(&req);
    let messages = wire_json["messages"].as_array().unwrap();

    // Should be: user, assistant (with tool_calls), tool (result)
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    // The third message should be role: "tool"
    assert_eq!(messages[2]["role"], "tool");
    assert_eq!(messages[2]["tool_call_id"], "call_abc");
    assert_eq!(messages[2]["content"], "15°C, cloudy");
}

#[test]
fn multiple_tool_results_expand_to_multiple_role_tool_messages() {
    use serde_json::json;

    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: None,
        messages: vec![
            LlmMessage::user("compare weather"),
            LlmMessage::assistant_with_tool_calls(
                "",
                vec![
                    ToolCall {
                        id: "call_1".into(),
                        name: "get_weather".into(),
                        input: json!({"city": "London"}),
                    },
                    ToolCall {
                        id: "call_2".into(),
                        name: "get_weather".into(),
                        input: json!({"city": "Paris"}),
                    },
                ],
            ),
            LlmMessage::tool_results(vec![
                ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "15°C".into(),
                    is_error: false,
                },
                ToolResult {
                    tool_use_id: "call_2".into(),
                    content: "18°C".into(),
                    is_error: false,
                },
            ]),
        ],
        temperature: None,
        max_tokens: None,
        tools: None,
    };

    let wire_json = workflow_engine_llm_openai::build_wire_request_json(&req);
    let messages = wire_json["messages"].as_array().unwrap();

    // user + assistant + 2x tool = 4
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[2]["role"], "tool");
    assert_eq!(messages[2]["tool_call_id"], "call_1");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_2");
}

#[test]
fn assistant_tool_calls_serialized_in_wire_message() {
    use serde_json::json;

    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: None,
        messages: vec![LlmMessage::assistant_with_tool_calls(
            "",
            vec![ToolCall {
                id: "call_xyz".into(),
                name: "search".into(),
                input: json!({"query": "rust programming"}),
            }],
        )],
        temperature: None,
        max_tokens: None,
        tools: None,
    };

    let wire_json = workflow_engine_llm_openai::build_wire_request_json(&req);
    let messages = wire_json["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "assistant");
    let tool_calls = &messages[0]["tool_calls"];
    assert!(tool_calls.is_array());
    assert_eq!(tool_calls[0]["id"], "call_xyz");
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["function"]["name"], "search");
    // arguments should be a JSON string
    let args_str = tool_calls[0]["function"]["arguments"].as_str().unwrap();
    let args: serde_json::Value = serde_json::from_str(args_str).unwrap();
    assert_eq!(args["query"], "rust programming");
}
