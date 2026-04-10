import React, { useState, useCallback } from "react";
import { Box, Text, useApp } from "ink";
import { MessageList, type Message } from "./MessageList.tsx";
import { ToolStatus, type ActiveTool } from "./ToolStatus.tsx";
import { InputPrompt } from "./InputPrompt.tsx";
import { runAgentLoop } from "../loop.ts";
import { ContextWindow } from "../context.ts";
import { ToolRegistry } from "../registry.ts";

type Props = {
  context: ContextWindow;
  registry: ToolRegistry;
};

export function App({ context, registry }: Props) {
  const { exit } = useApp();
  const [messages, setMessages] = useState<Message[]>([]);
  const [streamingText, setStreamingText] = useState("");
  const [activeTools, setActiveTools] = useState<ActiveTool[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);

  const handleSubmit = useCallback(
    async (value: string) => {
      const trimmed = value.trim();
      if (!trimmed) return;

      if (trimmed === "exit" || trimmed === "quit") {
        exit();
        return;
      }

      // Slash commands
      if (trimmed.startsWith("/")) {
        setInput("");
        const [cmd, ...rest] = trimmed.slice(1).split(/\s+/);
        switch (cmd) {
          case "exit":
          case "quit":
            exit();
            return;
          case "clear":
            setMessages([]);
            return;
          case "help":
            setMessages((prev) => [
              ...prev,
              {
                role: "assistant",
                content: [
                  "**Commands:**",
                  "  `/help`   — show this help",
                  "  `/clear`  — clear conversation",
                  "  `/tools`  — list available tools",
                  "  `/exit`   — quit themion",
                ].join("\n"),
              },
            ]);
            return;
          case "tools":
            setMessages((prev) => [
              ...prev,
              {
                role: "assistant",
                content: registry
                  .getDefinitions()
                  .map((t) => `  **${t.function.name}** — ${t.function.description}`)
                  .join("\n"),
              },
            ]);
            return;
          default:
            setMessages((prev) => [
              ...prev,
              {
                role: "assistant",
                content: `Unknown command: /${cmd}. Type /help for available commands.`,
              },
            ]);
            return;
        }
      }

      setInput("");
      setBusy(true);
      setMessages((prev) => [...prev, { role: "user", content: trimmed }]);
      setStreamingText("");
      setActiveTools([]);

      let fullText = "";

      try {
        for await (const event of runAgentLoop(trimmed, context, registry)) {
          switch (event.type) {
            case "thinking":
              break;

            case "text":
              fullText += event.content;
              setStreamingText(fullText);
              break;

            case "tool_call":
              setActiveTools((prev) => [
                ...prev,
                { name: event.name, status: "running" },
              ]);
              break;

            case "tool_result":
              setActiveTools((prev) =>
                prev.map((t) =>
                  t.name === event.name && t.status === "running"
                    ? { ...t, status: "done", result: event.result }
                    : t,
                ),
              );
              fullText = "";
              setStreamingText("");
              break;

            case "escalation":
              setMessages((prev) => [
                ...prev,
                { role: "assistant", content: `Escalation: ${event.reason}` },
              ]);
              break;

            case "done":
              setMessages((prev) => [
                ...prev,
                { role: "assistant", content: event.content },
              ]);
              setStreamingText("");
              break;

            case "error":
              setMessages((prev) => [
                ...prev,
                {
                  role: "assistant",
                  content: `Error: ${event.message}`,
                },
              ]);
              break;
          }
        }
      } catch (err) {
        setMessages((prev) => [
          ...prev,
          {
            role: "assistant",
            content: `Error: ${err instanceof Error ? err.message : String(err)}`,
          },
        ]);
      }

      setActiveTools([]);
      setStreamingText("");
      setBusy(false);
    },
    [context, registry, exit],
  );

  return (
    <Box flexDirection="column" gap={1}>
      <Text bold>themion v0.2.0</Text>

      <MessageList messages={messages} />

      {streamingText && (
        <Box>
          <Text color="green">{streamingText}</Text>
        </Box>
      )}

      <ToolStatus tools={activeTools} />

      <InputPrompt
        value={input}
        onChange={setInput}
        onSubmit={handleSubmit}
        disabled={busy}
      />
    </Box>
  );
}
