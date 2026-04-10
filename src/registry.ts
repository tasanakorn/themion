import type { ToolDef } from "./llm.ts";
import { ContextWindow } from "./context.ts";
import { TOOL_RESULT_MAX_CHARS } from "./config.ts";

export class ToolRegistry {
  private tools: Map<string, {
    def: ToolDef;
    handler: (args: Record<string, unknown>) => Promise<string>;
  }> = new Map();

  register(
    name: string,
    description: string,
    parameters: object,
    handler: (args: Record<string, unknown>) => Promise<string>,
  ): void {
    const def: ToolDef = {
      type: "function",
      function: { name, description, parameters },
    };
    this.tools.set(name, { def, handler });
  }

  getDefinitions(): ToolDef[] {
    return Array.from(this.tools.values()).map((t) => t.def);
  }

  /**
   * Render tool definitions as a system-prompt block. Used when the upstream
   * server's tool-calling template is unreliable (e.g. llama.cpp wrapping
   * Gemma), so we inject tool specs as plain text and parse calls from the
   * model's text output instead of using the server's `tools` parameter.
   */
  formatToolsPrompt(): string {
    const lines: string[] = [];
    lines.push("You have access to the following tools. To call a tool, emit ONE JSON object wrapped in <tool_call>...</tool_call> tags on a line by itself:");
    lines.push("");
    lines.push('<tool_call>{"name": "tool_name", "arguments": {"key": "value"}}</tool_call>');
    lines.push("");
    lines.push("After you emit a tool call, stop and wait — the next user turn will contain the tool result. If you don't need a tool, answer in plain text.");
    lines.push("");
    lines.push("Available tools:");
    for (const { def } of this.tools.values()) {
      const { name, description, parameters } = def.function;
      lines.push("");
      lines.push(`- ${name}: ${description}`);
      lines.push(`  parameters: ${JSON.stringify(parameters)}`);
    }
    return lines.join("\n");
  }

  async dispatch(name: string, argsRaw: string): Promise<string> {
    const entry = this.tools.get(name);
    if (!entry) {
      return JSON.stringify({ error: "Unknown tool: " + name });
    }

    let args: Record<string, unknown>;
    try {
      args = JSON.parse(argsRaw) as Record<string, unknown>;
    } catch {
      return JSON.stringify({ error: `Failed to parse arguments: ${argsRaw}` });
    }

    let result: string;
    try {
      result = await entry.handler(args);
    } catch (err) {
      return `Error: ${err instanceof Error ? err.message : String(err)}`;
    }

    return ContextWindow.truncateContent(result, TOOL_RESULT_MAX_CHARS);
  }
}

export const registry = new ToolRegistry();
