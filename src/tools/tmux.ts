import { registry } from "../registry.ts";

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
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
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

// ---------- tmux_list ----------

registry.register(
  "tmux_list",
  "List all tmux sessions, windows, and panes as a Session > Window > Pane hierarchy",
  {
    type: "object",
    properties: {},
    required: [],
  },
  async () => {
    // Single query: tab-delimited fields are safer than '|' since tmux names
    // can contain pipes. Order: session, win_idx, win_name, win_panes,
    // win_layout, pane_idx, pane_cmd, pane_size, pane_pos, pane_active,
    // win_zoomed.
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
    if (!output.trim()) return "No tmux panes found.";

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
        window = {
          idx: wIdx,
          name: wName,
          panes: wPanes,
          layout: wLayout,
          zoomed: wZoomed === "1",
          paneList: [],
        };
        session.windows.set(wIdx, window);
      }

      window.paneList.push({
        idx: pIdx,
        cmd: pCmd,
        size: pSize,
        pos: pPos,
        active: pActive === "1",
      });
    }

    // Pane identifiers are emitted in tmux target form (`session:window.pane`)
    // so they can be passed directly to tmux_capture / tmux_send_text / etc.
    // Windows use `session:window` form for the same reason.
    const lines: string[] = [];
    for (const session of sessions.values()) {
      lines.push(`Session ${session.name}`);
      for (const window of session.windows.values()) {
        const zoom = window.zoomed ? " zoomed" : "";
        const winTarget = `${session.name}:${window.idx}`;
        lines.push(
          `  Window ${winTarget} [${window.name}] ${window.panes} panes${zoom} layout=${window.layout}`,
        );
        for (const pane of window.paneList) {
          const active = pane.active ? " active" : "";
          const paneTarget = `${session.name}:${window.idx}.${pane.idx}`;
          lines.push(
            `    Pane ${paneTarget} [${pane.cmd}] ${pane.size} at (${pane.pos})${active}`,
          );
        }
      }
    }

    return lines.join("\n");
  },
);

// ---------- tmux_capture ----------

registry.register(
  "tmux_capture",
  "Capture visible content of a tmux pane",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Pane target, e.g. 'mysession:0.0'",
      },
      lines: {
        type: "number",
        description: "Number of scrollback lines to capture. Omit for visible area only.",
      },
    },
    required: ["target"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    const tmuxArgs = ["capture-pane", "-t", target, "-p"];

    if (args.lines !== undefined) {
      const lines = Number(args.lines);
      if (!Number.isInteger(lines) || lines < 1) {
        throw new Error("lines must be a positive integer");
      }
      tmuxArgs.push("-S", `-${lines}`);
    }

    return await runTmux(tmuxArgs);
  },
);

// ---------- tmux_send_keys ----------

registry.register(
  "tmux_send_keys",
  "Send special keys to a tmux pane (Enter, C-c, Escape, Up, Down, etc.)",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Pane target, e.g. 'mysession:0.0'",
      },
      keys: {
        type: "string",
        description: "Keys to send, e.g. 'Enter', 'C-c', 'Escape'",
      },
    },
    required: ["target", "keys"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    const keys = args.keys as string;
    if (!keys || typeof keys !== "string") {
      throw new Error("keys is required");
    }

    await runTmux(["send-keys", "-t", target, keys]);
    return `Sent keys to ${target}`;
  },
);

// ---------- tmux_send_text ----------

registry.register(
  "tmux_send_text",
  "Type text into a tmux pane, optionally pressing Enter afterwards",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Pane target, e.g. 'mysession:0.0'",
      },
      text: {
        type: "string",
        description: "Text to type into the pane",
      },
      enter: {
        type: "boolean",
        description: "Press Enter after typing (default: true)",
      },
    },
    required: ["target", "text"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    const text = args.text as string;
    if (typeof text !== "string") {
      throw new Error("text is required");
    }
    if (text.length > TEXT_MAX_LENGTH) {
      throw new Error(`text exceeds maximum length of ${TEXT_MAX_LENGTH} characters`);
    }

    await runTmux(["send-keys", "-t", target, "--", text]);

    const pressEnter = args.enter !== false;
    if (pressEnter) {
      await runTmux(["send-keys", "-t", target, "Enter"]);
    }

    return `Sent text to ${target}${pressEnter ? " (Enter)" : ""}`;
  },
);

// ---------- tmux_split_pane ----------

registry.register(
  "tmux_split_pane",
  "Split a tmux pane horizontally (side-by-side) or vertically (top/bottom). Returns new pane target.",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Pane to split, e.g. 'mysession:0.0'",
      },
      direction: {
        type: "string",
        enum: ["horizontal", "vertical"],
        description: "horizontal = side-by-side, vertical = top/bottom (default: vertical)",
      },
      command: {
        type: "string",
        description: "Optional command to run in the new pane",
      },
    },
    required: ["target"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    const direction = (args.direction as string) ?? "vertical";
    const flag = direction === "horizontal" ? "-h" : "-v";

    const tmuxArgs = [
      "split-window",
      flag,
      "-t",
      target,
      "-P",
      "-F",
      "#{session_name}:#{window_index}.#{pane_index}",
    ];

    if (args.command !== undefined) {
      const command = args.command as string;
      if (typeof command !== "string") {
        throw new Error("command must be a string");
      }
      if (command.length > TEXT_MAX_LENGTH) {
        throw new Error(`command exceeds maximum length of ${TEXT_MAX_LENGTH} characters`);
      }
      tmuxArgs.push(command);
    }

    const output = await runTmux(tmuxArgs);
    return `Created pane: ${output.trim()}`;
  },
);

// ---------- tmux_kill_pane ----------

registry.register(
  "tmux_kill_pane",
  "Kill a tmux pane",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Pane target to kill, e.g. 'mysession:0.1'",
      },
    },
    required: ["target"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    await runTmux(["kill-pane", "-t", target]);
    return `Killed pane: ${target}`;
  },
);

// ---------- tmux_select_layout ----------

registry.register(
  "tmux_select_layout",
  "Apply a preset layout to a tmux window's split panes",
  {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "Window target, e.g. 'mysession:0'",
      },
      layout: {
        type: "string",
        enum: ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"],
        description: "Layout preset",
      },
    },
    required: ["target", "layout"],
  },
  async (args) => {
    const target = validateTarget(args.target as string);
    const layout = args.layout as string;
    const validLayouts = ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"];
    if (!validLayouts.includes(layout)) {
      throw new Error(`Invalid layout. Must be one of: ${validLayouts.join(", ")}`);
    }
    await runTmux(["select-layout", "-t", target, layout]);
    return `Applied ${layout} layout to ${target}`;
  },
);
