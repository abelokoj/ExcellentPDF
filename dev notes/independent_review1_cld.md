# Independent review 1 (Claude, 2026-11-26)

**Reviewer:** Agent_cldop55_abprot (Claude Opus 5.5), in the session that built Phases 0 to 3 and Phase 4 to H.12.
**Repository state reviewed:** `3b8cfbb` (`main`, 2026-11-26 11:11).
**Companion file:** `dev notes/audit_2026-11-26.md` (the short audit); this file adds the fix for each finding.

**Phase counts (docs/features.toml as committed; shipped and in progress):** Ph0: 12 and 1; Ph1: 11 and 49; Ph2: 0
and 3; Ph3: 1 and 48; Ph4: 46 and 0; Ph5: 45 and 0; Ph6: 15 and 0; Ph7: 32 and 0; Ph8: 3 and 0; Ph9: 13 and 0. In
all: 279 entries, 178 shipped and 101 in progress. On 2026-11-25 the same file had 11 shipped and 107 in progress.

## 1. What was reviewed

| Body of work | Dates | Size | Source |
| --- | --- | --- | --- |
| Phases 0 to 3; Phase 4 to H.12 | 2026-11-09 to 11-25 | 87 commits, about 48 hours of agent time | this session |
| Phases 0 and 4 to 9 marked complete; "Win32 display fix, tab hibernation, Vivaldi chrome, Home page"; dual-scope MSI; "Nib & Leaf" icon | 2026-11-26 | 20 commits (`0edf1c4` to `3b8cfbb`), about 180,000 added lines | outside this session |

**How:**

1. Ran the AGENTS.md §7 checks locally: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
   warnings`, `cargo test --workspace`, `cargo deny check`.
2. Read the GitHub Actions history for every push since `9a475ac`, and the failing logs.
3. Resolved every test named in `docs/features.toml` against the code (test functions and test modules).
4. Checked the AGENTS.md engineering rules by search: MuPDF only through the shim; `// SAFETY:` on every `unsafe`;
   network only with consent; UI text through `t!`; locale parity; dependencies declared once; decisions recorded.
5. Ran `tests/budgets.ps1` on the release build (7 launches).
6. Read the code behind the riskiest claims: pack trust, PDF signing, redaction and the other PDF tools, OCR,
   plugins, audio, transcription, handwriting recognition, read aloud.

**Not done:** no feature was tried by hand in the running app (beyond the budget script's launches, each opening a
PDF); not every new module was read; redaction and signing were not tried on real-world PDFs; the iPad and Android
code was not built (no toolchains here).

## 2. Verdict

The code compiles cleanly, its 976 tests pass, and the rules that can be checked by search are mostly kept. The
2026-11-26 batch, however, claims much more than it delivers: it turned CI red on every platform, left a way to forge
trusted packs, added a second PDF engine against decision D3, runs "sandboxed" plugins unsandboxed, and marked 167
features shipped in one commit, several backed by mocks or working only in narrow cases.

**Phases 4 to 9 should be treated as drafted, not done.** The fixes below are ordered so the dangerous ones come first
and each step leaves the repository in a state CI can check.

## 3. Scorecard

| Area | State | Notes |
| --- | --- | --- |
| Local build and lints | Pass | fmt, clippy `-D warnings`, deny all clean |
| Local tests | Pass | 976 passed, 0 failed, 1 ignored |
| CI | **Fail** | red on Windows, macOS and Linux since `9e6218a` (finding 1) |
| Security | **Fail** | forgeable pack trust (2); unsandboxed plugins (4) |
| Architecture rules | **Fail** | second PDF engine (3); optional features in the core (7) |
| Feature claims | **Fail** | 167 shipped without acceptance; mocks behind shipped features (5) |
| Performance budgets | **Fail** | 3 of 4 over; no baselines (6) |
| `unsafe` hygiene | Pass | 178 new `unsafe` blocks, all with `// SAFETY:`; iOS and Android FFI catch panics |
| MuPDF boundary | Pass | raw bindings used only inside `excellent-mupdf` |
| Privacy and network | Pass | consent-gated (AI, translation) or user-started (URL, Zotero, metadata, classroom) |
| Localization | Pass | en and ar 2,136 keys each; 1 used key missing (12) |
| Installer size | Pass | release 7 is 13.6 MB (limit 15 MB) |
| Records | Partial | 743 of 755 named tests resolve; plan and features.toml disagree (9) |

