# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

EcclesiaPDF is a native app (Rust with NSH, not a website). The value is `web` because it keeps **one design language
on every OS** (maintainer, 2026-11-14): the same tab strip, panels, tokens and themes on Windows, macOS, Linux and
tablets; only window controls and system dialogs are native. Windows first (plan D10).

## Users

Students, teachers and researchers who read, annotate, write on and study documents for hours at a time, on a desktop
or a pen tablet (the maintainer tests pen work on a Surface Pro). Their job: open any document quickly, understand it
deeply, work with it precisely, and find every useful thing again.

## Product Purpose

The most complete reader, annotator, note-taking, study and research app on any platform, and still the fastest to
open, the smallest to download and the easiest to use. Anything people rely on in Samsung Notes, GoodNotes,
Notability, OneNote, Xodo, PDF Expert, MarginNote, LiquidText or Flexcil should exist here, on PDFs and on blank
notebooks alike. Success: it can be used every day for reading (Phase 1), then annotation and ink with nothing lost.

## Positioning

No other product combines SumatraPDF's speed and small size, PDF-XChange's annotation depth, GoodNotes' pen feel,
LiquidText's excerpt workspace, a modern calm and colorful interface, one codebase across desktops and tablets, and
offline use with no account and open file formats. North star: invisible when reading, extraordinarily capable when
needed, never trading capability for speed.

## Operating Context

Long reading and study sessions with PDFs (papers, books, scans, forms, slides) and other MuPDF formats (XPS, EPUB,
MOBI, FB2, CBZ, SVG, images). Many documents open at once in tabs, tab groups, split views and workspaces. Mouse and
keyboard, and pen. The reference parity target is SumatraPDF Enhanced (`dev notes/sumatra_pdf_enhanced_plan.md`).

## Capabilities and Constraints

- The four rules, every feature: blazingly fast (hard budgets in CI, plan §13.1), small (core installer at most
  15 MB, everything optional is a signed pack), easy (clean by default, progressive disclosure, every action reachable
  by toolbar or menu, shortcut, and command palette), vibrant but readable (color in the chrome, the document stays
  neutral; WCAG AA).
- Local-first and trustworthy: works offline with no account; source files change only on an explicit save;
  annotations are standard PDF annotations.
- Modes with their own toolbar and accent: Read (blue), Annotate (amber), Write (violet), Study (green), Present
  (coral).
- Layout follows Vivaldi (plan §10.1): tabs in the title bar, a workspace switcher at their left, a tab-list button at
  their right, an optional icon rail opening the left sidebar, the page canvas in the center, an inspector on the
  right, and a command palette.
- The page is drawn by a native canvas component; NSH draws the chrome around it.
- Open decisions: the licence (Q1) and the app id are the maintainer's to settle.

## Brand Commitments

The name EcclesiaPDF. Calm chrome with joyful accents; the document is the hero. An original duotone icon set that
recolors with the theme; one bundled UI font in the core (more fonts are packs).

## Evidence on Hand

The plan (`dev notes/EcclesiaPDF_detailed_plan_nov_9th.md`), the maintainer's layout references
(`dev notes/images/ui-reference-vivaldi-layout.jpg`, `ui-reference-vivaldi-tab-list.png`) and the test corpus
(`tests/corpus/`). There are no users, testimonials, benchmarks against competitors or press yet; do not invent them.

## Product Principles

1. The document is the hero: chrome recedes while reading and appears when it is needed.
2. Speed is a feature: nothing runs or draws that the user can't see, and every budget is measured.
3. Nothing is lost: every change is undoable, saved safely and recoverable.
4. Three ways to everything: toolbar or menu, keyboard, command palette.
5. Open by default: standard formats, offline, no account.

## Accessibility & Inclusion

Full keyboard operation with visible focus; screen readers on the chrome (NSH) and on the page canvas (UI Automation
with the Text pattern on Windows); high-contrast and color-blind-safe themes; UI scaling; reduced motion; WCAG AA
contrast checked in CI for every theme (plan §15, K.17).
