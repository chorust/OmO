# wake-parent (pi extension)

Posts a JSON webhook so Grok Bot (parent) wakes only when pi needs attention or finishes — no polling.

## Load

Pi auto-loads project extensions from `.pi/extensions/*.ts` after project trust.

A copy may also live at `~/.pi/agent/extensions/wake-parent.ts` (user-global). Same env gate applies.

## Enable on lizhan-spark

1. In Grok Bot, open the routine / inbound webhook panel for **「pi wake from spark」** (or equivalent).
2. Copy the **Webhook URL** and **sender key**.
3. Export before starting pi (shell profile, direnv, or a wrapper):

```bash
export PI_WAKE_PARENT=1
export PI_WAKE_WEBHOOK_URL='https://…'   # from routine panel
export PI_WAKE_WEBHOOK_KEY='…'           # sender key from routine panel
# optional:
# export PI_WAKE_WEBHOOK_HEADER='X-Webhook-Key'  # override auth header name
# export PI_WAKE_ON_SETTLE=1                     # also wake on every agent_settled (interactive)
# export PI_WAKE_ON_SHUTDOWN=1                   # wake on session_shutdown
# export PI_WAKE_HOST=lizhan-spark               # override host field (default: lizhan-spark)
```

Or copy `.pi/env.wake-parent.example` → something like `.pi/env.wake-parent.local` (do **not** commit secrets) and `set -a; source .pi/env.wake-parent.local; set +a`.

## Auth header choice

| Env | Behavior |
|-----|----------|
| `PI_WAKE_WEBHOOK_KEY` unset | No auth header |
| `PI_WAKE_WEBHOOK_KEY` set, `PI_WAKE_WEBHOOK_HEADER` unset | `Authorization: Bearer <key>` (**default**) |
| both set | Header name = `PI_WAKE_WEBHOOK_HEADER`, value = key (e.g. `X-Webhook-Key`) |

Match whatever the Grok Bot routine panel documents. Default Bearer is the usual Cursor/Grok inbound webhook pattern.

## When it POSTs

Master switch: `PI_WAKE_PARENT=1` **and** `PI_WAKE_WEBHOOK_URL` set.

| Event | `reason` | Gate |
|-------|----------|------|
| `tool_call` `ask_user_question` | `ask_user` | always (when enabled) |
| `tool_call` `goal_blocked` | `blocked` | always |
| `tool_call` `goal_complete` | `goal_complete` | always |
| `agent_settled` | `agent_settled` | only if `PI_WAKE_ON_SETTLE=1` **or** non-interactive (`!hasUI` / `-p` / `--print`) |
| `session_shutdown` | `session_shutdown` | if `PI_WAKE_ON_SHUTDOWN=1`, or non-interactive |

## Payload

```json
{
  "source": "pi",
  "host": "lizhan-spark",
  "cwd": "/home/blizhan/repo/github/duckling",
  "session": "<session-id-or-null>",
  "reason": "goal_complete",
  "aborted": null,
  "summary": "…",
  "ts": "2026-10-30T15:12:00.000Z",
  "toolName": "goal_complete"
}
```

Failures are logged to stderr as `[wake-parent] …` and never block the agent turn.
