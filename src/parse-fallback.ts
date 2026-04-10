import type { ToolCall } from "./llm.ts";

export function extractToolCalls(content: string): ToolCall[] {
  const results: ToolCall[] = [];

  const tagRegex = /<tool_call>\s*([\s\S]*?)\s*<\/tool_call>/g;
  let match: RegExpExecArray | null;
  let i = 0;

  while ((match = tagRegex.exec(content)) !== null) {
    try {
      const parsed = JSON.parse(match[1]) as { name: string; arguments: unknown };
      if (parsed.name) {
        results.push({
          id: `call_fb_${Date.now()}_${i++}`,
          type: "function",
          function: {
            name: parsed.name,
            arguments: typeof parsed.arguments === "string"
              ? parsed.arguments
              : JSON.stringify(parsed.arguments ?? {}),
          },
        });
      }
    } catch {
      // skip invalid JSON
    }
  }

  if (results.length === 0) {
    try {
      const parsed = JSON.parse(content.trim()) as { name: string; arguments: unknown };
      if (parsed.name && parsed.arguments !== undefined) {
        results.push({
          id: `call_fb_${Date.now()}_0`,
          type: "function",
          function: {
            name: parsed.name,
            arguments: typeof parsed.arguments === "string"
              ? parsed.arguments
              : JSON.stringify(parsed.arguments),
          },
        });
      }
    } catch {
      // not JSON
    }
  }

  return results;
}
