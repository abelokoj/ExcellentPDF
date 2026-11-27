# Ph7: 23/32 | O.12: Save and reuse form values

**Tag:** O.12; **Priority:** P1; **Phase:** 7; **Needs:** O.11; **Code:** `ecclesia-features::forms` (`exchange.rs`, `profiles.rs`)
**Plan:** §27 entry `#### Ph7: 23/32 | O.12`.

## Problem

Users repeatedly enter identical contact, billing, or institutional fields across different PDF forms and often need to export or import structured form responses outside the PDF.

## User story

As a regular form filler, I want to export and import form data in JSON, CSV, FDF, or XFDF and save reusable autofill profiles so a second form with matching field names fills in one step.

## UI sketch

- Form toolbar / menu actions: **Export form data…** (JSON, CSV, FDF, XFDF), **Import form data…**, and **Autofill profile…** (save current values as profile, apply saved profile).

## Mode

Read and Annotate modes on PDFs containing `/AcroForm` fields.

## Speed and size impact

- Startup: zero startup cost; profiles load on demand from SQLite or JSON.
- Installer size: uses existing `serde_json`, `quick-xml`, and `rusqlite` dependencies.

## Design

- **Exchange (`ecclesia_features::forms::exchange`)**:
  - JSON (`export_json` / `import_json`) with `ExportScope` (`AllFields`, `ExportableOnly`, `NonEmptyOnly`).
  - RFC 4180 CSV (`export_csv`, `export_csv_rows` for multi-form batch export, `import_csv`).
  - ISO 19444-1 XFDF (`export_xfdf` / `import_xfdf`) preserving nested `<field name="...">` hierarchies via `quick-xml`.
  - PDF Forms Data Format `%FDF-1.2` (`export_fdf` / `import_fdf`).
  - All importers enforce read-only guards and trigger recalculation via `AcroForm::recalculate`.
- **Profiles (`ecclesia_features::forms::profiles`)**: `FormProfile`, `ProfileMatchMode` (`ExactName`, `NormalizedName` ignoring case/punctuation), alias mapping, `apply_profile`, and `ProfileStore` supporting JSON file and SQLite persistence.

## Acceptance tests

- [x] `round_trips_json_csv_xfdf_and_fdf`: lossless round-trip across JSON, CSV, XFDF, and FDF plus recalculation.
- [x] `autofill_profile_fills_second_form_with_matching_or_normalized_names_and_persists_sqlite`: profile captured from form A fills form B using aliases and normalized names, and survives SQLite round-trip.
