export const LLM_BASE_URL = process.env.LLM_URL ?? "http://localhost:30434";
export const LLM_ENDPOINT = `${LLM_BASE_URL}/v1/chat/completions`;
export const MAX_TOKENS = 512;
export const TEMPERATURE = 0;
// Disable Gemma 3n's built-in thinking/reasoning so all output goes to `content`.
// With thinking enabled, output goes to `reasoning_content` and can easily exhaust
// the token budget before emitting the actual answer.
export const ENABLE_THINKING = false;
export const CONTEXT_CHAR_BUDGET = 5500;
export const MAX_TURNS = 15;
export const ALLOWED_PATH_PREFIXES = [
  "/home/tas/Documents/Projects/workspace-stele/themion",
  "/tmp",
];
export const ALLOWED_COMMANDS = [
  "ls", "cat", "echo", "find", "wc", "head", "tail", "grep",
  "mkdir", "cp", "mv", "git", "which", "pwd", "date",
];
export const SYSTEM_PROMPT = `You are Themion, a task-execution agent. You have tools to interact with the filesystem and shell. Use tools to accomplish the user's request.

Style rules:
- Keep your own prose concise — no filler, no "how can I help you next" closers.
- When presenting tool results (lists, tables, file contents, command output), preserve every entry and its key attributes verbatim. Do not collapse items into a summary or drop details the user asked to see.
- If a task is too complex for you, use the escalate tool.`;
export const TOOL_RESULT_MAX_CHARS = 1500;
