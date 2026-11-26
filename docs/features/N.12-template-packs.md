# Ph5: 21/45 | N.12: Template packs

**Tag:** N.12; **Priority:** P2; **Phase:** 5; **Needs:** K.21, N.1; **Code:** `excellent-features::notebook::packs`
**Plan:** §27 entry `#### Ph5: 21/45 | N.12: Template packs`.

## Problem

Bundling dozens of specialized paper templates (French Seyès, isometric, music sheets, custom PDF backgrounds) in the core installer bloats installer size.

## User story

As a notebook user, I want to install and remove template packs from the notebook dialog and see the template list update immediately without restarting the app.

## UI sketch

- New Notebook / Insert Page dialog includes **Template Packs…** browser showing installed and available packs, previews, Install, and Remove buttons.

## Mode

Write (notebook creation and page insertion).

## Speed and size impact

- Startup: zero cost; `TemplatePackCatalog` scans `packs/templates/` lazily when the template picker opens.
- Installer size: optional packs stay outside the core binary (`packs/templates/`).

## Design

- **Model (`excellent-features::notebook::packs`):**
  - `TemplatePackManifest` supporting both `PackTemplateKind::Procedural` (`PaperSpec` presets) and `PackTemplateKind::PdfBackground` (validated via MuPDF on install).
  - `TemplatePackCatalog` provides atomic zip-archive installation (`install_zip_bytes`), `remove`, and live `list_templates` updates without restart.

## Acceptance tests

- [x] `notebook::packs::tests::install_and_remove_pack_updates_template_list_without_restart`: verifies installing a pack with procedural + PDF background templates updates `list_templates()` immediately, and removing it drops those entries without restart.
