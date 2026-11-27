# Ph5: 14/45 | A.27: Annotation history timeline

**Tag:** A.27; **Priority:** P2; **Phase:** 5; **Needs:** K.14; **Code:** `ecclesia-features::annotate::history`
**Plan:** §27 entry `#### Ph5: 14/45 | A.27`.

## Problem

Once an annotation is modified or deleted in an earlier session, standard undo may no longer reach it directly or show who changed what and when.

## User story

As a reader, I want an annotation history timeline built from the K.14 operation log so I can scrub to any point in time and restore a single annotation or the whole document as a new operation.

## UI sketch

- Annotations / Document history drawer with chronological timeline slider, author/timestamp badges, "Restore this annotation", and "Restore all to this point".

## Mode

Annotate.

## Speed and size impact

- Startup: nothing loads until the history timeline is opened.
- Budgets touched: reads `.oplog` NDJSON sequentially on demand.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::annotate::history::{AnnotationTimeline, HistoryEntry, HistoryError}` producing standard `ecclesia_core::edit::Operation`s for non-destructive restore.

## Acceptance tests

- [x] `restoring_an_annotation_deleted_a_week_ago_works`: Parses a multi-session oplog, scrubs before and after deletion, and emits a `CreateAnnotation` restore `Operation` for an annotation deleted a week ago.
