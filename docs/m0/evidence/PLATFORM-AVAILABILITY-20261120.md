# Host availability check — 2026-11-20

This records why cross-platform host/session work remains `not-run`. Dependency trees and cross-compilation metadata do not substitute for builds or GUI acceptance on the target hosts.

## macOS 14 arm64

- The latest Tailscale status check on 2026-11-20 lists `nvsync-macbook-pro-6` as offline (`Online=false`), last seen `2026-11-20T16:10:00.1Z`. All other visible macOS peers were also offline, including entries last seen on 2026-11-19 and 2026-11-16.
- Fresh `tailscale ping --timeout=5s nvsync-macbook-pro-6`, `ssh -o BatchMode=yes -o ConnectTimeout=5 nvsync-macbook-pro-6 true`, and `tailscale ssh blizhan@100.71.203.93 true` timed out. Earlier TCP checks to ports 22 and 5900 also timed out.
- No macOS build, N1–N8 run, or EXIT run was produced.

## Linux x86_64

- Tailscale listed one Linux peer and zero online Linux peers. The only potentially relevant host, `abc`, maps to an offline peer last seen `2026-08-20T05:06:18.1Z`; its node key has expired. Its architecture could not be verified while offline.
- A fresh `tailscale ping --timeout=5s abc` reported `peer's node key has expired`; `ssh -o BatchMode=yes -o ConnectTimeout=5 abc true` timed out.
- A fresh lookup and SSH attempt for `lizhan-linux-x86_64` failed with temporary hostname-resolution errors; `ssh abc` timed out. Earlier SSH to `abc`'s configured tailnet address also timed out.
- No Linux x86_64 host build, N1–N8 run, or EXIT run was produced.

These are host-access checks from Spark, not claims that the peer machines are permanently unavailable. Keep their acceptance cells `not-run` until a real target host and session can be reached.
