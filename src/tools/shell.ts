import { registry } from "../registry.ts";
import { ALLOWED_COMMANDS, ALLOWED_PATH_PREFIXES } from "../config.ts";

const SHELL_METACHARACTERS = /[|`&;$><\n\\(){}!#]/;

registry.register(
  "shell",
  "Run a shell command. Allowed: ls, cat, echo, find, wc, head, tail, grep, mkdir, cp, mv, git, which, pwd, date",
  {
    type: "object",
    properties: {
      command: { type: "string", description: "Command to run" },
    },
    required: ["command"],
  },
  async (args) => {
    const command = args.command as string;

    if (SHELL_METACHARACTERS.test(command)) {
      throw new Error("Shell metacharacters are not allowed");
    }

    const parts = command.trim().split(/\s+/);
    const cmd = parts[0];
    const cmdArgs = parts.slice(1);

    if (!ALLOWED_COMMANDS.includes(cmd)) {
      throw new Error(`Command not allowed: ${cmd}`);
    }

    const proc = Bun.spawn([cmd, ...cmdArgs], {
      cwd: ALLOWED_PATH_PREFIXES[0],
      stdout: "pipe",
      stderr: "pipe",
    });

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
      return `Error: command timed out after ${timeoutMs / 1000}s`;
    }

    return (stdout + stderr).trim();
  },
);
