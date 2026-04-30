use std::env;
use std::fs;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tiktoken_rs::cl100k_base;

fn collect_tool_text_lengths(tools: &[Value]) -> (usize, usize, usize) {
    let mut tool_desc_chars = 0usize;
    let mut prop_desc_chars = 0usize;
    for tool in tools {
        if let Some(desc) = tool.get("description").and_then(Value::as_str) {
            tool_desc_chars += desc.len();
        }
        if let Some(props) = tool
            .get("parameters")
            .and_then(|p| p.get("properties"))
            .and_then(Value::as_object)
        {
            for spec in props.values() {
                if let Some(desc) = spec.get("description").and_then(Value::as_str) {
                    prop_desc_chars += desc.len();
                }
            }
        }
    }
    (tool_desc_chars, prop_desc_chars, tool_desc_chars + prop_desc_chars)
}

fn count_tokens_for_tools(tools: &[Value]) -> Result<usize> {
    let bpe = cl100k_base().context("load cl100k tokenizer")?;
    let raw = serde_json::to_string(tools).context("serialize tools json")?;
    Ok(bpe.encode_with_special_tokens(&raw).len())
}

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: tool_schema_token_check <round_json>")?;
    let raw = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
    let doc: Value = serde_json::from_str(&raw).with_context(|| format!("parse {path}"))?;
    let request = doc.get("request").context("missing request")?;
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .context("missing request.tools array")?;

    let (tool_desc_chars, prop_desc_chars, total_desc_chars) = collect_tool_text_lengths(tools);
    let total_tool_tokens = count_tokens_for_tools(tools)?;
    let no_desc_tools: Vec<Value> = tools
        .iter()
        .map(|tool| {
            let mut tool = tool.clone();
            if let Some(obj) = tool.as_object_mut() {
                obj.remove("description");
                if let Some(params) = obj.get_mut("parameters").and_then(Value::as_object_mut) {
                    if let Some(props) = params.get_mut("properties").and_then(Value::as_object_mut) {
                        for spec in props.values_mut() {
                            if let Some(spec_obj) = spec.as_object_mut() {
                                spec_obj.remove("description");
                            }
                        }
                    }
                }
            }
            tool
        })
        .collect();
    let no_desc_tokens = count_tokens_for_tools(&no_desc_tools)?;
    let desc_only_tokens = total_tool_tokens.saturating_sub(no_desc_tokens);

    let report = json!({
        "path": path,
        "tool_count": tools.len(),
        "tool_description_chars": tool_desc_chars,
        "property_description_chars": prop_desc_chars,
        "total_description_chars": total_desc_chars,
        "tool_json_tokens_total": total_tool_tokens,
        "tool_json_tokens_without_descriptions": no_desc_tokens,
        "estimated_description_only_tokens": desc_only_tokens,
        "chars_per_description_token": if desc_only_tokens > 0 {
            json!(total_desc_chars as f64 / desc_only_tokens as f64)
        } else {
            Value::Null
        },
        "chars_per_all_tool_tokens": if total_tool_tokens > 0 {
            json!(total_desc_chars as f64 / total_tool_tokens as f64)
        } else {
            Value::Null
        }
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
