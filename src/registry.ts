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
