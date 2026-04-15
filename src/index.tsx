import {
  AuthStorage,
  ModelRegistry,
  SessionManager,
  SettingsManager,
  DefaultResourceLoader,
  codingTools,
  createAgentSessionRuntime,
  type CreateAgentSessionRuntimeFactory,
  createAgentSessionServices,
  createAgentSessionFromServices,
  getAgentDir,
  InteractiveMode,
  runPrintMode,
} from "@mariozechner/pi-coding-agent";
import type { Model } from "@mariozechner/pi-ai";
import { escalateTool } from "./tools/escalate.ts";
import { tmuxTools } from "./tools/tmux.ts";
import { SYSTEM_PROMPT, LLM_BASE_URL, MAX_TOKENS, ENABLE_THINKING } from "./config.ts";

const authStorage = AuthStorage.inMemory();
authStorage.setRuntimeApiKey("openai", "sk-dummy"); // local LLM dummy key
const modelRegistry = ModelRegistry.inMemory(authStorage);

const localModel: Model<string> = {
  id: "local",
  name: "Local Model",
  api: "openai-completions",
  provider: "openai",
  baseUrl: `${LLM_BASE_URL}/v1`, // e.g., "http://localhost:30434/v1"
  reasoning: ENABLE_THINKING,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 16384,
  maxTokens: MAX_TOKENS,
};

const settingsManager = SettingsManager.inMemory({
  compaction: { enabled: true },
});

const createRuntime: CreateAgentSessionRuntimeFactory = async ({ cwd, sessionManager, sessionStartEvent }) => {
  // Create resource loader that overrides system prompt
  const resourceLoader = new DefaultResourceLoader({
    cwd,
    settingsManager,
    systemPromptOverride: () => SYSTEM_PROMPT,
  });
  await resourceLoader.reload();

  const services = await createAgentSessionServices({
    cwd,
    settingsManager,
    authStorage,
    modelRegistry,
    resourceLoaderOptions: {
      systemPromptOverride: () => SYSTEM_PROMPT,
    }
  });
  
  return {
    ...(await createAgentSessionFromServices({
      services: { ...services, resourceLoader }, // explicitly inject our custom loader
      sessionManager,
      sessionStartEvent,
      model: localModel,
      scopedModels: [{ model: localModel, thinkingLevel: ENABLE_THINKING ? "high" : "off" }],
      thinkingLevel: ENABLE_THINKING ? "high" : "off",
      tools: codingTools,
      customTools: [escalateTool, ...tmuxTools],
    })),
    services,
    diagnostics: services.diagnostics,
  };
};

const runtime = await createAgentSessionRuntime(createRuntime, {
  cwd: process.cwd(),
  agentDir: getAgentDir(),
  sessionManager: SessionManager.inMemory(),
});

const args = process.argv.slice(2);
if (args.length > 0) {
  const message = args.join(" ");
  await runPrintMode(runtime, {
    mode: "text",
    initialMessage: message,
    initialImages: [],
    messages: [],
  });
  process.exit(0);
} else {
  // REPL mode using InteractiveMode
  const mode = new InteractiveMode(runtime, {
    migratedProviders: [],
    modelFallbackMessage: undefined,
  });
  await mode.run();
  process.exit(0);
}
