import type { ChatMessage } from "./llm.ts";
import { CONTEXT_CHAR_BUDGET } from "./config.ts";

export class ContextWindow {
  private messages: ChatMessage[] = [];
  private systemMessage: ChatMessage;

  constructor(systemPrompt: string) {
    this.systemMessage = { role: "system", content: systemPrompt };
  }

  addMessage(msg: ChatMessage): void {
    this.messages.push(msg);
  }

  getMessages(): ChatMessage[] {
    const selected: ChatMessage[] = [];
    let chars = 0;

    // Walk backwards, keeping tool call/result pairs atomic
    let i = this.messages.length - 1;
    while (i >= 0) {
      // Collect a "group": tool results + their preceding assistant message
      const group: ChatMessage[] = [];
      // Gather contiguous tool result messages
      while (i >= 0 && this.messages[i].role === "tool") {
        group.unshift(this.messages[i]);
        i--;
      }
      // Include the assistant message that produced the tool calls
      if (i >= 0 && this.messages[i].role === "assistant" && this.messages[i].tool_calls?.length) {
        group.unshift(this.messages[i]);
        i--;
      } else if (group.length === 0 && i >= 0) {
        // Regular message (user or assistant without tool calls)
        group.unshift(this.messages[i]);
        i--;
      }

      const groupLen = group.reduce((s, m) => s + JSON.stringify(m).length, 0);
      if (chars + groupLen > CONTEXT_CHAR_BUDGET && selected.length > 0) {
        break;
      }

      selected.unshift(...group);
      chars += groupLen;
    }

    return [this.systemMessage, ...selected];
  }

  static truncateContent(content: string, maxChars: number = 1500): string {
    if (content.length <= maxChars) return content;
    return content.slice(0, maxChars) + "... [truncated]";
  }
}
