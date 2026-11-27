# Ph5: 37/45 | F.11: Zotero integration

**Tag:** F.11; **Priority:** P1; **Phase:** 5; **Needs:** F.10; **Code:** `ecclesia-features::citations::zotero`
**Plan:** §27 entry `#### Ph5: 37/45 | F.11`.

## Problem

Zotero users want to browse their Zotero items, open attached PDFs in EcclesiaPDF with metadata shown, and optionally send annotations back as Zotero notes.

## User story

As a Zotero user, I want EcclesiaPDF to read my Zotero library via Zotero 7's local API, local `zotero.sqlite`, or Web API v3, so that I can open Zotero PDFs and sync notes seamlessly.

## UI sketch

- Zotero source picker in the Library sidebar: auto-detects Zotero 7 on `127.0.0.1:23119`, falls back to local `zotero.sqlite` or Web API v3 with API key.
- Optional "Send annotations to Zotero as note" action in the document inspector.

## Mode

All modes.

## Speed and size impact

- Startup: nothing loads until the Zotero panel is opened.
- Budgets touched: network calls run on background workers via `ureq`; SQLite opened read-only.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::citations::zotero::{ZoteroClient, ZoteroSource, ZoteroItem, read_local_sqlite, parse_zotero_items_json, build_child_note_payload}`.

## Acceptance tests

- [x] `parses_zotero_api_v3_json_and_builds_child_note`: Parses Zotero API v3 JSON items and builds valid child note payloads.
- [x] `reads_local_zotero_sqlite_schema_read_only`: Reads Zotero items, creators, collections, tags, and `storage:` PDF attachments from `zotero.sqlite` read-only.
