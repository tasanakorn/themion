import { registry } from "../registry.ts";
import { guardPath } from "../guard.ts";

registry.register(
  "read_file",
  "Read file contents",
  {
    type: "object",
    properties: {
      path: { type: "string", description: "File path" },
    },
    required: ["path"],
  },
  async (args) => {
    const abs = guardPath(args.path as string);
    return await Bun.file(abs).text();
  },
);
