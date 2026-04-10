import { LLM_ENDPOINT, MAX_TOKENS, TEMPERATURE, ENABLE_THINKING } from "./config.ts";

// Special tokens that some local models (Gemma, harmony-format models, etc.)
// leak into `content`. We ask the server to stop on the real turn-end markers
// and strip any tokens that still slip through on the client.
//
// NOTE: `<eos>` is deliberately NOT in STOP_TOKENS — this server emits it as
// the *first* output token in some responses, and using it as a stop sequence
// halts generation at position 0, yielding an empty response. We strip it
// client-side via ARTIFACT_RE instead.
const STOP_TOKENS = ["<end_of_turn>", "<|end|>", "<|return|>", "<|eot_id|>"];

// Broad pattern: strip literal Gemma markers plus anything shaped like a
// harmony-style special token (`<|start|>`, `<|end|>`, `<|tool_response|>`,
// `<|channel|>`, even the corrupted `<channel|>` variant). The inner class
// is bounded to 60 chars without `<`, `>`, or newlines to avoid eating
// legitimate prose or markup.
const ARTIFACT_RE = /<eos>|<end_of_turn>|<channel\|>|<\|[^<>\n]{0,60}?\|?>/g;

// Longest realistic artifact we need to hold back across chunk boundaries.
// "<|tool_response|>" is 17 chars; round up for headroom.
const MAX_ARTIFACT_LEN = 20;

function stripArtifacts(s: string | null): string | null {
  if (s == null) return s;
  return s.replace(ARTIFACT_RE, "");
}

export type ChatMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
};

export type ToolCall = {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
};

export type ToolDef = {
  type: "function";
  function: { name: string; description: string; parameters: object };
};

export type LLMResponse = {
  content: string | null;
  tool_calls: ToolCall[];
  finish_reason: string;
};

export type StreamChunk =
  | { type: "text_delta"; content: string }
  | { type: "tool_call_delta"; index: number; id?: string; name?: string; arguments_delta: string }
  | { type: "finish"; finish_reason: string };

export async function chatCompletion(messages: ChatMessage[], _tools: ToolDef[]): Promise<LLMResponse> {
  // Tools are intentionally NOT forwarded to the server. See formatToolsPrompt
  // in registry.ts — we inject tool specs as system-prompt text and parse
  // calls from model output, bypassing llama.cpp's tool-calling template.
  const body: Record<string, unknown> = {
    messages,
    max_tokens: MAX_TOKENS,
    temperature: TEMPERATURE,
    stop: STOP_TOKENS,
    chat_template_kwargs: { enable_thinking: ENABLE_THINKING },
  };

  let res: Response;
  try {
    res = await fetch(LLM_ENDPOINT, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (err) {
    throw new Error(`Fetch error: ${err instanceof Error ? err.message : err}`);
  }

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`LLM request failed (${res.status}): ${text}`);
  }

  const data = await res.json() as {
    choices: Array<{
      message: {
        content: string | null;
        tool_calls?: Array<{
          id?: string;
          type: "function";
          function: { name: string; arguments: string | object };
        }>;
      };
      finish_reason: string;
    }>;
  };

  if (!data.choices?.length) {
    throw new Error("LLM returned empty choices array");
  }

  const message = data.choices[0].message;
  const content = stripArtifacts(message.content ?? null);
  const finish_reason = data.choices[0].finish_reason;

  const rawToolCalls = message.tool_calls ?? [];
  const tool_calls: ToolCall[] = rawToolCalls.map((tc, i) => ({
    id: tc.id ?? `call_${Date.now()}_${i}`,
    type: "function",
    function: {
      name: tc.function.name,
      arguments: typeof tc.function.arguments === "string"
        ? tc.function.arguments
        : JSON.stringify(tc.function.arguments),
    },
  }));

  return { content, tool_calls, finish_reason };
}

export async function* chatCompletionStream(
  messages: ChatMessage[],
  _tools: ToolDef[],
): AsyncGenerator<StreamChunk> {
  // Tools are intentionally NOT forwarded — see `chatCompletion` above.
  const body: Record<string, unknown> = {
    messages,
    max_tokens: MAX_TOKENS,
    temperature: TEMPERATURE,
    stream: true,
    stop: STOP_TOKENS,
    chat_template_kwargs: { enable_thinking: ENABLE_THINKING },
  };

  let res: Response;
  try {
    res = await fetch(LLM_ENDPOINT, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (err) {
    throw new Error(`Fetch error: ${err instanceof Error ? err.message : err}`);
  }

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`LLM request failed (${res.status}): ${text}`);
  }

  if (!res.body) {
    throw new Error("LLM response has no body for streaming");
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  // Pending text holdback so artifact tokens split across chunk boundaries
  // (e.g. "<eo" + "s>") still get stripped before reaching the UI.
  let pending = "";

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });

      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith("data: ")) continue;

        const payload = trimmed.slice(6);
        if (payload === "[DONE]") {
          return;
        }

        let data: any;
        try {
          data = JSON.parse(payload);
        } catch {
          continue;
        }

        const choice = data.choices?.[0];
        if (!choice) continue;

        const delta = choice.delta;
        if (!delta) continue;

        if (delta.content) {
          pending += delta.content;
          // Strip any complete artifact tokens present in pending.
          pending = pending.replace(ARTIFACT_RE, "");
          // Emit everything except the trailing tail that could still be the
          // start of an artifact token. If the tail contains no '<', flush all.
          let emitUpTo = pending.length;
          const lastLt = pending.lastIndexOf("<");
          if (lastLt !== -1 && pending.length - lastLt <= MAX_ARTIFACT_LEN) {
            emitUpTo = lastLt;
          }
          if (emitUpTo > 0) {
            const out = pending.slice(0, emitUpTo);
            pending = pending.slice(emitUpTo);
            yield { type: "text_delta", content: out };
          }
        }

        if (delta.tool_calls) {
          for (const tc of delta.tool_calls) {
            yield {
              type: "tool_call_delta",
              index: tc.index ?? 0,
              id: tc.id ?? undefined,
              name: tc.function?.name ?? undefined,
              arguments_delta: tc.function?.arguments ?? "",
            };
          }
        }

        if (choice.finish_reason) {
          // Flush any held-back text, stripping any remaining artifacts.
          if (pending) {
            const tail = pending.replace(ARTIFACT_RE, "");
            pending = "";
            if (tail) yield { type: "text_delta", content: tail };
          }
          yield { type: "finish", finish_reason: choice.finish_reason };
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
