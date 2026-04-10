import "./tools/shell.ts";
import "./tools/read-file.ts";
import "./tools/write-file.ts";
import "./tools/list-files.ts";
import "./tools/escalate.ts";

import { ContextWindow } from "./context.ts";
import { registry } from "./registry.ts";
import { runAgentLoop } from "./loop.ts";
import { SYSTEM_PROMPT } from "./config.ts";

const context = new ContextWindow(SYSTEM_PROMPT);

// Single-shot mode: drain generator, print to stdout, no Ink
const args = process.argv.slice(2);
if (args.length > 0) {
  const message = args.join(" ");
  let finalContent = "";

  for await (const event of runAgentLoop(message, context, registry)) {
    switch (event.type) {
      case "tool_call":
        process.stderr.write(`[tool] ${event.name}(${event.args})\n`);
        break;
      case "tool_result":
        process.stderr.write(`[result] ${event.result.slice(0, 200)}\n`);
        break;
      case "escalation":
        console.log(`Escalation: ${event.reason}`);
        process.exit(0);
        break;
      case "done":
        finalContent = event.content;
        break;
      case "error":
        console.error(`Error: ${event.message}`);
        process.exit(1);
        break;
    }
  }

  console.log(finalContent);
  process.exit(0);
}

// REPL mode: mount Ink app
const React = await import("react");
const { render } = await import("ink");
const { App } = await import("./ui/App.tsx");

const { waitUntilExit, unmount } = render(React.createElement(App, { context, registry }));
await waitUntilExit();
process.exit(0);
