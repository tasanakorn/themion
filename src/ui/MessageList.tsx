import React from "react";
import { Text, Box } from "ink";
import { marked } from "marked";
import { markedTerminal } from "marked-terminal";

marked.use(markedTerminal());

export type Message = {
  role: "user" | "assistant";
  content: string;
};

export function MessageList({ messages }: { messages: Message[] }) {
  return (
    <Box flexDirection="column" gap={1}>
      {messages.map((msg, i) => (
        <Box key={i} flexDirection="column">
          {msg.role === "user" ? (
            <Text bold color="cyan">{"❯ "}{msg.content}</Text>
          ) : (
            <Text>{(marked.parse(msg.content) as string).trimEnd()}</Text>
          )}
        </Box>
      ))}
    </Box>
  );
}
