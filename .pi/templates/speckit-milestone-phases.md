# Spec Kit milestone phases (Preludio on spark)

Update when orchestration policy changes. Do not commit unless asked.

## Models
| Phase | Model | Thinking |
|---|---|---|
| specify / plan / tasks | `magpie/codex/gpt-6.1-sol` **or** Claude Opus 5.5 | high / **medium** |
| implement | `magpie/codex/gpt-6-luna:max` **or** `magpie/worker` | **max** |
| code-review | astra-medium (`magpie/codex/gpt-6-astra:medium`) **or** Claude Opus 5.5 | **high** (Opus) |
| review-fix | luna:max **or** worker | **max** |

## Sequence
specify → (webhook) → implement → (webhook) → code-review → (webhook) → fix if needed → notify user.
No auto-commit. No auto next milestone. Wake-parent only; ignore session_shutdown:quit noise.

## Launch skeleton
```bash
cd /home/blizhan/repo/github/preludio
set -a; source .pi/env.wake-parent.local; set +a
pi --approve --no-extensions --extension .pi/extensions/wake-parent.ts \
  --model "$MODEL" --thinking "$THINKING" --name "preludio-<feature>-<phase>" \
  -p -- "$PROMPT"
```

## Hygiene
Probes/smokes: `--no-session` or archive under `~/.pi/agent/sessions-archive/`.
