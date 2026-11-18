// Limux workspace card for oh-my-pi (omp).
//
// Install: copy to ~/.omp/agent/extensions/limux-card.ts (or under $OMP_AGENT_DIR).
// omp loads it at startup. It does nothing outside a Limux terminal.
//
// What it does, inside Limux:
//   - prompt submitted        -> status row `omp`: Running
//   - turn finished           -> status row `omp`: Idle, plus a notification with
//                                the start of omp's last reply, and a background
//                                `limux refresh-pr` for the PR row
//   - session start           -> Idle
//   - session shutdown        -> the `omp` row is removed
// Turns omp continues by itself (retries, queued messages) stay Running.
//
// Opt out per process with LIMUX_OMP_CARD_DISABLED=1. Every call has a short
// timeout and its errors are ignored, so the card can never block or fail a turn.
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import type { AgentEndEvent, ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

const CALL_TIMEOUT_MS = 1500;
const KEY = "omp";

function enabled(): boolean {
  return process.env.LIMUX_OMP_CARD_DISABLED !== "1" && !!process.env.LIMUX_WORKSPACE_ID;
}

function firstString(...values: unknown[]): string | null {
  for (const value of values) {
    if (typeof value === "string" && value.trim() !== "") return value;
  }
  return null;
}

// A subagent's session lives in a folder next to its parent's session file;
// only the session the user drives should move the card.
function isNestedSession(ctx: ExtensionContext): boolean {
  const sessionFile = firstString(ctx.sessionManager.getSessionFile());
  return sessionFile !== null && fs.existsSync(`${path.dirname(sessionFile)}.jsonl`);
}

function textFromContent(content: unknown): string | null {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return null;
  const parts: string[] = [];
  for (const block of content) {
    if (!block || typeof block !== "object") continue;
    const typed = block as { type?: unknown; text?: unknown };
    if (typed.type === "text" && typeof typed.text === "string") parts.push(typed.text);
  }
  return parts.join("\n") || null;
}

function lastAssistantText(event: AgentEndEvent): string | null {
  const messages = (event as { messages?: unknown[] }).messages ?? [];
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as { role?: unknown; content?: unknown } | undefined;
    if (!message || message.role !== "assistant") continue;
    const text = firstString(textFromContent(message.content));
    if (text) return text;
  }
  return null;
}

// One line, short enough for a card.
function oneLine(text: string, max = 140): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length <= max ? line : `${line.slice(0, max - 1)}…`;
}

// Run `limux <args>` and wait at most CALL_TIMEOUT_MS; never throws.
function limux(args: string[], detach = false): Promise<void> {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(process.env.LIMUX_OMP_LIMUX_BIN || "limux", args, {
        stdio: "ignore",
        detached: detach,
      });
    } catch {
      resolve();
      return;
    }
    if (detach) {
      child.unref();
      resolve();
      return;
    }
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {}
      resolve();
    }, CALL_TIMEOUT_MS);
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    child.on("error", done);
    child.on("close", done);
  });
}

function setStatus(value: string, icon: string, color: string): Promise<void> {
  return limux(["set-status", KEY, value, "--icon", icon, "--color", color, "--priority", "10"]);
}

export default function limuxCardExtension(api: ExtensionAPI) {
  api.on("session_start", async (_event, ctx) => {
    if (!enabled() || isNestedSession(ctx)) return;
    await setStatus("Idle", "pause.circle.fill", "#8d8d8d");
  });

  api.on("before_agent_start", async (_event, ctx) => {
    if (!enabled() || isNestedSession(ctx)) return;
    await setStatus("Running", "bolt.fill", "#5c9cf5");
  });

  api.on("agent_end", async (event, ctx) => {
    if (!enabled() || isNestedSession(ctx)) return;
    // omp ends a turn it is about to continue by itself (auto-retry, queued
    // messages, background jobs) with willContinue; it is still running then.
    if ((event as { willContinue?: unknown }).willContinue === true) return;
    await setStatus("Idle", "pause.circle.fill", "#8d8d8d");
    const reply = lastAssistantText(event);
    await limux(["notify", "--body", oneLine(reply ?? "turn complete"), "omp finished"]);
    const cwd = firstString(ctx.cwd, process.cwd()) ?? process.cwd();
    await limux(["refresh-pr", "--cwd", cwd], true);
  });

  api.on("session_shutdown", async (_event, ctx) => {
    if (!enabled() || isNestedSession(ctx)) return;
    await limux(["clear-status", KEY]);
  });
}
