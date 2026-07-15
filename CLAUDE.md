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

Done: policy engine + grants/usage ledger (1), profiles + per-child difficulty (2), schedules + cooldown (3), renewal (4), admin children CRUD + parent panic-grant (5), `sesame-kiosk` supervising a kiosk browser under `cage` (6), `sesame-timer` heartbeat daemon with a **monotonic** clock (8), SDDM session entry + hardening + installer (9).

**Phase 7 (`sesame-lock`) is cancelled, not pending.** It was designed as an X11 override-redirect overlay à la xsecurelock. Wayland does not allow that — a client cannot grab input or cover the screen; the only sanctioned path is `ext-session-lock-v1`. It isn't needed: when the grant runs out, `sesame-timer` **terminates the session**, which returns to SDDM, which re-runs the gate. Backend-agnostic, already implemented, and the right failure direction.

## The display stack — read this before touching the gate

Both target machines run **Plasma 6 on Wayland**. `plasma-x11-session` is a *separate package* in Plasma 6 and is **not installed**: an X11 desktop is not merely non-default, it is **unavailable**. Anything below that assumes X11 is stale.

**Who starts the display server is the whole design.** An entry in `/usr/share/xsessions/` makes SDDM start Xorg *before* running your script — that is how the gate used to get a screen with no WM. A **Wayland** entry (`/usr/share/wayland-sessions/`, which is what `install.sh` now writes) makes SDDM start **nothing**: the compositor *is* the server. So `sesame-session` brings its own — **`cage`**, a kiosk compositor whose only policy is *one app, fullscreen*. Alt+Tab, Alt+F4, taskbar: cage doesn't implement them. Same guarantee as a bare X server, but by design rather than by absence.

Consequences, all of them load-bearing:
- **`--kiosk` finally works.** A real compositor honours fullscreen, so the old x11rb `screen_size()` geometry hack is gone, and `x11rb` with it.
- **Each browser needs its Wayland key** or it hunts for an X server and dies: Chromium `--ozone-platform=wayland`, Firefox `MOZ_ENABLE_WAYLAND=1`. See `Flavour::command` in `kiosk.rs`.
- **`DontZap` is gone** — no X server to kill, so Ctrl+Alt+Backspace has no meaning.
- **VT switching is off by default under cage, and that matters.** `cage` swallows Ctrl+Alt+F1…F6 unless started with `-s` ("Allow VT switching"); KWin (once the desktop is up) honours them, which is why the shortcut *looks* like it only works after a pass. `sesame-session` starts `cage -s` **on purpose**: during a blocked slot the parent has no desktop, no `/admin` link on the blocked page, and often no SSH (Wi-Fi secret asleep in KWallet) — the VT console is their only escape hatch. Two preconditions for that hatch to be real: (1) **do not** apply the `NAutoVTs=0` hardening — it empties the VTs, so `-s` lands on a getty-less black screen; the two cancel out. (2) the child account **must have a password**, or Ctrl+Alt+F2 is a passwordless child shell — a worse hole than the one it plugs. Keep `xorg-xwayland` installed (a wlroots bug freezes the screen on VT switch without it).
- **The exit-0 contract now passes *through* cage**, and nothing guarantees cage propagates its child's status. So it isn't trusted: after cage returns, `sesame-session` asks the API again via `sesame-kiosk --check` (one request, no browser, exit 0/1). Missing cage, broken cage, crashed kiosk, silent server — all land on exit 1. **A lock that fails open is not a lock.** Never let the desktop start on anything but a positive answer from `policy::evaluate`.

### SDDM reads the session's exit code — and a non-zero one kills the autologin

**`sesame-session` must exit 0 when the session ends normally.** SDDM logs any non-zero exit from an autologin session as `Process crashed`, **disables the autologin, and stops** — no retry, no greeter, black screen until an adult reboots. That anti-crash-loop guard is *correct*; the bug is lying to it.

This is why `sesame-session` no longer ends in `exec startplasma-wayland`: with `exec`, Plasma's exit code *was* the session's, and the timer killed Plasma, so Plasma died with an error, so SDDM saw a crash. A session ending because time ran out is not a crash. It runs Plasma as a child, then `exit 0` (plus a `TERM` trap for the force path).

The other half of the same bug is in `timer.rs::logout()`, and **the order of the two paths is load-bearing**:
1. **Ask politely first** — `qdbus6 org.kde.Shutdown /Shutdown org.kde.Shutdown.logout` (no args, no confirmation). Plasma leaves on its own and returns 0, SDDM re-autologins, the child is back at the gate. **This is the path that must normally run.** Note the Qt6 binary is **`qdbus6`**; `qdbus` may be Qt5's or absent, so try both. (Plasma 5's `org.kde.ksmserver` `/KSMServer logout 0 0 0` is the *old* signature — the bus name still exists in Plasma 6, but don't use it.)
2. **`loginctl terminate-session` last, never first.** It doesn't ask, it *kills* — and a killed Plasma exits with an error. It's the emergency hammer, not the normal path.

The whole loop of the design (earn time → use it → time runs out → back to the gate → earn more) depends on that clean exit. Break it and the child is locked out after their first session, which is the *opposite* failure of the one everyone worries about — but a failure all the same.

**Phase 6 uses a supervised browser, not an embedded webview.** The renderer is swappable: everything outside `Browser::spawn` is renderer-agnostic, and the exit-0 contract wouldn't change. Known tradeoff: a browser has an escape surface a native window wouldn't (devtools, `Ctrl+N`) — cage neither adds to it nor removes it. Architecture decisions live in engram (`kidgate/*`).

## Database

Schema in `migrations/*.sql`; add a new numbered file, never edit an applied one. Timestamps are `INTEGER` Unix epoch seconds; booleans `0/1` with `CHECK`; `daily_usage.day` is a **local** `YYYY-MM-DD` string. `attempt_answers` stores text snapshots deliberately — history survives question edits/deletes. Queries use the runtime sqlx API (`query_as` + `.bind()`), no `DATABASE_URL` needed.

## Templates (Askama 0.12)

`templates/` public + `templates/admin/`; stylesheets `static/quiz.css` (public) and `static/admin.css`. Askama 0.12 gotcha: `{% if %}` comparing a dereferenced binding against a tuple field inside a `{% match %}` arm fails to parse — precompute a bool in Rust instead.

## Handlers

Handler futures must be `Send`: never hold `rand::thread_rng()` across an `.await` — scope it in a block. Opaque "Handler not implemented" errors → add `#[axum::debug_handler]`.
