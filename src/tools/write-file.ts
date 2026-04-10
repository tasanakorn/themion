import { registry } from "../registry.ts";
import { guardPath } from "../guard.ts";

registry.register(
  "write_file",
  "Write content to a file",
  {
    type: "object",
    properties: {
      path: { type: "string", description: "File path" },
      content: { type: "string", description: "Content to write" },
    },
    required: ["path", "content"],
  },
  async (args) => {
    const abs = guardPath(args.path as string);
    const content = args.content as string;
    await Bun.write(abs, content);
    return `Written ${content.length} bytes to ${abs}`;
  },
);
