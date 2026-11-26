# Ph5: 30/45 | F.4: Search operators

**Tag:** F.4; **Priority:** P1; **Phase:** 5; **Needs:** F.3; **Code:** `excellent-features::search::query`
**Plan:** §27 entry `#### Ph5: 30/45 | F.4`.

## Problem

Plain keyword search cannot narrow results by annotation tag, color, author, date, page range, boolean logic, fuzzy spelling, or regular expressions.

## User story

As a power user, I want structured search operators (`author:`, `tag:`, `color:`, `after:`, `before:`, `page:>100`, `AND`/`OR`/`NOT`, `"phrase"`, wildcards, `~`, `/regex/`), so that I can pinpoint exact passages and annotations.

## UI sketch

- Query input bar in global search with inline validation diagnostics showing the character position and reason when a query is malformed.

## Mode

All modes.

## Speed and size impact

- Startup: zero startup cost; pure Rust parser compiles on demand.
- Budgets touched: translates boolean/phrase/prefix queries directly to FTS5 + indexed SQL filters; regex/fuzzy run only on SQL-filtered candidate rows.
- Installer size: 0 bytes added (zero-dependency pure Rust regex and Levenshtein matchers).

## Design

- Model: `excellent-features::search::query::{ParsedQuery, QueryExpr, QueryParseError, RegexPattern, PageFilter}`.
- Storage: translates to `excellent_store::fts::SearchFilters` and FTS5 `MATCH` strings.

## Acceptance tests

- [x] `parses_all_operators_and_translates_to_fts5`: Verifies `author:`, `tag:`, `color:`, `after:`, `before:`, `page:>100`, `kind:`, phrases, prefix wildcards, and boolean operators.
- [x] `fuzzy_wildcard_and_regex_evaluate_accurately`: Verifies fuzzy `~`, glob `*`, and `/regex/` matching on candidate hits.
- [x] `malformed_queries_produce_helpful_errors`: Verifies clear diagnostic messages for unclosed quotes/regexes/parens and invalid operators.
