import { resolve } from "node:path";
import { ALLOWED_PATH_PREFIXES } from "./config.ts";

export function guardPath(rawPath: string): string {
  const abs = resolve(rawPath);
  if (!ALLOWED_PATH_PREFIXES.some((p) => abs === p || abs.startsWith(p + "/"))) {
    throw new Error(`Path not allowed: ${abs}`);
  }
  return abs;
}
