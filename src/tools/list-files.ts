import { registry } from "../registry.ts";
import { guardPath } from "../guard.ts";
import { readdir } from "node:fs/promises";

registry.register(
  "list_files",
  "List files in a directory",
  {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory path" },
    },
    required: ["path"],
  },
  async (args) => {
    const abs = guardPath(args.path as string);
    const entries = await readdir(abs);
    return entries.join("\n");
  },
);