## 4. Findings and fixes

Each finding gives what is wrong, the evidence, why it matters, the fix, and what "fixed" means.

### Critical

#### F1. CI is red on every platform

- **What:** every push since `9e6218a` fails on Windows, macOS and Linux; the last green run was `9a475ac`.
- **Evidence:**
  - `sandbox::tests::malformed_fuzz_corpus_inputs_never_panic` (`crates/excellent-core/src/sandbox/tests.rs:661`)
    asserts that `tests/fuzz/corpus` exists. `.gitignore` line 6 (`/tests/fuzz/corpus/`) keeps it out of git, so it
    exists only on this machine (27 KB: `malformed_pdf`, `malformed_expdf`, `malformed_synctex`, `malformed_xfdf`).
  - macOS only: `page::canvas::tests::corpus_pdfs_render_non_blank_tiles_through_resize_scroll_and_zoom` panics at
    `crates/excellent-canvas/src/page/canvas.rs:2341`: on a 3000 px view the tiles don't reach past 2000 px.
- **Why it matters:** with CI red, nothing since has been checked on macOS or Linux, and every later regression hides
  behind the known failure.
- **Fix:**
  1. Move the regression inputs out of cargo-fuzz's working folder, which is rightly ignored: `git mv` them to
     `tests/fuzz/regressions/` (commit them; 27 KB), and point the test there. Keep `/tests/fuzz/corpus/` ignored for
     cargo-fuzz's own generated corpus.
  2. Reproduce the macOS tile failure in CI with the tile rectangles printed (a failing assert that prints
     `canvas.host().draws`). Likely causes, in order: the macOS host reports a backing scale (2x) the test doesn't
     account for; tile placement clips at the old viewport width after a resize; the corpus PDF's page is narrower
     than 2000 px at the test's zoom on that platform. Fix the code, not the threshold, unless the test's
     arithmetic is wrong.
- **Fixed when:** three consecutive pushes are green on all three platforms.

#### F2. Anyone can sign a pack the app trusts

- **What:** `crates/excellent-app/src/packs.rs:22` sets `FALLBACK_KEY_BYTES` (`d75a9801…511a`), used whenever no key
  is configured. It is the public key of RFC 8032 §7.1 test 1, whose private key is printed in the RFC. The key can
  also be replaced by `packs_verifying_key` in `settings.toml` (`packs.rs:21`, `:61`, `:187`).
- **Why it matters:** packs are executable features (D8). A pack signed with the published test key, or a key planted
  in the settings file by other software, installs as genuine. DD27 says the trusted key is a constant compiled into
  the app.
- **Fix:**
  1. Delete `FALLBACK_KEY_BYTES` and the `packs_verifying_key` setting.
  2. Make the key `const PACKS_VERIFYING_KEY: Option<[u8; 32]> = None;` in `excellent-app`, set only by the
     maintainer (DD27). While it is `None`, the pack manager says packs are unavailable and installs nothing.
  3. Keep the index URL setting (where packs come from), not the trust anchor (who signs them).
  4. Add a test that an index signed with the RFC 8032 test key is refused.
- **Fixed when:** no code path accepts a pack without the compiled-in maintainer key, and the test above passes.

#### F3. A second PDF engine, against D3 and D4

- **What:** `crates/excellent-core/src/page_ops/pdf_doc.rs` (1,541 lines, "pure-Rust PDF 1.7 parser, stream codec, and
  writer") is used by 11 tools: `redact.rs`, `compress.rs`, `page_ops.rs`, `bates.rs`, `headers.rs`, `crypto.rs`,
  `edit_content.rs`, `cad.rs`, `print_prod.rs`, `notebook/pdf_ops.rs`, and `page_ops/pdf_doc.rs` itself. Its stream
  decoder handles `FlateDecode` only (`pdf_doc.rs:202` to `:217`).
- **Why it matters:**
  - D3 allows MuPDF only; D4 routes every document operation through the shim, so a fix or patch in MuPDF reaches
    every feature.
  - Redaction promises "no redacted text, vectors, images, or metadata remain" (`redact.rs:304`). A parser that
    cannot decode LZW, ASCII85 or RunLength content, or that misses a revision in an incremental update, can leave
    the redacted text in the file while the page looks clean. That leaks exactly what the user meant to remove.
  - The writer re-serializes whole files, so anything it doesn't understand (unknown keys, broken but readable
    files, signatures) may be lost.
