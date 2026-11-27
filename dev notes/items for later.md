# Items for later

Ideas and findings to come back to. Not yet in the plan (`EcclesiaPDF_detailed_plan_nov_9th.md`); each item says
what would change there if adopted.

---

## L1. Plugins: themes, skins, interface and features from other developers (noted 2026-11-13)

**Goal:** other people can write plugins that add skins, themes, interface and new functions to EcclesiaPDF.

### What already makes it possible

NSH is built for this. Its own `AGENTS.md` says plugins for native apps use the same machinery as embedded apps. A
Rust app such as EcclesiaPDF can load **signed TSX packages** (`.nshpkg`) written by others at run time. Each package:

- **Runs in a sandbox** (its own QuickJS VM) with memory, time and node limits enforced in Rust.
- **Has capabilities:** it declares what it needs (network hosts, files, web views, host functions); the app grants
  at most what it chooses (`Policy.granted` on the bundle). It is refused before running if its signature,
  certificate, revocations or build number fail.
- **Can use our UI pieces:** it imports the Rust components, modules and classes our crate exports
  (`#[native_module]`), so a plugin's UI looks and works like ours.
- **Can call into the app** through the host channel (`host.call`, which rejects on failure and never throws).
- **Can take our theme** (`AppView::set_theme_overrides`).

On Windows this works in the setup the plan already uses: `nsh-platform-windows`'s `AppView` (a child window) loads
the bundle that `nsh_engine::load_package` / `verify_package` returns.

Sources: `C:\dev\nsh-src\website\docs\signed-packages.md`, `sandbox-and-capabilities.md`, `rust.md` ("Extending a
Rust app with TSX"), `windows.md` ("Embedding in a Win32 app"); `crates/nsh-engine/src/lib.rs` (`load_package`,
`Policy`).

The plan also has pieces a plugin system can sit on:

- **K.16 command table:** every action has one entry, so plugins can add commands, shortcuts and menu items.
- **E.2 tool table:** typed document actions (open, find, annotate, fill forms, organize pages) plugins can call.
- **K.17 themes:** themes are already small shareable TOML files.
- **D8 / K.21 packs:** signing, install, update and removal.
- **§14.2:** packs and plugins are signed and checked before loading.

### What is missing

Today **E.5** (Phase 9) is one paragraph: "signed TSX packages … they call the tool table". That covers automation,
not the goal above.

| Wanted | What it needs | In the plan? |
| --- | --- | --- |
| Themes | A theme package type: colours, fonts, reading themes (paper colours, dark-mode settings) | Partly: theme files exist, not as installable add-ons |
| Skins / icon sets | Icon packs plus swappable styles for named UI parts | No |
| New interface | Named places a plugin can fill: toolbar, sidebar panel, context menu, status bar, settings page, start page, dialogs | No |
| New functions | Plugin commands, events (document opened, page changed, selection, annotation added), new annotation tools, import and export formats | Commands only, through E.2 |
| Drawing on the page | An overlay API: the plugin sends shapes in page coordinates and our native page canvas draws them. Plugins never touch the canvas itself (D5) | No |
| Other people publishing | Developer release keys certified by our root key (NSH's two-level key scheme), a plugin manifest, a permissions screen at install, a catalogue, perhaps an unsigned "developer mode" | No: only our own signing |
| Stable plugin API | Versioned extension points so old plugins keep working | No |

### Recommendation

1. **Design the extension points early, ship plugin loading later.** Define the places plugins can fill in Phase 1
   and build our own features through them, so the API is tested by real use. Opening them to outsiders can wait.
2. **Plugins are TSX packages only.** No native Rust or C plugins (DLLs): they bypass the sandbox, break with
   compiler changes and need a build per OS. Heavy work (OCR, AI, rendering) stays in our Rust code; plugins reach it
   through functions we expose.
3. **Three kinds of package:**
   - themes and skins: data only, no code, safe to install freely;
   - UI plugins: fill the named places;
   - feature plugins: commands, tools and formats, with explicit permissions.

### Risks

- **NSH's API is not frozen yet.** No release is recorded in `nsh-compat` (0.3.0–0.5.0 are not), so plugins built
  against 0.5.0 may break when NSH is updated. Our own plugin API must shield plugin authors from that.
- **Each plugin view runs its own JS engine,** so plugins load only when needed, or they break the startup and memory
  budgets (§13.1).
- **Apple's App Store rules** limit downloading code that changes how an app works; iPad plugins may be limited to
  themes and data, or bundled with the app. (Our understanding; not checked against the current guidelines.)

### If adopted, the plan changes

- §11.13 and §27.16: expand **E.5** and add tags for theme packages, UI extension points, plugin events, page
  overlays, developer signing and the catalogue.
- Phase 1 (§18): define the extension points with K.16 and K.17.
- §14.2: third-party signing and the install-time permissions screen.
- §20: the NSH API-stability risk.

**Not checked:** loading a package in a Windows `AppView` was read from NSH's docs and code, not built or run.
