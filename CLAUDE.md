# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`luanti-gate` is a Rust web app that acts as a **drop-in wrapper** for the Luanti game binary. The kid launches "Luanti", but what actually runs is this app: it serves a quiz in the browser, and only if the kid passes does it spawn the real game (`config.paths.game_binary`) — killed automatically after a configurable interval.

The compiled binary is deliberately named `luanti` (see `[[bin]]` in `Cargo.toml`) so it can shadow the real one on `$PATH`. **All user-facing strings — UI, CLI help, errors — are in French.** Code comments and identifiers are mixed FR/ES/EN; match the surrounding file.

## Commands

```bash
cargo check                      # primary feedback loop
cargo build --release
cargo run -- --help              # CLI help (French)
cargo run                        # start server (opens browser; /admin if no password set, else /)
cargo run -- admin               # start server, force-open /admin
cargo run -- import data/questions_mathematiques.json
cargo run -- preview 10          # console simulation of a quiz + grading
cargo run -- --config ./config.toml
./scripts/install.sh             # Linux only: release build → ~/.local/bin/luanti + .desktop entry
```

There are no tests in the repo. Do not run builds unless asked.

## Configuration split — this trips people up

Two different config sources, on purpose:

- **`config.toml`** (static, requires restart): `server.host`/`server.port`, `paths.game_binary`, `paths.database`. Lookup order when `--config` is not given: `$XDG_CONFIG_HOME/luanti-gate/config.toml`, then `./config.toml`. A relative `paths.database` resolves against `$XDG_DATA_HOME/luanti-gate/`, **not** the CWD (`config::resolve_data_path`).
- **`settings` table in SQLite** (dynamic, live-editable from `/admin/settings`): `questions_per_test`, `pass_threshold_pct`, `kill_interval_minutes`, `admin_password_hash`. Read via the `read_setting_*` helpers in `web.rs`, always with a default.

New tunable knobs belong in the `settings` table, not in `config.toml`.

## Architecture

- **`main.rs`** — hand-rolled CLI parser (no clap): `Cli`/`Command`. Dispatches to server / import / preview. `run_server` binds the listener (port `0` = OS-assigned), rewrites an unspecified bind IP to loopback for the browser URL, spawns the OS-specific browser opener, and installs a Ctrl+C shutdown that kills the running game child.
- **`web.rs`** — `AppState { pool, cfg, game }` where `game: GameSlot = Arc<Mutex<Option<GameSession>>>` holds the single running child process. Public routes (`/`, `/submit`, `/game`, `/game/start`), `/static/*` served from assets **embedded in the binary via rust-embed** (a relative `ServeDir` broke on client machines), and `/admin` nested. Also hosts `AppError` and the `render()` helper.
- **`admin.rs`** — the whole admin panel (~1000 lines). Auth is an axum extractor `AdminAuth` that validates the `luanti_admin` cookie against `admin_sessions`; missing/invalid → redirect to `/admin/login` or `/admin/setup` (first run, no password yet). Routes: questions CRUD, subjects (create/delete/enable-toggle/dedupe), settings, JSON import, attempt history.
- **`quiz.rs`** — `pick_questions` selects questions **proportionally to `subjects.weight`** among `enabled = 1` subjects, using `distribute()`: a pure Hamilton/largest-remainder allocator with an iterative cap for subjects that don't have enough questions. Weights are relative, they need not sum to 1. `grade()` is **all-or-nothing for `multi` questions**: the chosen answer set must exactly equal the correct set.
- **`auth.rs`** — argon2 password hash stored in `settings`, random 32-byte hex session tokens in `admin_sessions`.
- **`importer.rs`** — permissive JSON import: per-question errors are collected into `ImportReport.questions_failed` instead of aborting. Subjects referenced by a question must already exist or be declared in the same file.
- **`db.rs`** — creates the parent dir, opens SQLite with `foreign_keys(true)`, runs `sqlx::migrate!("./migrations")` at startup.

### Game session lifecycle

`POST /game/start` → spawn child, store `GameSession { child, started_at, kill_at }`, `spawn_watchdog` sleeps until `kill_at` and kills the child (re-checking `kill_at` matches, so a replaced session isn't killed by a stale watchdog). `GET /game` renders a meta-refresh page timed to `kill_at`; if the slot is empty it redirects to `/`.

## Database

Schema lives in `migrations/*.sql`; sqlx runs them at startup. Add a new numbered file, never edit an applied one. Conventions: timestamps are `INTEGER` Unix epoch seconds, booleans `0/1` with `CHECK`.

`attempt_answers` stores **snapshots** of question/answer text, deliberately — history must survive editing or deleting questions. Everything else cascades: deleting a subject deletes its questions and their answers.

Queries use the **runtime** sqlx API (`sqlx::query_as` with `.bind()`), not the compile-time `query!` macros — no `DATABASE_URL` or offline metadata needed.

## Templates (Askama 0.12)

`templates/` — `base.html` + public pages; `templates/admin/` with its own layout. Two stylesheets in `static/`: `quiz.css` (public, teal design system) and `admin.css`. Pico is present but no longer loaded by `base.html`.

Known Askama 0.12 gotcha: an `{% if %}` comparing a dereferenced binding against a tuple field inside a `{% match %}` arm fails to parse. Precompute the comparison in Rust and pass a bool/flag to the template instead.

## Handlers

Handler futures must be `Send`. Never hold a `rand::thread_rng()` (`ThreadRng` is `!Send`) across an `.await` — scope it in a block, as `quiz::pick_questions` and `auth::hash_password` do. If you get an opaque "Handler not implemented" error, slap `#[axum::debug_handler]` on the handler to see the real cause.
