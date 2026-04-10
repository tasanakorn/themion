import { LLM_ENDPOINT, MAX_TOKENS, TEMPERATURE } from "./config.ts";

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

export async function chatCompletion(messages: ChatMessage[], tools: ToolDef[]): Promise<LLMResponse> {
  const body: Record<string, unknown> = {
    messages,
    max_tokens: MAX_TOKENS,
    temperature: TEMPERATURE,
  };

  if (tools.length > 0) {
    body.tools = tools;
    body.tool_choice = "auto";
  }

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
  const content = message.content ?? null;
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
  tools: ToolDef[],
): AsyncGenerator<StreamChunk> {
  const body: Record<string, unknown> = {
    messages,
    max_tokens: MAX_TOKENS,
    temperature: TEMPERATURE,
    stream: true,
  };

  if (tools.length > 0) {
    body.tools = tools;
    body.tool_choice = "auto";
  }

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
          yield { type: "text_delta", content: delta.content };
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
          yield { type: "finish", finish_reason: choice.finish_reason };
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
