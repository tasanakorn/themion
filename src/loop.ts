import { chatCompletionStream } from "./llm.ts";
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

    try {
      for await (const chunk of chatCompletionStream(context.getMessages(), [])) {
        switch (chunk.type) {
          case "text_delta":
            contentAccum += chunk.content;
            yield { type: "text", content: chunk.content };
            break;

          case "tool_call_delta":
            // Server-side tool calls are disabled — we parse from text below.
            break;

          case "finish":
            break;
        }
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      yield { type: "error", message: msg };
      return;
    }

    // Parse tool calls from the model's plain-text output (<tool_call>{...}</tool_call>).
    const toolCalls = contentAccum ? extractToolCalls(contentAccum) : [];

    // Always record the raw assistant text so the model can see what it said
    // (including any <tool_call> tags) on the next turn.
    context.addMessage({ role: "assistant", content: contentAccum || null });

    if (toolCalls.length === 0) {
      yield { type: "done", content: contentAccum || "[No response]" };
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
      // Feed tool result back as a user message. We deliberately avoid
      // `role: "tool"` because llama.cpp wraps it in its tool-calling
      // template, which is what caused the `<|tool_response|>` leak.
      context.addMessage({
        role: "user",
        content: `Tool result for \`${tc.function.name}\`:\n\`\`\`\n${result}\n\`\`\``,
      });
    }
  }

  yield { type: "done", content: "[Max turns reached]" };
}
