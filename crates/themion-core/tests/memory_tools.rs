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
        local_agent_tool_invoker: None,
        system_inspection: None,
    }
}

fn test_ctx_with_project_dir(db: std::sync::Arc<DbHandle>, project_dir: &str) -> ToolCtx {
    ToolCtx {
        project_dir: PathBuf::from(project_dir),
        ..test_ctx(db)
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
    assert_eq!(node["entity"], "memory_node");
    assert_eq!(node["operation"], "create");

    let by_tag = call_json(
        &ctx,
        "unified_search",
        json!({"hashtags": ["rust"], "source_kinds": ["memory"], "limit": 10}),
    )
    .await;
    assert_eq!(by_tag["results"].as_array().unwrap().len(), 1);
    assert_eq!(by_tag["results"][0]["source_id"], node["node_id"]);

    let by_keyword = call_json(
        &ctx,
        "unified_search",
        json!({"query": "durable", "hashtags": ["#knowledge_base"], "source_kinds": ["memory"]}),
    )
    .await;
    assert_eq!(by_keyword["results"].as_array().unwrap().len(), 1);
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
    assert_eq!(edge["entity"], "memory_edge");
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
async fn unified_search_supports_all_match_hashtags() {
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
        "unified_search",
        json!({"hashtags": ["rust", "provider"], "hashtag_match": "all"}),
    )
    .await;
    assert_eq!(results["results"].as_array().unwrap().len(), 1);
    assert_eq!(results["results"][0]["source_id"], both["node_id"]);
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
        "unified_search",
        json!({"hashtags": ["partition"], "limit": 10}),
    )
    .await;
    assert_eq!(default_results["results"].as_array().unwrap().len(), 1);
    assert_eq!(default_results["results"][0]["source_id"], project_node["node_id"]);

    let global_results = call_json(
        &ctx,
        "unified_search",
        json!({"project_dir": "[GLOBAL]", "hashtags": ["partition"], "limit": 10}),
    )
    .await;
    assert_eq!(global_results["results"].as_array().unwrap().len(), 1);
    assert_eq!(global_results["results"][0]["source_id"], global_node["node_id"]);
}

#[cfg(feature = "semantic-memory")]
#[tokio::test]
async fn memory_create_node_succeeds_without_legacy_embedding_table() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db.clone());

    let node = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "node_type": "observation",
            "title": "Create without legacy table",
            "content": "Project Memory writes should not depend on retired embedding storage.",
            "hashtags": ["semantic-memory", "regression"]
        }),
    )
    .await;

    assert_eq!(node["entity"], "memory_node");
    assert_eq!(node["operation"], "create");

    let results = call_json(
        &ctx,
        "unified_search",
        json!({
            "query": "retired embedding storage",
            "source_kinds": ["memory"],
            "mode": "fts",
            "limit": 10
        }),
    )
    .await;
    assert!(results["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source_id"] == node["node_id"]));
}

#[cfg(feature = "semantic-memory")]
#[tokio::test]
async fn memory_update_node_succeeds_without_legacy_embedding_table() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db.clone());

    let created = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "title": "Update without legacy table",
            "content": "original",
            "hashtags": ["semantic-memory", "regression"]
        }),
    )
    .await;

    let updated = call_json(
        &ctx,
        "memory_update_node",
        json!({
            "node_id": created["node_id"],
            "content": "updated content without legacy table",
            "hashtags": ["semantic-memory", "updated"]
        }),
    )
    .await;

    assert_eq!(updated["entity"], "memory_node");
    assert_eq!(updated["operation"], "update");

    let results = call_json(
        &ctx,
        "unified_search",
        json!({
            "query": "updated content without legacy table",
            "source_kinds": ["memory"],
            "mode": "fts",
            "limit": 10
        }),
    )
    .await;
    assert!(results["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source_id"] == created["node_id"]));
}


