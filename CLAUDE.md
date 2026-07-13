# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`sesame` is a Rust web app that **gates kids' access to the computer** (Manjaro + KDE Plasma 5 + X11). The kid boots, faces a quiz in a kiosk window; passing grants N minutes of free desktop use, bounded by a daily time budget. Formerly this project was a wrapper that gated only the Luanti game — the pivot to gating the whole session is in progress (see Roadmap).

**All user-facing strings — UI, CLI help, errors — are in French.** Code comments are mixed FR/ES/EN; match the surrounding file.

## Commands

```bash
cargo check                      # primary feedback loop
cargo run --bin sesame -- --help # CLI help (French)
cargo run --bin sesame           # start server (opens browser; /admin if no password set, else /)
cargo run --bin sesame -- admin  # start server, force-open /admin
cargo run --bin sesame -- import data/questions_mathematiques.json
cargo run --bin sesame -- preview 10   # console simulation of a quiz + grading
SESAME_NO_BROWSER=1 cargo run --bin sesame   # suppress browser auto-open
cargo run --bin sesame-kiosk     # the gate (needs the server running; Linux/X11 in practice)
```

There are no tests in the repo. Do not run builds unless asked.

## Binaries

The crate is a **lib + bins**. `src/lib.rs` exposes the modules; `main.rs` and `src/bin/*.rs` are thin.

- **`sesame`** — the server. The only owner of the SQLite file. Long-lived, **fixed port** (`config.toml`), so its clients know where to knock.
- **`sesame-kiosk`** — the gate. Runs in a bare X server with **no window manager**, before the desktop exists. Shows the quiz in a browser it supervises, polls `GET /api/gate`, and **exits 0** when someone has earned the computer. That exit code is the entire contract with the session script (`sesame-kiosk || exit 1; exec startplasma-x11`).

System binaries are **thin clients**: they never touch the DB, they call the API. One source of truth.

## Configuration split — this trips people up

- **`config.toml`** (static, requires restart): `server.host`/`server.port`, `paths.database`, `kiosk.browser`. Lookup order: `$XDG_CONFIG_HOME/sesame/config.toml`, then `./config.toml`. A relative `paths.database` resolves against `$XDG_DATA_HOME/sesame/`, **not** the CWD.
- **SQLite** (dynamic, live-editable from `/admin`): global defaults in the `settings` table (`questions_per_test`, `pass_threshold_pct`, `session_minutes`, `lock_mode`, `admin_password_hash`) and **per-child settings in `children`** (difficulty range, budgets, session length) plus `schedules`. Per-child wins; new tunable knobs belong in the DB, not in `config.toml`.

## Architecture

The core rule: **`policy::evaluate(pool, &child) -> GateDecision` is the single source of truth** for "can this kid use the computer right now?". Every surface (quiz page, `/api/status`, `/api/heartbeat`, future kiosk/lock/timer binaries) calls it; new conditions (schedules, cooldowns) get added inside `evaluate`, never as ad-hoc `if`s in handlers.

- **`policy.rs`** — `GateDecision::{Granted, ExamAvailable, Blocked}`. Key invariants:
  - Time is accounted as **consumed seconds in a per-day ledger** (`daily_usage`), never as expiry timestamps. Reboots don't reset it; changing the system clock doesn't mint minutes (clients measure monotonically, the server only adds).
  - `max_grant_minutes = min(session_minutes, remaining daily budget)` — the daily budget always wins (anti exam-farming). Weekends have a separate budget.
  - `consume()` clamps heartbeat increments to [0, 300] s.
- **`web.rs`** — public routes `/`, `/profiles`, `/submit`, `/unlock`; API `/api/gate`, `/api/status`, `/api/heartbeat`; `/admin` nested; `/static/*` embedded via rust-embed. The child is identified by the `sesame_child` cookie (a single enabled child skips the selector).
  - **`/api/gate` vs `/api/status`**: `status` answers *"does THIS child have time?"* (cookie or `?child_id=`). `gate` answers *"is the MACHINE unlocked?"* — any child with a live grant. The kiosk/timer/lock have no cookie and don't know who's sitting there, so `gate` is the only question that makes sense for them.
  - **The clock starts at `/unlock`, not `/submit`**: the kid reads corrections for free; the grant opens when they click the button. A passed attempt is redeemable **once** (partial unique index on `grants.attempt_id` + handler check).
- **`quiz.rs`** — `pick_questions` allocates proportionally to `subjects.weight` among `enabled` subjects (Hamilton/largest-remainder in `distribute()`). `grade(pool, submission, threshold_pct)` takes the threshold from the child. Multi-answer questions are all-or-nothing.
- **`admin.rs`** — admin panel; `AdminAuth` extractor validates the `sesame_admin` cookie against `admin_sessions`; no password set → redirect to `/admin/setup`.
- **`auth.rs`** — argon2 hash in `settings`, random session tokens.
- **`importer.rs`** — permissive JSON import; per-question errors collected, not fatal.
- **`db.rs`** — opens SQLite with `foreign_keys(true)`, runs `sqlx::migrate!` at startup.

## Roadmap (pivot in progress)

Done: policy engine + grants/usage ledger (1), profiles + per-child difficulty (2), schedules + cooldown (3), renewal (4), admin children CRUD + parent panic-grant (5), `sesame-kiosk` supervising a kiosk browser (6). Pending: `sesame-lock` X11 overlay, parent/child à la xsecurelock, honouring the `lock_mode` setting (7); `sesame-timer` heartbeat daemon with a **monotonic** clock (8); SDDM session entry + hardening + installer (9).

**X11-without-a-WM gotchas** (the whole reason phase 6 looks the way it does): `_NET_WM_STATE_FULLSCREEN` is a *protocol addressed to the WM* — with no WM, `--kiosk`/`gtk_window_fullscreen()` silently do nothing, so you must ask for the screen's geometry at 0,0 yourself (`screen_size()` in `kiosk.rs` reads it via x11rb). Nobody assigns keyboard focus either — browsers cope as the sole X client, but a native window would need `XSetInputFocus`/`XGrabKeyboard`.

**Phase 6 uses a supervised browser, not an embedded webview.** The renderer is swappable: everything outside `Browser::spawn` is renderer-agnostic, and the exit-0 contract wouldn't change. Known tradeoff: a browser has an escape surface a native WebKitGTK window wouldn't (devtools, `Ctrl+N`), though with no WM there is no desktop to escape *to*. Architecture decisions live in engram (`kidgate/*`).

## Database

Schema in `migrations/*.sql`; add a new numbered file, never edit an applied one. Timestamps are `INTEGER` Unix epoch seconds; booleans `0/1` with `CHECK`; `daily_usage.day` is a **local** `YYYY-MM-DD` string. `attempt_answers` stores text snapshots deliberately — history survives question edits/deletes. Queries use the runtime sqlx API (`query_as` + `.bind()`), no `DATABASE_URL` needed.

## Templates (Askama 0.12)

`templates/` public + `templates/admin/`; stylesheets `static/quiz.css` (public) and `static/admin.css`. Askama 0.12 gotcha: `{% if %}` comparing a dereferenced binding against a tuple field inside a `{% match %}` arm fails to parse — precompute a bool in Rust instead.

## Handlers

Handler futures must be `Send`: never hold `rand::thread_rng()` across an `.await` — scope it in a block. Opaque "Handler not implemented" errors → add `#[axum::debug_handler]`.
