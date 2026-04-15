import { defineTool } from "@mariozechner/pi-coding-agent";
import { Type } from "@sinclair/typebox";

export const escalateTool = defineTool({
  name: "escalate",
  label: "Escalate",
  description: "Flag task as too complex. Provide reason.",
  parameters: Type.Object({
    reason: Type.String({ description: "Why this needs a larger model" }),
  }),
  execute: async (_id, args) => {
    return {
      content: [{ type: "text", text: `ESCALATION_REQUESTED: ${args.reason}` }],
      details: {},
    };
  },
});