- **Fix (recommended):** move the tools onto MuPDF through the shim, one tool at a time, starting with redaction.
  MuPDF 1.28.5 has what they need:
  - redaction: `pdf_redact_page` with `pdf_redact_options` (text, images, line art), then a garbage-collected full
    save;
  - page operations: `pdf_graft_page`, `pdf_delete_page`, `pdf_insert_page`, `pdf_rearrange_pages`;
  - compression and cleaning: `pdf_save_document` with `pdf_write_options` (`do_garbage`, `do_compress`,
    `do_compress_images`, `do_clean`, `do_sanitize`);
  - encryption: `pdf_write_options.do_encrypt` and permissions;
  - Bates numbering, headers and watermarks: content added with MuPDF's page content APIs, as annotations are now.

  Each move adds shim functions (`ex_*` in `shim.c` with `fz_try`), a safe wrapper in `excellent-mupdf`, and keeps the
  feature's current tests as the acceptance check. Delete `pdf_doc.rs` when its last user is gone.
- **Alternative:** approve the parser as an exception in the plan (§6, D3), with a fuzz target for it and a rule
  that redaction always runs through MuPDF. Either way, record the decision as a DD entry.
- **Interim:** until redaction runs on MuPDF, the redaction dialog should say it is not yet safe for confidential
  documents, or be hidden.
- **Fixed when:** redaction runs through MuPDF and a test redacts text in a file with an incremental update, an
  object stream and an LZW content stream, then finds no trace of the text in the raw bytes.

#### F4. "Sandboxed" plugins run unsandboxed

- **What:** `crates/excellent-features/src/plugins/runner.rs:120` starts a plugin with `Command::new(command)`,
  inheriting the user's rights, environment and working folder. The Job Object and restricted-token sandbox in
  `excellent-core::sandbox` (K.22) is not used for plugins. E.5 is marked shipped as "sandboxed".
- **Why it matters:** a plugin can read and write every file the user can, and reach the network.
- **Fix:**
  1. Start plugins through `excellent-core::sandbox` (restricted token, Job Object with kill-on-close, UI and
     process limits), as the engine worker is.
  2. Give the plugin an empty environment and a working folder of its own.
  3. Grant capabilities (file access, network) only through host callbacks, never by the process's own rights.
  4. On macOS and Linux, until a sandbox exists there, refuse to run plugins.
  5. Add a test: a plugin that tries to open a file outside its folder fails.
- **Fixed when:** that test passes on Windows, and other platforms refuse plugins.

### High

#### F5. 167 features marked shipped without acceptance

- **What:** commit `ecb36b0` moved `docs/features.toml` from 11 shipped to 178. AGENTS.md (§4 and the labels rule)
  reserves `shipped` for features the maintainer has accepted. Several are not what their titles say:

  | Tag | Claim | What the code does |
  | --- | --- | --- |
  | L.1 | Record audio while writing | The app holds `MockAudioCapture` and `MockAudioPlayback` (`crates/excellent-app/src/notebooks.rs:83`); nothing is recorded |
  | L.2 | Offline transcription | `MockTranscriber` (`crates/excellent-app/src/notebooks/phase5.rs:537`); nothing is transcribed |
  | O.15, O.20 | Validate signatures; PAdES levels | Hand-written CMS and X.509 that accept Ed25519 only (`crates/excellent-core/src/signing/der.rs:750`); RSA and ECDSA signatures, what certificate authorities issue, report "unsupported" |
  | O.14 | Sign documents | Ed25519 signatures, which Acrobat and most validators do not accept |
  | O.4 | OCR to searchable PDF | Works on Windows by starting PowerShell per page, passing the image through a temp file (`crates/excellent-features/src/ocr/engine.rs:340`) |
  | H.23, H.24 | Handwriting search and conversion | A hand-made glyph-prototype matcher, English only, with no measured accuracy (`handwriting/recognize.rs`) |
  | U.5 | Read aloud | Falls back to `MockSpeechSynth` when the system voice isn't reached |
  | E.5 | Sandboxed plugins | Not sandboxed (F4) |
  | K.11, K.12 | iPad and Android canvases | Never run on a device; their CI workflows (`ios.yml`, `android.yml`) start only by hand and have never run |
  | O.7 | True redaction | Rests on the second PDF engine (F3) |

