# <TAG> <Title>

**Tag:** <TAG> · **Priority:** P? · **Phase:** ? · **Needs:** <tags> · **Code:** <crates and modules>
**Plan:** §27 entry `#### <TAG>` and the sections it cites.

## Problem

What the user can't do today, and why it matters.

## User story

As a <kind of user>, I want <action> so that <outcome>.

## UI sketch

Where it appears (menu or toolbar, shortcut, command palette entry), what the user sees, and its states (empty,
loading, error, offline). Every control has an accessible name and keyboard access.

## Mode

Which modes it appears in (plan §10) and whether it is visible by default.

## Speed and size impact

- Startup: what loads, and when (nothing at startup unless the feature is visible).
- Budgets touched (plan §13.1) and the expected change.
- Installer size, and anything that belongs in a pack instead.

## Design

Model (pure Rust, no UI or OS types), view (NSH components), command id (K.16), settings (with defaults), storage.

## Acceptance tests

Copied from the entry's **Done when**, one per line, each with the test that proves it:

- [ ] `<test_name>`: <check>
