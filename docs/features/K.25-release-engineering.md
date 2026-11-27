# Ph1: 11/60 | K.25: Release engineering: installers, updates, crash reports

**Tag:** K.25; **Priority:** P0; **Phase:** 1; **Needs:** K.1; **Code:** `packaging/`, `.github/workflows/windows.yml`
**Plan:** §27 entry `#### Ph1: 11/60 | K.25` and Appendix D.3.9, C.3.6; `dev notes/dev_decisions.md` DD3, DD31. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Users cannot install or update Ecclesia PDF safely without a self-contained installer within the 15 MB core size budget, reproducible release artifacts, signed update metadata, and opt-in crash reporting.

## User story

As a Windows user, I want to download a compact MSI installer that installs cleanly without administrator rights by default (or per-machine when needed), registers the app icon and runtime cleanly, and stays within the promised size budget.

## UI sketch

- Windows MSI installer wizard with Single Package Authoring scope selection (per-user default without UAC, optional per-machine install with UAC; DD31) and standard uninstall entry in Settings > Apps displaying the Nib & Leaf icon (DD28).
- Future update notifications and opt-in crash report consent toggle live in the Settings window (`K.16`).

## Mode

Packaging and distribution outside the document canvas; Settings controls apply across all modes.

## Speed and size impact

- Startup: zero runtime overhead for packaging; installer bundles `ecclesiapdf.exe` plus app-local Visual C++ CRT.
- Budgets touched: core installer size (`<= 15 MB` budget; measured `13.51 MB` on 2026-11-26).
- Optional runtimes and ML weights stay outside the core installer in signed `.expack` archives (D8, DD35).

## Design

- `packaging/windows/build-msi.ps1`: builds the x64 MSI using the Windows Installer API and `makecab` (LZX:21), following DD3 release naming (`EcclesiaPDF-vYY-MM-DD.N-vSEMVER-windows-x64.msi`).
- `packaging/windows/test-msi.ps1`: builds a test MSI into a temp folder, validates MSI 5.0 tables/streams, enforces the `<= 15 MB` budget, and verifies `msiexec /a` unpack SHA-256 payload integrity.
- `.github/workflows/windows.yml`: builds and uploads the test MSI (`-Release 0`) on every push.

## Acceptance tests

- [x] `packaging/windows/test-msi.ps1`: builds the Windows x64 MSI, validates MSI 5.0 schema and UI dialogs, checks the `<= 15 MB` budget, and verifies unpacked payload hashes.
- [ ] Signed binaries/installers for Windows x64 + ARM64, portable zip, macOS notarized DMG, Linux AppImage/Flatpak/deb.
- [ ] TUF-signed auto-updater (`tough`) with rollback and opt-in minidump crash reporting.
- [ ] Tagged `.github/workflows/release.yml` workflow with SBOM generation and maintainer hand-acceptance.

## Open

- Per-user/dual-scope unsigned x64 MSI builds and passes `packaging/windows/test-msi.ps1`; code signing, ARM64, portable zip, macOS/Linux packages, TUF auto-updates, opt-in crash reports, and `.github/workflows/release.yml` remain open.