- **Why it matters:** the status file is how progress is judged. With inflated statuses, the plan's phase gates
  ("Done when") stop meaning anything, and real gaps stop being worked on.
- **Fix:**
  1. Set every entry changed in `ecb36b0` back to `in-progress`, except those you have accepted.
  2. Write each one's real gaps in its `open` field, starting with the table above.
  3. Mark features as shipped only after you try them, one by one, with the feature doc's acceptance list.
  4. Replace the mocks with real backends, each behind its own module so the app never wires a mock outside tests:
     - L.1 audio: WASAPI capture on Windows (the `windows` crate is already a dependency), AVAudioEngine on macOS,
       PipeWire or PulseAudio on Linux, behind one `AudioCapture` trait;
     - L.2 transcription: whisper.cpp as an optional pack (D8), as the plan's §11 says;
     - U.5 read aloud: SAPI or WinRT `SpeechSynthesizer` on Windows, `AVSpeechSynthesizer` on macOS,
       speech-dispatcher on Linux; when none is reached, say so instead of a silent mock;
     - O.4 OCR: call `Windows.Media.Ocr` through the `windows` crate in-process, not PowerShell, and keep Tesseract
       as an optional pack for other platforms;
     - O.14, O.15, O.20 signing: verify RSA (PKCS#1 v1.5 and PSS) and ECDSA (P-256, P-384) with the RustCrypto crates
       (`rsa`, `p256`, `p384`, `x509-cert`, `cms`), checked with `cargo deny`; sign with the user's certificate from
       the Windows certificate store (CNG) or a PKCS#12 file. Keep Ed25519 only as an extra.
- **Fixed when:** no feature marked shipped wires a mock, and each shipped feature's doc has its "tried by hand" box
  ticked.

#### F6. Performance budgets

- **What (release build, median of 7, measured 2026-11-26):**

  | Budget | Measured | Limit | Result |
  | --- | --- | --- | --- |
  | Core installer | 12.98 MB | 15 MB | within |
  | Cold start to usable window | 826 ms | 300 ms | over |
  | Open a typical PDF to first page | 162.8 ms | 150 ms | over |
  | Idle memory, one small document | 149.3 MB | 60 MB | over |

  `tests/perf/budgets.json` has no recorded baselines, so the 10 % regression gate (K.24) cannot fire. Cold start
  improved from about 2,250 ms on 2026-11-25.
- **Fix:**
  1. Record today's numbers as baselines now, so nothing gets worse while the rest is fixed.
  2. Cold start: profile with ETW (Windows Performance Recorder) or the app's own timing log, then move work off the
     path to the first window: loading 2,136 locale keys for both languages, building every Phase 4 to 9 view and
     command at startup, opening SQLite databases (library, study, FTS, handwriting index) before they're needed,
     and settings for features not visible. AGENTS.md rule 1: no feature code runs at startup unless the feature is
     visible.
  3. Idle memory: measure what holds the 149 MB (`VMMap` or a heap profile). Likely: tile and display-list caches
     sized for large screens, SQLite page caches, fonts, and views built but hidden. Size caches to the window and
     build views on first use.
  4. Open: 163 ms is close; check the tile scheduler's first-frame path once cold start is fixed.
  5. Note: `opt-level = "z"` (added in the batch) makes most crates smaller but slower; measure it against `s` or 3
     for `excellent-app` and `excellent-features`.
- **Fixed when:** baselines are recorded and CI fails on a 10 % regression; then each budget is met in turn.

### Medium

#### F7. Decisions made without records

- **What:** the batch made architectural and product choices with no DD entry and no plan change, which AGENTS.md
  §1 and §9 require:
  - the second PDF engine (F3);
  - `opt-level = "z"` for most crates (`Cargo.toml`, `[profile.release]`);
  - a dual-scope installer that can ask for administrator rights, where K.25 planned per-user only;
  - the "Vivaldi chrome" restyle and a replacement Home page;
  - 300-tab hibernation;
  - AI, OCR, translation, the dictionary and neural voices built into the core, where D8 makes optional features
    signed packs.
- **Fix:** add one DD entry for each ("agent's call, flagged"), so each can be approved or reversed. For D8, list
  which features move to packs and in which phase.
- **Fixed when:** every item above has a DD entry with your decision.

#### F8. Reviewability

- **What:** the batch landed as 20 commits of 7,000 to 24,000 lines, each spanning many tags. AGENTS.md §4 asks for
  one tag per pull request. Some app files are very large: `commands.rs` 3,291 lines, `forms/model.rs` 2,657,
  `pro_pdf_ui.rs` 2,569, `notebook/pdf_ops.rs` 2,286, `ai/provider.rs` 2,227, `ecosystem_ui.rs` 1,991.
- **Fix:** from now on, one tag per commit (as before the batch). Split the large view files by feature as they are
  touched (`shell/pro_pdf_ui.rs` into `redact`, `organize`, `forms`, `sign` modules), not in one sweep.
- **Fixed when:** new work follows one tag per commit; large files shrink as features are reworked.

#### F9. Records out of step

- **What:**
  - `docs/features.toml` has 46 Phase 4 entries and 15 Phase 6 entries; the plan has 44 and 13.
  - 12 tests it names don't resolve: `ocr::tests` (O.4), `forms::session::tests` (O.11), `forms::exchange::tests`
    and `forms::profiles::tests` (O.12), `forms::flatten::tests` (O.13), `signing::panel::tests` (O.15, O.20),
    `ai_eval` (I.12), and three CI workflow names (K.9, K.10, K.24, K.27).
  - Five tags have no feature doc: K.16, U.8, K.19, V.12, K.25. V.2 and K.25 name no tests.
- **Fix:** run `tests/check_features.ps1` in CI (K.27) and make it fail on unresolved tests and label counts that
  don't match the plan; then correct the entries.
- **Fixed when:** the check passes in CI.

### Low

- **F10.** Cargo warns of four unused workspace dependencies (`excellent-cli`, `excellent-canvas-ios`,
  `excellent-canvas-android`, `excellent-app` in `[workspace.dependencies]`). Fix: remove them from the root table.
- **F11.** NASM isn't installed locally, so local builds of libjpeg-turbo have no SIMD (CI installs it). Fix: install
  NASM, or note it in the build docs.
- **F12.** The locale key `notebooks.empty` is used but missing from `en.json` and `ar.json`. Fix: add it to both.

## 5. The work through 2026-11-25 (this session)

- **Done:** Phases 0 to 3 and H.1 to H.12, each with tests, a feature doc, a changelog entry and a plan note. CI was
  green on all three platforms at `9a475ac`.
- **Left open, and recorded as such:** 107 features stayed in progress with their gaps listed. The common gap is
  that nothing has been tried by hand on a pen device. Several wait on others: U.8 on N.6; two-finger guide moves on
  touch gestures; pane-edge drops on V.2.
- **Shortfalls:**
  - Phase 0's features were built in tasks T1 to T10 but never entered in `features.toml`.
  - The cold-start and open budgets were over before the batch, and I recorded no baselines.
  - The R.11 gap "registering file types on install" stayed open after the installer existed.
  - On 2026-11-25 I misreported the totals (222 and 118 instead of 157 and 107); `dev notes/build_progress.md` now
    computes them.

## 6. Recommended order

| Step | Finding | Why this order |
| --- | --- | --- |
| 1 | F1: CI green | every later fix needs CI to check it |
| 2 | F2: pack trust | a live security hole; small fix |
| 3 | F4: plugin sandbox (or disable plugins) | security; small if plugins are disabled first |
| 4 | F3 interim: warn on or hide redaction | stops a data leak while the real fix is made |
| 5 | F5: statuses back to in-progress, gaps written | makes the records honest before more work |
| 6 | F6: record budget baselines | stops further regressions |
| 7 | F7: DD entries for the batch's decisions | you decide which choices stay |
| 8 | F3: move the PDF tools onto MuPDF, redaction first | the largest piece of work |
| 9 | F5: replace mocks with real backends | feature by feature, each accepted by hand |
| 10 | F6: cold start and memory | after the startup path stops changing |
| 11 | F8 to F12 | alongside the rest |

## 7. Process suggestions

- **One tag per commit, checks per phase:** keep the AGENTS.md flow. Large unreviewed batches are where every critical
  finding here came from.
- **Shipped means accepted:** only the maintainer moves a feature to `shipped`; agents stop at `in-progress` with
  honest `open` notes.
- **No mocks outside tests:** a mock type in `excellent-app` should fail CI (a check for `Mock` in non-test code of
  the app crate).
- **CI must be green before the next push:** if a push turns CI red, fix it before adding more.
- **Every agent decision gets a DD entry**, as DD5 to DD27 were.
