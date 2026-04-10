import { chatCompletionStream } from "./llm.ts";
import type { ToolCall } from "./llm.ts";
import { extractToolCalls } from "./parse-fallback.ts";
import { ContextWindow } from "./context.ts";
import { ToolRegistry } from "./registry.ts";
import { MAX_TURNS } from "./config.ts";

export type AgentEvent =
  | { type: "thinking" }
  | { type: "text"; content: string }
  | { type: "tool_call"; name: string; args: string }
  | { type: "tool_result"; name: string; result: string }
  | { type: "escalation"; reason: string }
  | { type: "done"; content: string }
  | { type: "error"; message: string };

export async function* runAgentLoop(
  userMessage: string,
  context: ContextWindow,
  toolRegistry: ToolRegistry,
): AsyncGenerator<AgentEvent> {
  context.addMessage({ role: "user", content: userMessage });

  for (let turn = 0; turn < MAX_TURNS; turn++) {
    yield { type: "thinking" };

    let contentAccum = "";
    const toolCallAccum = new Map<number, { id: string; name: string; arguments: string }>();

    try {
      for await (const chunk of chatCompletionStream(
        context.getMessages(),
        toolRegistry.getDefinitions(),
      )) {
        switch (chunk.type) {
          case "text_delta":
            contentAccum += chunk.content;
            yield { type: "text", content: chunk.content };
            break;

          case "tool_call_delta": {
            let entry = toolCallAccum.get(chunk.index);
            if (!entry) {
              entry = { id: "", name: "", arguments: "" };
              toolCallAccum.set(chunk.index, entry);
            }
            if (chunk.id) entry.id = chunk.id;
            if (chunk.name) entry.name = chunk.name;
            entry.arguments += chunk.arguments_delta;
            break;
          }

          case "finish":
            break;
        }
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      yield { type: "error", message: msg };
      return;
    }

    let toolCalls: ToolCall[] = Array.from(toolCallAccum.entries())
      .sort(([a], [b]) => a - b)
      .map(([i, tc]) => ({
        id: tc.id || `call_${Date.now()}_${i}`,
        type: "function" as const,
        function: {
          name: tc.name,
          arguments: tc.arguments,
        },
      }));

    if (toolCalls.length === 0 && contentAccum) {
      toolCalls = extractToolCalls(contentAccum);
    }

    const content = contentAccum || null;
    context.addMessage({
      role: "assistant",
      content,
      ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}),
    });

    if (toolCalls.length === 0) {
      yield { type: "done", content: content ?? "[No response]" };
      return;
    }

    for (const tc of toolCalls) {
      yield { type: "tool_call", name: tc.function.name, args: tc.function.arguments };

      const result = await toolRegistry.dispatch(tc.function.name, tc.function.arguments);

      if (result.startsWith("ESCALATION_REQUESTED:")) {
        const reason = result.slice(22);
        yield { type: "escalation", reason };
        return;
      }

      yield { type: "tool_result", name: tc.function.name, result };
      context.addMessage({ role: "tool", tool_call_id: tc.id, content: result });
    }
  }

  yield { type: "done", content: "[Max turns reached]" };
}
