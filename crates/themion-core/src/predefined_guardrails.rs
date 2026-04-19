pub const PREDEFINED_GUARDRAILS: &str = r#"When working on code:
- Avoid making important assumptions silently. If ambiguity blocks a correct solution, ask a brief clarifying question.
- Prefer the simplest solution that cleanly solves the user's request.
- Make targeted changes and avoid unrelated refactors or edits outside the requested scope.
- After changes, run the narrowest useful validation and report the result or any blockers clearly."#;
