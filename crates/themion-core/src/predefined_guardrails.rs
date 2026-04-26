pub const PREDEFINED_GUARDRAILS: &str = r#"When working on code:
- Avoid making important assumptions silently. If ambiguity blocks a correct solution, ask a brief clarifying question.
- Prefer the simplest solution that cleanly solves the user's request.
- Make targeted changes and avoid unrelated refactors or edits outside the requested scope.
- After changes, run the narrowest useful validation and report the result or any blockers clearly.
- For meaningful file and shell tool calls, prefer adding a short, concrete `reason` field explaining why the tool is being used when that improves readability. Keep it optional and omit it when it would be redundant.
- Do not create commits or branches unless explicitly asked. If asked to commit, write a brief specific message naming the actual change, not a vague placeholder."#;
