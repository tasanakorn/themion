import { defineTool } from "@mariozechner/pi-coding-agent";
import { Type } from "@sinclair/typebox";

const DANGEROUS_CHARS = /[|`&;$><\n\\(){}!#]/;
const TEXT_MAX_LENGTH = 4096;

function validateTarget(target: string): string {
  if (!target || typeof target !== "string" || target.trim().length === 0) {
    throw new Error("target is required and must be non-empty");
  }
  const t = target.trim();
  if (DANGEROUS_CHARS.test(t)) {
    throw new Error("target contains disallowed characters");
  }
  return t;
}

async function runTmux(args: string[]): Promise<string> {
  let proc: ReturnType<typeof Bun.spawn>;
  try {
    proc = Bun.spawn(["tmux", ...args], {
      stdout: "pipe",
      stderr: "pipe",
    });
  } catch {
    throw new Error("Failed to spawn tmux. Is tmux installed and on PATH?");
  }

  const timeoutMs = 10_000;
  let timedOut = false;

  const timeoutHandle = setTimeout(() => {
    timedOut = true;
    proc.kill();
  }, timeoutMs);

  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout as ReadableStream).text(),
    new Response(proc.stderr as ReadableStream).text(),
  ]);

  await proc.exited;
  clearTimeout(timeoutHandle);

  if (timedOut) {
    throw new Error(`tmux command timed out after ${timeoutMs / 1000}s`);
  }

  if (proc.exitCode !== 0) {
    const msg = stderr.trim() || stdout.trim();
    throw new Error(`tmux exited with code ${proc.exitCode}: ${msg}`);
  }

  return stdout;
}

export const tmuxListTool = defineTool({
  name: "tmux_list",
  label: "Tmux List",
  description: "List all tmux sessions, windows, and panes as a Session > Window > Pane hierarchy",
  parameters: Type.Object({}),
  execute: async () => {
    const FMT = [
      "#{session_name}",
      "#{window_index}",
      "#{window_name}",
      "#{window_panes}",
      "#{window_layout}",
      "#{pane_index}",
      "#{pane_current_command}",
      "#{pane_width}x#{pane_height}",
      "#{pane_left},#{pane_top}",
      "#{?pane_active,1,0}",
      "#{?window_zoomed_flag,1,0}",
    ].join("\t");

    const output = await runTmux(["list-panes", "-a", "-F", FMT]);
    if (!output.trim()) return { content: [{ type: "text", text: "No tmux panes found." }], details: {} };

    type Pane = { idx: string; cmd: string; size: string; pos: string; active: boolean };
    type Window = { idx: string; name: string; panes: string; layout: string; zoomed: boolean; paneList: Pane[] };
    type Session = { name: string; windows: Map<string, Window> };

    const sessions = new Map<string, Session>();

    for (const line of output.split("\n")) {
      if (!line.trim()) continue;
      const parts = line.split("\t");
      if (parts.length < 11) continue;
      const [sName, wIdx, wName, wPanes, wLayout, pIdx, pCmd, pSize, pPos, pActive, wZoomed] = parts;

      let session = sessions.get(sName);
      if (!session) {
        session = { name: sName, windows: new Map() };
        sessions.set(sName, session);
      }

      let window = session.windows.get(wIdx);
      if (!window) {
        window = { idx: wIdx, name: wName, panes: wPanes, layout: wLayout, zoomed: wZoomed === "1", paneList: [] };
        session.windows.set(wIdx, window);
      }

      window.paneList.push({ idx: pIdx, cmd: pCmd, size: pSize, pos: pPos, active: pActive === "1" });
    }

    const lines: string[] = [];
    for (const session of sessions.values()) {
      lines.push(`Session ${session.name}`);
      for (const window of session.windows.values()) {
        const zoom = window.zoomed ? " zoomed" : "";
        const winTarget = `${session.name}:${window.idx}`;
        lines.push(`  Window ${winTarget} [${window.name}] ${window.panes} panes${zoom} layout=${window.layout}`);
        for (const pane of window.paneList) {
          const active = pane.active ? " active" : "";
          const paneTarget = `${session.name}:${window.idx}.${pane.idx}`;
          lines.push(`    Pane ${paneTarget} [${pane.cmd}] ${pane.size} at (${pane.pos})${active}`);
        }
      }
    }

    return { content: [{ type: "text", text: lines.join("\n") }], details: {} };
  },
});

export const tmuxCaptureTool = defineTool({
  name: "tmux_capture",
  label: "Tmux Capture",
  description: "Capture visible content of a tmux pane",
  parameters: Type.Object({
    target: Type.String({ description: "Pane target, e.g. 'mysession:0.0'" }),
    lines: Type.Optional(Type.Number({ description: "Number of scrollback lines to capture. Omit for visible area only." })),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    const tmuxArgs = ["capture-pane", "-t", target, "-p"];

    if (args.lines !== undefined) {
      if (!Number.isInteger(args.lines) || args.lines < 1) {
        throw new Error("lines must be a positive integer");
      }
      tmuxArgs.push("-S", `-${args.lines}`);
    }

    const output = await runTmux(tmuxArgs);
    return { content: [{ type: "text", text: output }], details: {} };
  },
});

export const tmuxSendKeysTool = defineTool({
  name: "tmux_send_keys",
  label: "Tmux Send Keys",
  description: "Send special keys to a tmux pane (Enter, C-c, Escape, Up, Down, etc.)",
  parameters: Type.Object({
    target: Type.String({ description: "Pane target, e.g. 'mysession:0.0'" }),
    keys: Type.String({ description: "Keys to send, e.g. 'Enter', 'C-c', 'Escape'" }),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    await runTmux(["send-keys", "-t", target, args.keys]);
    return { content: [{ type: "text", text: `Sent keys to ${target}` }], details: {} };
  },
});

export const tmuxSendTextTool = defineTool({
  name: "tmux_send_text",
  label: "Tmux Send Text",
  description: "Type text into a tmux pane, optionally pressing Enter afterwards",
  parameters: Type.Object({
    target: Type.String({ description: "Pane target, e.g. 'mysession:0.0'" }),
    text: Type.String({ description: "Text to type into the pane" }),
    enter: Type.Optional(Type.Boolean({ description: "Press Enter after typing (default: true)" })),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    if (args.text.length > TEXT_MAX_LENGTH) {
      throw new Error(`text exceeds maximum length of ${TEXT_MAX_LENGTH} characters`);
    }

    await runTmux(["send-keys", "-t", target, "--", args.text]);

    const pressEnter = args.enter !== false;
    if (pressEnter) {
      await runTmux(["send-keys", "-t", target, "Enter"]);
    }

    return { content: [{ type: "text", text: `Sent text to ${target}${pressEnter ? " (Enter)" : ""}` }], details: {} };
  },
});

export const tmuxSplitPaneTool = defineTool({
  name: "tmux_split_pane",
  label: "Tmux Split Pane",
  description: "Split a tmux pane horizontally (side-by-side) or vertically (top/bottom). Returns new pane target.",
  parameters: Type.Object({
    target: Type.String({ description: "Pane to split, e.g. 'mysession:0.0'" }),
    direction: Type.Optional(Type.Union([Type.Literal("horizontal"), Type.Literal("vertical")], { description: "horizontal = side-by-side, vertical = top/bottom (default: vertical)" })),
    command: Type.Optional(Type.String({ description: "Optional command to run in the new pane" })),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    const direction = args.direction ?? "vertical";
    const flag = direction === "horizontal" ? "-h" : "-v";

    const tmuxArgs = ["split-window", flag, "-t", target, "-P", "-F", "#{session_name}:#{window_index}.#{pane_index}"];
    if (args.command !== undefined) {
      if (args.command.length > TEXT_MAX_LENGTH) {
        throw new Error(`command exceeds maximum length of ${TEXT_MAX_LENGTH} characters`);
      }
      tmuxArgs.push(args.command);
    }

    const output = await runTmux(tmuxArgs);
    return { content: [{ type: "text", text: `Created pane: ${output.trim()}` }], details: {} };
  },
});

export const tmuxKillPaneTool = defineTool({
  name: "tmux_kill_pane",
  label: "Tmux Kill Pane",
  description: "Kill a tmux pane",
  parameters: Type.Object({
    target: Type.String({ description: "Pane target to kill, e.g. 'mysession:0.1'" }),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    await runTmux(["kill-pane", "-t", target]);
    return { content: [{ type: "text", text: `Killed pane: ${target}` }], details: {} };
  },
});

export const tmuxSelectLayoutTool = defineTool({
  name: "tmux_select_layout",
  label: "Tmux Select Layout",
  description: "Apply a preset layout to a tmux window's split panes",
  parameters: Type.Object({
    target: Type.String({ description: "Window target, e.g. 'mysession:0'" }),
    layout: Type.Union([
      Type.Literal("even-horizontal"),
      Type.Literal("even-vertical"),
      Type.Literal("main-horizontal"),
      Type.Literal("main-vertical"),
      Type.Literal("tiled")
    ], { description: "Layout preset" }),
  }),
  execute: async (_id, args) => {
    const target = validateTarget(args.target);
    await runTmux(["select-layout", "-t", target, args.layout]);
    return { content: [{ type: "text", text: `Applied ${args.layout} layout to ${target}` }], details: {} };
  },
});

export const tmuxTools = [
  tmuxListTool,
  tmuxCaptureTool,
  tmuxSendKeysTool,
  tmuxSendTextTool,
  tmuxSplitPaneTool,
  tmuxKillPaneTool,
  tmuxSelectLayoutTool
];