#[tokio::test]
async fn project_dir_dot_matches_current_project_for_targeted_tools() {
    let db = DbHandle::open_in_memory().unwrap();
    let current_project = "/tmp/themion-prd094-current-project";
    let other_project = "/tmp/themion-prd094-other-project";
    let ctx = test_ctx_with_project_dir(db.clone(), current_project);

    let default_node = call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Current project default", "hashtags": ["dot-fallback"]}),
    )
    .await;
    let explicit_node = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "project_dir": current_project,
            "title": "Current project explicit",
            "hashtags": ["dot-fallback"]
        }),
    )
    .await;
    let dot_node = call_json(
        &ctx,
        "memory_create_node",
        json!({
            "project_dir": ".",
            "title": "Current project dot",
            "hashtags": ["dot-fallback"]
        }),
    )
    .await;
    call_json(
        &ctx,
        "memory_create_node",
        json!({
            "project_dir": other_project,
            "title": "Other project explicit",
            "hashtags": ["other-project-only"]
        }),
    )
    .await;

    assert_eq!(default_node["project_dir"], current_project);
    assert_eq!(explicit_node["project_dir"], current_project);
    assert_eq!(dot_node["project_dir"], current_project);

    let default_results = call_json(
        &ctx,
        "unified_search",
        json!({"hashtags": ["dot-fallback"], "source_kinds": ["memory"], "limit": 10}),
    )
    .await;
    let explicit_results = call_json(
        &ctx,
        "unified_search",
        json!({
            "project_dir": current_project,
            "hashtags": ["dot-fallback"],
            "source_kinds": ["memory"],
            "limit": 10
        }),
    )
    .await;
    let dot_results = call_json(
        &ctx,
        "unified_search",
        json!({
            "project_dir": ".",
            "hashtags": ["dot-fallback"],
            "source_kinds": ["memory"],
            "limit": 10
        }),
    )
    .await;

    assert_eq!(default_results["results"].as_array().unwrap().len(), 3);
    assert_eq!(explicit_results["results"].as_array().unwrap().len(), 3);
    assert_eq!(dot_results["results"].as_array().unwrap().len(), 3);

    let default_tags = call_json(&ctx, "memory_list_hashtags", json!({})).await;
    let explicit_tags = call_json(
        &ctx,
        "memory_list_hashtags",
        json!({"project_dir": current_project}),
    )
    .await;
    let dot_tags = call_json(
        &ctx,
        "memory_list_hashtags",
        json!({"project_dir": "."}),
    )
    .await;

    assert_eq!(default_tags, explicit_tags);
    assert_eq!(default_tags, dot_tags);
    assert_eq!(default_tags.as_array().unwrap().len(), 1);
    assert_eq!(default_tags[0]["hashtag"], "#dot_fallback");

    let other_tags = call_json(
        &ctx,
        "memory_list_hashtags",
        json!({"project_dir": other_project}),
    )
    .await;
    assert_eq!(other_tags.as_array().unwrap().len(), 1);
    assert_eq!(other_tags[0]["hashtag"], "#other_project_only");
}

