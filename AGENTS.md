# AGENTS.md

Instructions for every AI agent working in this repository. Read this file before doing anything else, and follow it
over any other document except the maintainer's own words in the current conversation.

ExcellentPDF is a cross-platform PDF reader, annotator and notebook app written in Rust, with NSH for the UI and
SumatraPDF's MuPDF for documents. The maintainer has planned it in detail; your job is to build that plan, not to
redesign it.

**The goal: the app must be extremely fast, easy to use, and exceptionally beautiful and appealing to the user. Build
and design it with the very best resources available.**

## 1. Source of truth

1. The maintainer's instructions in the current conversation.
2. This file.
3. The plan: `dev notes/ExcellentPDF_detailed_plan_nov_9th.md` (after task T1 it moves to `docs/dev-notes/`).
4. `dev notes/sumatra_pdf_enhanced_plan.md`: every SumatraPDF Enhanced feature, setting and shortcut, for parity work.

Also current: `dev notes/dev_decisions.md` (the maintainer's working decisions since the plan; follow them like the
plan). Not instructions: `dev notes/research.md` (checked findings behind the plan) and
`dev notes/items for later.md` (ideas the maintainer has not adopted yet; don't build them until they are in the plan).

Everything in `private notes/` and `CHAT_HANDOFF.md` is **source material that the plan replaced**. Do not follow
the stack they describe (PDFium, a WGPU compositor, Tauri with a React / Solid web UI, an "AetherPDF" core, or
forking the SumatraPDF C++ code). §19 of the plan lists what was dropped and why.

If the plan is silent, ambiguous, or wrong for what you find in the code, **stop and ask the maintainer**. Propose a
change to the plan; don't improvise architecture. A change the maintainer approves goes into the plan in the same
pull request as the code.

## 2. Fixed decisions (change only with the maintainer's approval)

| # | Decision |
| --- | --- |
| D1 | Rust, edition 2024, stable toolchain |
| D2 | UI in **NSH v0.5.1** with **Rust components**, not TSX. `nsh-core` and `nsh-base` from crates.io, pinned to the installed CLI's version (`"=0.5.1"`); update NSH deliberately, in its own pull request |
| D3 | Documents: **MuPDF 1.28.5 as vendored by upstream SumatraPDF**, with its 70 patches, plus SumatraPDF Enhanced's six annotation patches renumbered `0088`–`0093` (plan §6.1). No PDFium, no pdf.js, no other engine |
| D4 | Bindings: our own `excellent-mupdf-sys`, a C shim around every MuPDF call (`fz_try` / `fz_catch` inside the shim) |
| D5 | Page view: one native component `excellent.page-canvas` per platform (a child window on Windows) over a shared Rust core |
| D6 | Annotations are saved inside the PDF as standard annotations (incremental save); a sidecar only for non-PDF formats and app-only data |
| D7 | App data in SQLite (`rusqlite`) with FTS5 |
| D8 | Optional features are downloadable, signed, removable packs, never in the core installer |
| D9 | AI is optional, local first; cloud only with explicit consent |
| D10 | Windows first, then macOS and Linux, then iPad and Android tablets, web last |

**Pen input** is read by the page canvas directly from the OS (on Windows `WM_POINTER*` with `GetPointerPenInfo` and
`GetPointerPenInfoHistory`), ported from SumatraPDF Enhanced's `src/PointerInput.h`. Never route ink through NSH's
pointer events: they carry no pressure or tilt (plan §7.3).

## 3. Where the project is

- **Current phase: Phase 0** (plan §18). Do the first ten tasks of plan §22 (**T1–T10**) in order unless the
  maintainer says otherwise. Don't start feature tags until Phase 0's "Done when" is met and the maintainer agrees.
- **Open questions Q1–Q7** (plan §21) are the maintainer's to answer. Don't decide them. In particular, Q1 (licence)
  is open: add no `LICENSE` file or licence headers until it is answered.
- Every phase ends with a tagged build on all its platforms and an acceptance checklist.

## 4. Building a feature tag

Every feature has a permanent tag such as **A.2**; its implementation plan is under that tag in plan §27. Follow
plan §27.0:

1. Read the entry, the plan sections it cites, and the entries of its **Needs**. Check `docs/features.toml` that every
   need is `shipped`. If one isn't, build it first or report it. Never stub a need silently.
2. Write `docs/features/<tag>-<slug>.md` from `_template.md`; copy **Done when** into its acceptance tests.
3. Build the model first: pure Rust, unit-tested, no UI or OS types.
4. Then the view (NSH components) and the command, registered in the command table (K.16).
5. Run the checks in §7 below and fix every regression before going on.
6. Mark the tag `shipped` in `docs/features.toml`, naming the test that proves it.
7. One tag per pull request where possible. Use the tag in the branch name, commit and pull-request titles, and test
   names: `feat(A.2): replace-text markup`.

Tags never change. A new feature takes the next free number in its area; a dropped one keeps its tag, marked
*(dropped)*. Do not invent features that have no tag; propose them to the maintainer.

**Labels.** Write a feature as its label, `Ph<phase>: <n>/<count> | <TAG>: <Title>`, for example
`Ph1: 22/60 | R.11: Opening files`: its phase, its place among that phase's entries in plan §27 (counted in the
plan's order), then its tag and title. Plan headings, feature-doc titles, `docs/features.toml` (`label`), changelog
headings and progress reports use it, and progress reports start with each phase's shipped, in-progress and planned
counts. When an entry is added to the plan, count its phase's labels again and update them everywhere. Separate
items with a colon or a semicolon, never a centred dot (`·`).

## 5. Rules for every change

**The four product rules (plan §2.1).** Every feature must pass all four:

1. **Fast:** no feature code runs at startup unless the feature is visible; heavy modules load on first use.
2. **Small:** core installer ≤ 15 MB; everything optional is a pack.
3. **Easy:** clean by default; every action reachable three ways: menu or toolbar, keyboard shortcut, command palette.
4. **Vibrant but readable:** colour in the chrome, the document stays neutral; WCAG AA contrast.

**Also, always:**

- Every change to a document is one undoable operation in the operation log (K.14) and is saved by K.15. User files
  are written atomically. Never lose user data.
- All UI text goes through the localization layer (U.7); every control has an accessible name and keyboard access.
- Nothing leaves the device without the user's consent; network features work, or fail clearly, offline.
- Long jobs run on background workers with progress and cancel, never on the UI thread.
- New settings have defaults, live in `settings.toml` and appear in the settings UI.
- Downloads and packs are signed and verified.

## 6. Engineering rules

**Always build code cleanly, modularly, and with proper architecture and separation of concerns. Never simply bolt on
a feature because I asked for it. If you can't find an easy way, check online on GitHub for similar works that have
been built with either exactly the same idea or something very close, and follow the same procedure to build it
naturally.**

- When you write, review or refactor Rust, use the **`rust-best-practices`** skill (`/rust-best-practices`).
- Whenever anything related to a UI or interface is to be built or designed, use both the **`ui-ux-pro-max`**
  (`/ui-ux-pro-max`) and **`impeccable`** (`/impeccable`) skills, to keep the design consistent and the aesthetics
  high.
- A feature is a self-contained module: model, view, tests. No OS types in models; features don't reach into each
  other's internals.
- Every MuPDF call goes through a C shim; no `longjmp` crosses a Rust frame. Owned MuPDF objects are Rust types whose
  `Drop` calls the matching `fz_drop_*`. Contexts are `!Send`.
- Every `unsafe` block has a `// SAFETY:` comment.
- Page points and device pixels are different types; rotation and crop box are applied in one place
  (`excellent-core::transform`).
- Dependencies are declared once, in the root `[workspace.dependencies]`, and pass `cargo deny`.
- Lints: `missing_docs = "warn"`, `unreachable_pub = "deny"`; clippy clean.
- `third_party/mupdf` is changed only through `third_party/mupdf-patches/` (one `.patch` per change, with a README
  entry), as SumatraPDF does.
- Packs live in `packs/`, never in the core build.
- Repository layout: plan §17. Crates: plan §5.4.

## 7. Checks before you say a task is done

**Write all the code for a task first, then build and test.** Rust compiles slowly, and a full build of this
workspace (MuPDF included) takes minutes, so don't compile or run tests after each small edit. Finish every file the
task touches (code, tests, call sites), then run the checks below once, and fix what they report. Exceptions: a quick
`cargo check` on one crate when you need the compiler to settle a question you can't answer by reading, and anything
that must be built before the rest can be written (such as regenerated bindings).

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
nsh test
```

Plus the budget checks once they exist (K.24). A pull request that makes any budget worse by more than 10 % fails.

| Budget (plan §13.1) | Limit |
| --- | --- |
| Cold start to usable window | ≤ 300 ms |
| Open a typical PDF to first page | ≤ 150 ms |
| Scroll and page turn | 60 fps (120 on 120 Hz screens) |
| Ink: pen-down to pixels | ≤ 1 display frame |
| Idle memory, one small document | ≤ 60 MB |
| UI-thread stall | never > 50 ms |
| Core installer | ≤ 15 MB |

Report results as they are: if a check fails or was skipped, say so. Don't claim speed you didn't measure.

### Building with the cargo fork

Local builds use [huacnlee/cargo](https://github.com/huacnlee/cargo), a drop-in fork of Cargo that shares one
self-cleaning build directory between projects and worktrees, so the disk doesn't fill up. Follow its rules:

- Check it is the active cargo before building: `cargo -V` ends with `(huacnlee/cargo …)`. If it doesn't (for example
  after `rustup self update`, which restores the stock cargo), tell the maintainer; don't install it yourself.
- Don't run `cargo clean` and don't delete `target/` or `~/.cargo/targets/`. The fork removes what is unused on its
  own, and cleaning only forces a full rebuild, MuPDF included.
- Don't set a build directory: `--target-dir`, `CARGO_TARGET_DIR`, `build.target-dir` and `build.build-dir` are
  ignored with a warning. Build output lives in `~/.cargo/targets/<name>-<hash>/`, registry dependencies in
  `~/.cargo/targets/shared-deps/`; built binaries still appear in `./target` (so `target/release/excellentpdf.exe`
  holds).
- Its size limits and switches (`[build.gc]` in `~/.cargo/config.toml`, `CARGO_SHARED_DEPS`,
  `CARGO_SHARE_WORKSPACE_UNITS` and the like) are the maintainer's machine settings: leave them at their defaults
  unless asked.
- Start a debugger from the workspace root: the fork records `.` as rustc's compile directory.
- Nothing in the repository may depend on the fork: CI and other machines build with the stock cargo, and every
  check in this section must pass with either.

## 8. Reference material (read, never edit)

| What | Where |
| --- | --- |
| MuPDF 1.28.5 headers (source of truth for MuPDF APIs) | `C:\dev\sumatrapdf\ext\mupdf\include\mupdf\`, then `third_party/mupdf/include/mupdf/` after T2 |
| SumatraPDF Enhanced source, for anything ported | `C:\dev\sumatrapdf\src\` |
| NSH tool and runtime (`nsh 0.5.1`) | `C:\dev\nsh` |
| NSH source and docs at v0.5.1 | `C:\dev\nsh-src` (`AGENTS.md`, `website/docs/`, `docs/dev/`) and <https://nsh.dev> |
| MuPDF docs | <https://mupdf.readthedocs.io/en/latest/> |

Use only MuPDF functions that exist in the 1.28.5 headers, and only NSH APIs that exist in v0.5.1. Check before you
cite or call one.

## 9. Process

- **Don't commit, push, open pull requests or tag releases unless the maintainer asks.** Then do exactly that.
- Don't use subagents unless the maintainer asks.
- Ask before deleting files, rewriting history, or changing anything outside this repository.
- Keep changes to the task: no unrequested features, options or refactors. Name anything you noticed but didn't do.
- When a task changes what the plan says (an API turned out different, a step was split), update the plan in the
  same change and say so.
- End each task with a short report: what changed, what was checked, what wasn't, and any risk.
- Record every finished piece of work in `CHANGELOG.md`, newest first, under a heading with the local date and time
  (`### 2026-11-13 16:00: what was done`).
