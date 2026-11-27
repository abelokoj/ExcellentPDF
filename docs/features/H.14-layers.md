# Ph4: 7/44 | H.14: Layers

**Tag:** H.14; **Priority:** P1; **Phase:** 4; **Needs:** A.22; **Code:** `ecclesia-features::ink::layers`
**Plan:** §8.4 and §27 entry `#### Ph4: 7/44 | H.14`.

## Problem

Complex annotated pages mix freehand ink, highlights, typed text, stamps/images, and working sketches. Without dedicated ink layers that can be shown, hidden, and locked against selection and erasing, erasing a sketch risks wiping out finished diagrams or notes underneath.

## User story

As a student or technical reviewer layering diagrams over a PDF, I want fixed layers (Ink, Highlights, Text, Images) plus custom user layers that I can show, hide, and lock so that locked layers cannot be accidentally selected or erased.

## UI sketch

Shown in the Layers panel (`Ctrl+Alt+L`, palette command `ink.layers`), Write toolbar toggle, and context menu. Lists the four fixed layers (`Ink`, `Highlights`, `Text`, `Images`) followed by user layers, each with Show/Hide toggle, Lock/Unlock toggle, active-layer indicator, and "+ Add layer" button. Every row has an accessible name and keyboard navigation.

## Mode

Write and Annotate.

## Speed and size impact

- Startup: nothing runs at startup; `LayerStack::from_ocgs` runs only when a document is opened or the Layers panel is shown.
- Budgets touched: zero impact on rendering or ink latency (simple slice filtering in microseconds).
- Installer size: negligible (~5 KB compiled Rust, zero new dependencies).

## Design

- **Model (`ecclesia-features::ink::layers`):**
  - `FixedLayer` (`Ink`, `Highlights`, `Text`, `Images`) with automatic classification of untagged annotations (`FixedLayer::classify`).
  - `LayerStack` synchronizes with A.22 PDF optional content groups (`ecclesia_mupdf::Layer`) and emits `Change::SetLayer` for persistence in standard PDF `/OCProperties`.
  - Visibility (`Visibility::Visible | Hidden`) and lock (`LockState::Unlocked | Locked`) controls enforce that locked or hidden layers cannot be selected (`can_select`, `filter_selection`), erased by stroke/area/highlighter erasers (`can_erase`, `erase_changes`), moved out of, or drawn onto (`set_active`, `place_new`).
- **Command ids (for app wiring):** `ink.layers` (`Ctrl+Alt+L`), `ink.layer_add`, `ink.layer_toggle_visible`, `ink.layer_toggle_lock`.
- **Storage:** standard PDF optional content groups (`/OCProperties` + annotation `/OC`) via A.22.

## Acceptance tests

- [x] `locked_layers_cannot_be_selected_or_erased`: verifies that lasso, marquee, stroke eraser, area eraser, and highlighter eraser exclude annotations on locked user and fixed layers while acting normally on unlocked layers, and resume once unlocked.
- [x] `fixed_layers_classify_kinds_and_round_trip_through_ocgs`: verifies all four fixed layer classifications, OCG synchronization, visibility lock enforcement, and active-layer placement.