#[tokio::test]
async fn unified_search_rebuild_treats_project_dir_dot_as_current_project() {
    let db = DbHandle::open_in_memory().unwrap();
    let current_project = "/tmp/themion-prd094-rebuild-project";
    let ctx = test_ctx_with_project_dir(db.clone(), current_project);

    call_json(
        &ctx,
        "memory_create_node",
        json!({
            "title": "Rebuild target node",
            "content": "used to test project_dir dot rebuild",
            "hashtags": ["rebuild-dot"]
        }),
    )
    .await;

    let default_rebuild = call_json(&ctx, "unified_search_rebuild", json!({})).await;
    let explicit_rebuild = call_json(
        &ctx,
        "unified_search_rebuild",
        json!({"project_dir": current_project}),
    )
    .await;
    let dot_rebuild = call_json(
        &ctx,
        "unified_search_rebuild",
        json!({"project_dir": "."}),
    )
    .await;

    assert_eq!(default_rebuild["project_dir"], current_project);
    assert_eq!(explicit_rebuild["project_dir"], current_project);
    assert_eq!(dot_rebuild["project_dir"], current_project);

    let dot_results = call_json(
        &ctx,
        "unified_search",
        json!({
            "project_dir": ".",
            "query": "rebuild target",
            "source_kinds": ["memory"],
            "limit": 10
        }),
    )
    .await;
    assert_eq!(dot_results["results"].as_array().unwrap().len(), 1);
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

#[tokio::test]
async fn board_and_file_mutations_return_compact_acks() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    let created = call_json(
        &ctx,
        "board_create_note",
        json!({"to_instance":"local","to_agent_id":"master","body":"hello"}),
    )
    .await;
    assert_eq!(created["entity"], "board_note");
    assert_eq!(created["operation"], "create");

    let moved = call_json(
        &ctx,
        "board_move_note",
        json!({"note_id": created["note_id"], "column":"done"}),
    )
    .await;
    assert_eq!(moved["operation"], "move");
    assert!(moved.get("body").is_none());

    let updated = call_json(
        &ctx,
        "board_update_note_result",
        json!({"note_id": created["note_id"], "result_text":"long text"}),
    )
    .await;
    assert_eq!(updated["operation"], "update_result");
    assert_eq!(updated["changed"]["has_result_text"], true);
    assert!(updated.get("result_text").is_none());

    let missing = call_json(
        &ctx,
        "board_move_note",
        json!({"note_id": "missing", "column":"done"}),
    )
    .await;
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["found"], false);

    let tmp = std::env::temp_dir().join(format!("themion-tools-test-{}.txt", Uuid::new_v4()));
    let rel = tmp.to_string_lossy().to_string();
    let write = call_json(
        &ctx,
        "fs_write_file",
        json!({"path": rel, "content":"abc", "mode":"raw"}),
    )
    .await;
    assert_eq!(write["operation"], "write");
    assert_eq!(write["written_bytes"], 3);
}

#[tokio::test]
async fn unified_search_reports_unavailable_non_memory_source_kinds() {
    let db = DbHandle::open_in_memory().unwrap();
    let ctx = test_ctx(db);

    call_json(
        &ctx,
        "memory_create_node",
        json!({"title": "Project fact", "content": "search me"}),
    )
    .await;

    let results = call_json(
        &ctx,
        "unified_search",
        json!({"query": "search", "source_kinds": ["memory", "chat_message"], "limit": 10}),
    )
    .await;

    assert_eq!(results["degraded"], false);
    assert_eq!(results["unavailable_source_kinds"], json!([]));
    assert_eq!(results["results"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unified_search_returns_chat_message_and_tool_result_rows() {
    use themion_core::client::Message;

    let db = DbHandle::open_in_memory().unwrap();
    let session_id = Uuid::new_v4();
    let workflow = WorkflowState::default();
    db.insert_session(session_id, PathBuf::from(".").as_path(), true).unwrap();
    let turn_id = db.begin_turn(session_id, 1, &workflow, None).unwrap();

    let user_msg = Message {
        role: "user".to_string(),
        content: Some("searchable chat text".to_string()),
        tool_calls: None,
        tool_call_id: None,
    };
    db.append_message(turn_id, session_id, 1, &user_msg, &workflow).unwrap();

    let tool_msg = Message {
        role: "tool".to_string(),
        content: Some(r#"{"tool_name":"fs_read_file","result":"searchable tool result"}"#.to_string()),
        tool_calls: None,
        tool_call_id: Some("call-1".to_string()),
    };
    db.append_message(turn_id, session_id, 2, &tool_msg, &workflow).unwrap();

    let ctx = test_ctx(db.clone());
    let chat_results = call_json(
        &ctx,
        "unified_search",
        json!({"query": "chat", "source_kinds": ["chat_message"], "limit": 10}),
    )
    .await;
    assert_eq!(chat_results["results"].as_array().unwrap().len(), 1);
    assert_eq!(chat_results["results"][0]["source_kind"], "chat_message");

    let tool_results = call_json(
        &ctx,
        "unified_search",
        json!({"query": "tool", "source_kinds": ["tool_result"], "limit": 10}),
    )
    .await;
    assert_eq!(tool_results["results"].as_array().unwrap().len(), 1);
    assert_eq!(tool_results["results"][0]["source_kind"], "tool_result");
}
