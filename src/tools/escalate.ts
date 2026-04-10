import { registry } from "../registry.ts";

registry.register(
  "escalate",
  "Flag task as too complex. Provide reason.",
  {
    type: "object",
    properties: {
      reason: { type: "string", description: "Why this needs a larger model" },
    },
    required: ["reason"],
  },
  async (args) => {
    return `ESCALATION_REQUESTED: ${args.reason as string}`;
  },
);
