import React from "react";
import { Text, Box } from "ink";
import Spinner from "ink-spinner";

export type ActiveTool = {
  name: string;
  status: "running" | "done";
  result?: string;
};

export function ToolStatus({ tools }: { tools: ActiveTool[] }) {
  if (tools.length === 0) return null;

  return (
    <Box flexDirection="column">
      {tools.map((tool, i) => (
        <Box key={i} gap={1}>
          {tool.status === "running" ? (
            <Text color="yellow">
              <Spinner type="dots" /> {tool.name}
            </Text>
          ) : (
            <Text color="gray" dimColor>
              {"  ✓ "}{tool.name}{tool.result ? `: ${tool.result.slice(0, 80)}` : ""}
            </Text>
          )}
        </Box>
      ))}
    </Box>
  );
}
