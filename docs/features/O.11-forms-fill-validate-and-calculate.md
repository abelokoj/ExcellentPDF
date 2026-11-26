# Ph7: 22/32 | O.11: Forms: fill, validate and calculate

**Tag:** O.11; **Priority:** P1; **Phase:** 7; **Needs:** K.14, U.8; **Code:** `excellent-core::forms` (`model.rs`, `calc.rs`, `pdf_codec.rs`), `excellent-features::forms::session`
**Plan:** §27 entry `#### Ph7: 22/32 | O.11`.

## Problem

Interactive PDF forms (tax returns, expense claims, applications, lab sheets) require typing into fields, toggling checkboxes/radios/lists, validating required inputs, and running Acrobat-compatible calculations before saving.

## User story

As a reader filling a PDF form, I want to navigate fields in tab order, have required and formatted inputs validated, watch calculated totals update automatically, and save filled values so other PDF readers display them identically.

## UI sketch

- Interactive widget overlays in the page view, plus a "Highlight fields" toggle distinguishing fillable, required-empty, invalid, calculated, and read-only fields.
- Inline validation messages when a field fails required, range, length, date, email, or regex rules.
- Read-only notice when a form uses XFA without an AcroForm fallback.

## Mode

Read and Annotate modes whenever the open PDF contains an `/AcroForm` dictionary.

## Speed and size impact

- Startup: zero startup cost; form parsing runs lazily when a PDF with `/AcroForm` is opened.
- Installer size: pure Rust implementation using existing workspace dependencies.

## Design

- **Core model (`excellent_core::forms::model`)**: Models `Text`, `Checkbox`, `RadioGroup`, `Choice` (`Combo` / `List`), `Signature`, and `Pushbutton` fields with `/Ff` `FieldFlags`, `/Opt`, `/DA`, `/Q`, `/TM`, `/TU`, `/XFA` detection, geometric/explicit tab ordering, bounded pure-Rust regex validation, and incremental PDF `/AcroForm` + `/AP` stream serialization (`to_pdf_bytes`).
- **Calculation DAG (`excellent_core::forms::calc`)**: Safe, bounded AcroForm JS-subset evaluator supporting `AFSimple_Calculate` (`SUM`, `AVG`, `MIN`, `MAX`, `PRD`), `AFNumber_Format`, `AFPercent_Format`, `AFDate_FormatEx`, `AFRange_Validate`, and arithmetic `getField("...").value` expressions with topological sorting (`/CO` tie-breaking) and cycle detection.
- **Session (`excellent_features::forms::session`)**: `FormSession` manages active tab focus, dirty tracking, field highlight tones, reset-to-defaults, and validation reporting.

## Acceptance tests

- [x] `parses_fills_calculates_validates_and_round_trips_acroform`: parses widgets, evaluates calculations, validates, saves incrementally, and reopens in both `AcroForm` and MuPDF.
- [x] `calc_dag_supports_af_simple_calculate_and_detects_cycles`: topological DAG evaluation and cycle rejection.
- [x] `validation_checks_required_range_length_email_date_and_regex`: required, range, email, date, and regex rules.
- [x] `tab_order_respects_page_and_top_left_geometry`: deterministic page/row/column tab traversal.
