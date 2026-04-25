use serde_json::{json, Value};
use std::path::PathBuf;
use themion_core::tools::{call_tool, ToolCtx};
use themion_core::{DbHandle, WorkflowState};
use uuid::Uuid;

fn test_ctx(db: std::sync::Arc<DbHandle>) -> ToolCtx {
    ToolCtx {
        db,
        session_id: Uuid::new_v4(),
        project_dir: PathBuf::from("."),
        workflow_state: Some(WorkflowState::default()),
        turn_seq: None,
        system_inspection: None,
    }
}

async fn call_json(ctx: &ToolCtx, name: &str, args: Value) -> Value {
    let result = call_tool(name, &args.to_string(), ctx).await;
    assert!(!result.starts_with("Error:"), "tool failed: {result}");
    serde_json::from_str(&result).expect("tool returned json")
}

#[tokio::test]
async fn knowledge_base_node_is_retrievable_by_hashtag_and_keyword() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    let node = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "node_type": "decision",
            "title": "Use SQLite knowledge base",
            "content": "Store durable facts as first-class graph nodes.",
            "hashtags": ["#Rust", "knowledge-base"]
        }),
    )
    .await;
    assert_eq!(node["hashtags"], json!(["#knowledge_base", "#rust"]));

    let by_tag = call_json(
        &ctx,
        "memory_search",
        json!({"hashtags": ["rust"], "limit": 10}),
    )
    .await;
    assert_eq!(by_tag.as_array().unwrap().len(), 1);
    assert_eq!(by_tag[0]["node_id"], node["node_id"]);

    let by_keyword = call_json(
        &ctx,
        "memory_search",
        json!({"query": "durable", "hashtags": ["#knowledge_base"]}),
    )
    .await;
    assert_eq!(by_keyword.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn memory_links_are_returned_and_deleted_with_nodes() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    let component = call_json(
        &ctx,
        "memory_create_node",
        json!({"node_type": "component", "title": "tools.rs", "hashtags": ["tools"]}),
    )
    .await;
    let observation = call_json(
        &ctx,
        "memory_create_node",
        json!({"node_type": "observation", "title": "Tool contract observation", "content": "Knowledge-base tools use unified graph semantics.", "hashtags": ["tools", "knowledge-base"]}),
    )
    .await;

    let edge = call_json(
        &ctx,
        "memory_link_nodes",
        json!({
            "from_node_id": observation["node_id"],
            "to_node_id": component["node_id"],
            "relation_type": "documents"
        }),
    )
    .await;
    assert_eq!(edge["relation_type"], "documents");

    let fetched = call_json(
        &ctx,
        "memory_get_node",
        json!({"node_id": observation["node_id"]}),
    )
    .await;
    assert_eq!(fetched["outgoing"].as_array().unwrap().len(), 1);

    let graph = call_json(
        &ctx,
        "memory_open_graph",
        json!({"node_id": observation["node_id"], "depth": 1}),
    )
    .await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);

    let deleted = call_json(
        &ctx,
        "memory_delete_node",
        json!({"node_id": component["node_id"]}),
    )
    .await;
    assert_eq!(deleted["deleted"], true);

    let fetched_after_delete = call_json(
        &ctx,
        "memory_get_node",
        json!({"node_id": observation["node_id"]}),
    )
    .await;
    assert!(fetched_after_delete["outgoing"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn memory_search_supports_all_match_hashtags() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Rust observation", "hashtags": ["rust"]}),
    )
    .await;
    let both = call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Rust provider fact", "hashtags": ["rust", "provider"]}),
    )
    .await;

    let results = call_json(
        &ctx,
        "memory_search",
        json!({"hashtags": ["rust", "provider"], "hashtag_match": "all"}),
    )
    .await;
    assert_eq!(results.as_array().unwrap().len(), 1);
    assert_eq!(results[0]["node_id"], both["node_id"]);
}

#[tokio::test]
async fn memory_project_dir_defaults_and_global_selector_partition_results() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    let project_node = call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Project-specific fact", "hashtags": ["partition"]}),
    )
    .await;
    assert_eq!(project_node["project_dir"], ".");

    let global_node = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "project_dir": "[GLOBAL]",
            "title": "Global reusable fact",
            "hashtags": ["partition"]
        }),
    )
    .await;
    assert_eq!(global_node["project_dir"], "[GLOBAL]");

    let default_results = call_json(
        &ctx,
        "memory_search",
        json!({"hashtags": ["partition"], "limit": 10}),
    )
    .await;
    assert_eq!(default_results.as_array().unwrap().len(), 1);
    assert_eq!(default_results[0]["node_id"], project_node["node_id"]);

    let global_results = call_json(
        &ctx,
        "memory_search",
        json!({"project_dir": "[GLOBAL]", "hashtags": ["partition"], "limit": 10}),
    )
    .await;
    assert_eq!(global_results.as_array().unwrap().len(), 1);
    assert_eq!(global_results[0]["node_id"], global_node["node_id"]);
}

#[tokio::test]
async fn memory_list_hashtags_is_scoped_by_project_dir() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Project item", "hashtags": ["project-only"]}),
    )
    .await;
    call_json(
        &ctx,
        "memory_create_node",
        json!({"project_dir": "[GLOBAL]", "title": "Global item", "hashtags": ["global-only"]}),
    )
    .await;

    let project_tags = call_json(&ctx, "memory_list_hashtags", json!({})).await;
    assert_eq!(project_tags.as_array().unwrap().len(), 1);
    assert_eq!(project_tags[0]["hashtag"], "#project_only");

    let global_tags = call_json(
        &ctx,
        "memory_list_hashtags",
        json!({"project_dir": "[GLOBAL]"}),
    )
    .await;
    assert_eq!(global_tags.as_array().unwrap().len(), 1);
    assert_eq!(global_tags[0]["hashtag"], "#global_only");
}
