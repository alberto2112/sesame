# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`sesame` is a Rust web app that **gates kids' access to the computer** (Manjaro + KDE Plasma 5 + X11). The kid boots, faces a quiz in a kiosk window; passing grants N minutes of free desktop use, bounded by a daily time budget. Formerly this project was a wrapper that gated only the Luanti game — the pivot to gating the whole session is in progress (see Roadmap).

**All user-facing strings — UI, CLI help, errors — are in French.** Code comments are mixed FR/ES/EN; match the surrounding file.

## Commands

```bash
cargo check                      # primary feedback loop
cargo run -- --help              # CLI help (French)
cargo run                        # start server (opens browser; /admin if no password set, else /)
cargo run -- admin               # start server, force-open /admin
cargo run -- import data/questions_mathematiques.json
cargo run -- preview 10          # console simulation of a quiz + grading
SESAME_NO_BROWSER=1 cargo run    # suppress browser auto-open (kiosk mode will rely on this)
```

There are no tests in the repo. Do not run builds unless asked.

## Configuration split — this trips people up

- **`config.toml`** (static, requires restart): `server.host`/`server.port`, `paths.database`. Lookup order: `$XDG_CONFIG_HOME/sesame/config.toml`, then `./config.toml`. A relative `paths.database` resolves against `$XDG_DATA_HOME/sesame/`, **not** the CWD.
- **SQLite** (dynamic, live-editable from `/admin`): global defaults in the `settings` table (`questions_per_test`, `pass_threshold_pct`, `session_minutes`, `lock_mode`, `admin_password_hash`) and **per-child settings in `children`** (difficulty range, budgets, session length). Per-child wins; new tunable knobs belong in the DB, not in `config.toml`.

## Architecture

The core rule: **`policy::evaluate(pool, &child) -> GateDecision` is the single source of truth** for "can this kid use the computer right now?". Every surface (quiz page, `/api/status`, `/api/heartbeat`, future kiosk/lock/timer binaries) calls it; new conditions (schedules, cooldowns) get added inside `evaluate`, never as ad-hoc `if`s in handlers.

- **`policy.rs`** — `GateDecision::{Granted, ExamAvailable, Blocked}`. Key invariants:
  - Time is accounted as **consumed seconds in a per-day ledger** (`daily_usage`), never as expiry timestamps. Reboots don't reset it; changing the system clock doesn't mint minutes (clients measure monotonically, the server only adds).
  - `max_grant_minutes = min(session_minutes, remaining daily budget)` — the daily budget always wins (anti exam-farming). Weekends have a separate budget.
  - `consume()` clamps heartbeat increments to [0, 300] s.
- **`web.rs`** — public routes `/`, `/submit`, `/unlock`, `/api/status`, `/api/heartbeat`; `/admin` nested; `/static/*` embedded via rust-embed. The child is identified by the `sesame_child` cookie (falls back to first enabled child until the profile selector lands).
  - **The clock starts at `/unlock`, not `/submit`**: the kid reads corrections for free; the grant opens when they click the button. A passed attempt is redeemable **once** (partial unique index on `grants.attempt_id` + handler check).
- **`quiz.rs`** — `pick_questions` allocates proportionally to `subjects.weight` among `enabled` subjects (Hamilton/largest-remainder in `distribute()`). `grade(pool, submission, threshold_pct)` takes the threshold from the child. Multi-answer questions are all-or-nothing.
- **`admin.rs`** — admin panel; `AdminAuth` extractor validates the `sesame_admin` cookie against `admin_sessions`; no password set → redirect to `/admin/setup`.
- **`auth.rs`** — argon2 hash in `settings`, random session tokens.
- **`importer.rs`** — permissive JSON import; per-question errors collected, not fatal.
- **`db.rs`** — opens SQLite with `foreign_keys(true)`, runs `sqlx::migrate!` at startup.

## Roadmap (pivot in progress)

Done: phase 1 (policy engine, grants/usage ledger, `/unlock` + API, binary renamed `sesame`). Pending: profile selector + per-child difficulty filtering (2), schedules/cooldown in `evaluate` (3), renewal flow (4), admin CRUD for children + parent panic-grant button (5), X11 kiosk binary run pre-session without a WM (6), X11 overlay lock `sesame-lock` parent/child à la xsecurelock (7), heartbeat timer daemon (8), SDDM session + hardening + installer (9). Architecture decisions live in engram (`kidgate/fase-1-policy-engine`, `luanti-gate/pivot-session-gate`).

X11-without-WM gotchas for phases 6-7: `_NET_WM_STATE_FULLSCREEN` needs a WM — size the window to the screen at 0,0 instead; nobody assigns keyboard focus — use `XSetInputFocus`/`XGrabKeyboard`.

## Database

Schema in `migrations/*.sql`; add a new numbered file, never edit an applied one. Timestamps are `INTEGER` Unix epoch seconds; booleans `0/1` with `CHECK`; `daily_usage.day` is a **local** `YYYY-MM-DD` string. `attempt_answers` stores text snapshots deliberately — history survives question edits/deletes. Queries use the runtime sqlx API (`query_as` + `.bind()`), no `DATABASE_URL` needed.

## Templates (Askama 0.12)

`templates/` public + `templates/admin/`; stylesheets `static/quiz.css` (public) and `static/admin.css`. Askama 0.12 gotcha: `{% if %}` comparing a dereferenced binding against a tuple field inside a `{% match %}` arm fails to parse — precompute a bool in Rust instead.

## Handlers

Handler futures must be `Send`: never hold `rand::thread_rng()` across an `.await` — scope it in a block. Opaque "Handler not implemented" errors → add `#[axum::debug_handler]`.
