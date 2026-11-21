/**
 * wake-parent — POST a webhook when pi needs parent/user attention or finishes a goal.
 *
 * Enable with PI_WAKE_PARENT=1 and PI_WAKE_WEBHOOK_URL=<url>.
 * Optional auth: PI_WAKE_WEBHOOK_KEY (+ optional PI_WAKE_WEBHOOK_HEADER).
 * See .pi/extensions/README-wake-parent.md
 */

const POST_TIMEOUT_MS = 2500;
const SUMMARY_MAX = 1500;

type WakeReason =
  | "ask_user"
  | "blocked"
  | "goal_complete"
  | "agent_settled"
  | "session_shutdown";

type WakePayload = {
  source: "pi";
  host: string;
  cwd: string;
  session: string | null;
  reason: WakeReason;
  aborted: boolean | null;
  summary: string | null;
  ts: string;
  toolName?: string;
  shutdownReason?: string;
};

function truthy(v: string | undefined): boolean {
  if (!v) return false;
  const s = v.trim().toLowerCase();
  return s === "1" || s === "true" || s === "yes" || s === "on";
}

function enabled(): boolean {
  return truthy(process.env.PI_WAKE_PARENT);
}

function webhookUrl(): string | null {
  const url = process.env.PI_WAKE_WEBHOOK_URL?.trim();
  return url || null;
}

function isNonInteractive(ctx: { hasUI?: boolean } | undefined): boolean {
  if (ctx && typeof ctx.hasUI === "boolean" && !ctx.hasUI) return true;
  const argv = process.argv.slice(2);
  if (argv.includes("-p") || argv.includes("--print")) return true;
  if (process.env.PI_WAKE_NONINTERACTIVE && truthy(process.env.PI_WAKE_NONINTERACTIVE)) return true;
  return false;
}

function sessionId(ctx: { sessionManager?: { getSessionId?: () => unknown } } | undefined): string | null {
  try {
    const fromEnv = process.env.PI_SESSION_ID?.trim();
    if (fromEnv) return fromEnv;
    const id = ctx?.sessionManager?.getSessionId?.();
    return typeof id === "string" && id ? id : null;
  } catch {
    return null;
  }
}

function truncate(text: string | null | undefined): string | null {
  if (!text) return null;
  const t = text.trim();
  if (!t) return null;
  return t.length > SUMMARY_MAX ? t.slice(0, SUMMARY_MAX - 1) + "…" : t;
}

function summaryFromInput(input: Record<string, unknown> | undefined, reason: WakeReason): string | null {
  if (!input || typeof input !== "object") return null;
  if (typeof input.summary === "string") return truncate(input.summary);
  if (typeof input.reason === "string") return truncate(input.reason);
  if (typeof input.evidence === "string") return truncate(input.evidence);
  if (reason === "ask_user" && Array.isArray(input.questions)) {
    try {
      const titles = input.questions
        .map((q) => (q && typeof q === "object" && "question" in q ? String((q as { question: unknown }).question) : null))
        .filter(Boolean);
      return truncate(titles.join(" | "));
    } catch {
      return null;
    }
  }
  return null;
}

function authHeaders(): Record<string, string> {
  const key = process.env.PI_WAKE_WEBHOOK_KEY?.trim();
  if (!key) return {};
  const custom = process.env.PI_WAKE_WEBHOOK_HEADER?.trim();
  if (custom) {
    return { [custom]: key };
  }
  // Default: Authorization Bearer (common for Cursor / Grok Bot webhook routines)
  return { Authorization: `Bearer ${key}` };
}

async function postWake(payload: WakePayload): Promise<void> {
  const url = webhookUrl();
  if (!url) return;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), POST_TIMEOUT_MS);
  if (typeof (timer as NodeJS.Timeout).unref === "function") (timer as NodeJS.Timeout).unref();

  try {
    const res = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...authHeaders(),
      },
      body: JSON.stringify(payload),
      signal: controller.signal,
    });
    if (!res.ok) {
      console.warn(`[wake-parent] webhook HTTP ${res.status} for reason=${payload.reason}`);
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.warn(`[wake-parent] webhook failed (${payload.reason}): ${msg}`);
  } finally {
    clearTimeout(timer);
  }
}

function basePayload(
  ctx: { cwd?: string; sessionManager?: { getSessionId?: () => unknown } } | undefined,
  reason: WakeReason,
  extra: Partial<WakePayload> = {},
): WakePayload {
  return {
    source: "pi",
    host: process.env.PI_WAKE_HOST?.trim() || "lizhan-spark",
    cwd: typeof ctx?.cwd === "string" ? ctx.cwd : process.cwd(),
    session: sessionId(ctx),
    reason,
    aborted: null,
    summary: null,
    ts: new Date().toISOString(),
    ...extra,
  };
}

export default function (pi: {
  on: (event: string, handler: (event: unknown, ctx: unknown) => unknown) => unknown;
}): void {
  // Do not activate without the master switch; keep factory side-effect free otherwise.
  if (!enabled()) return;
  if (!webhookUrl()) {
    console.warn("[wake-parent] PI_WAKE_PARENT=1 but PI_WAKE_WEBHOOK_URL is empty; extension idle");
    return;
  }

  pi.on("tool_call", (event, ctx) => {
    const e = event as { toolName?: string; input?: Record<string, unknown> };
    const name = e?.toolName;
    if (!name) return;

    let reason: WakeReason | null = null;
    if (name === "ask_user_question") reason = "ask_user";
    else if (name === "goal_blocked") reason = "blocked";
    else if (name === "goal_complete") reason = "goal_complete";
    if (!reason) return;

    const payload = basePayload(ctx as { cwd?: string; sessionManager?: { getSessionId?: () => unknown } }, reason, {
      toolName: name,
      summary: summaryFromInput(e.input, reason),
    });
    // Fire without blocking the tool pipeline on slow networks
    void postWake(payload);
  });

  pi.on("agent_settled", (event, ctx) => {
    const onSettle = truthy(process.env.PI_WAKE_ON_SETTLE);
    const nonInteractive = isNonInteractive(ctx as { hasUI?: boolean });
    if (!onSettle && !nonInteractive) return;

    const e = event as { aborted?: boolean };
    const payload = basePayload(ctx as { cwd?: string; sessionManager?: { getSessionId?: () => unknown } }, "agent_settled", {
      aborted: typeof e?.aborted === "boolean" ? e.aborted : null,
      summary: e?.aborted ? "agent settled (aborted)" : "agent settled",
    });
    void postWake(payload);
  });

  pi.on("session_shutdown", (event, ctx) => {
    // Optional: PI_WAKE_ON_SHUTDOWN=1, or any non-interactive session
    if (!truthy(process.env.PI_WAKE_ON_SHUTDOWN) && !isNonInteractive(ctx as { hasUI?: boolean })) {
      return;
    }

    const e = event as { reason?: string };
    const payload = basePayload(ctx as { cwd?: string; sessionManager?: { getSessionId?: () => unknown } }, "session_shutdown", {
      shutdownReason: typeof e?.reason === "string" ? e.reason : undefined,
      summary: `session_shutdown:${e?.reason ?? "unknown"}`,
    });
    void postWake(payload);
  });
}
