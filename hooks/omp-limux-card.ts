// Limux workspace card for oh-my-pi (omp).
//
// Install: limux hooks setup omp. omp loads this extension at startup.
// Inside a Limux terminal, remember the exact interactive session for this tab.
//
// What it does, inside Limux, when a turn finishes: a notification with the
// start of omp's last reply, and a background `limux refresh-pr` for the PR
// row. Turns omp continues by itself (retries, queued messages) are skipped.
//
// omp's state (working, done, waiting on you, failed) needs nothing from this
// file: omp reports it over OSC 7501 and Limux shows it as the card's
// `omp · Working` row, the way cmux does.
//
// Opt out of restore tracking with LIMUX_OMP_HOOKS_DISABLED=1, or of turn
// notifications with LIMUX_OMP_CARD_DISABLED=1. Calls are time-bounded.
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import type { AgentEndEvent, ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

const CALL_TIMEOUT_MS = 1500;

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
function limux(
  args: string[],
  { detach = false, input }: { detach?: boolean; input?: string } = {},
): Promise<void> {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(process.env.LIMUX_OMP_LIMUX_BIN || "limux", args, {
        stdio: [input === undefined ? "ignore" : "pipe", "ignore", "ignore"],
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
    if (input !== undefined) {
      child.stdin?.on("error", () => {});
      child.stdin?.end(input);
    }
  });
}

export default function limuxCardExtension(api: ExtensionAPI) {
  const rememberSession = async (_event: unknown, ctx: ExtensionContext) => {
    if (
      process.env.LIMUX_OMP_HOOKS_DISABLED === "1" ||
      !process.env.LIMUX_WORKSPACE_ID ||
      !process.env.LIMUX_SURFACE_ID ||
      !ctx.hasUI ||
      !process.stdin.isTTY ||
      isNestedSession(ctx)
    ) return;
    const file = ctx.sessionManager.getSessionFile();
    if (!file) return;
    // A new session may not have written the file yet. Recording its path now
    // replaces the old binding; Limux resumes only materialized transcripts.
    await limux(["hooks", "omp", "restore-session"], {
      input: JSON.stringify({
        session_id: path.resolve(ctx.cwd, file),
        cwd: ctx.cwd,
        pid: String(process.pid),
      }),
    });
  };
  api.on("session_start", rememberSession);
  api.on("session_switch", rememberSession);

  api.on("agent_end", async (event, ctx) => {
    if (!enabled() || isNestedSession(ctx)) return;
    // omp ends a turn it is about to continue by itself (auto-retry, queued
    // messages, background jobs) with willContinue; it is still running then.
    if ((event as { willContinue?: unknown }).willContinue === true) return;
    const reply = lastAssistantText(event);
    await limux([
      "notify", "--no-sidebar-preview", "--body", oneLine(reply ?? "turn complete"), "omp finished",
    ]);
    const cwd = firstString(ctx.cwd, process.cwd()) ?? process.cwd();
    await limux(["refresh-pr", "--cwd", cwd], { detach: true });
  });
}
