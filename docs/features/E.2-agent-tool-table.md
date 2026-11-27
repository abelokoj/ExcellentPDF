# Ph5: 42/45 | E.2: Agent tool table and `ecclesia-cli run`

**Tag:** E.2; **Priority:** P1; **Phase:** 5; **Needs:** E.1; **Code:** `ecclesia-core::tools` (`tools/registry.rs`, `tools/schema.rs`, `tools/builtin.rs`), `ecclesia-cli::ocr_tools`
**Plan:** §27 entry `#### Ph5: 42/45 | E.2` and Appendix C.3.1.

## Problem

CLI batch automation, MCP servers, UI control scripts, and future plugins all need one canonical, schema-validated tool surface over the PDF engine instead of duplicating dispatch logic.

## User story

As an agent or script author, I want every engine capability exposed with JSON Schema input/output definitions and runnable via `ecclesia-cli run` so tools are self-describing and validated before execution.

## UI sketch

- `ecclesia-cli run --list [--json]` prints every tool spec, description, and input/output schema.
- `ecclesia-cli run <tool> --json '<args>'` executes one tool; `ecclesia-cli run <script.json>` executes a call or array of calls sequentially.

## Mode

Core engine API (`ecclesia-core::tools`) and CLI runner (`ecclesia-cli run`).

## Speed and size impact

- Schema validation runs in-process with zero extra crates beyond `serde_json`.

## Design

- **`ToolRegistry`**: `BTreeMap<String, RegisteredTool>` mapping dotted tool names (`doc.info`, `doc.extract_text`, `doc.search`, `doc.outline`, `annot.list`, `annot.add`, `annot.export`, `doc.merge`, `doc.split`, `doc.compress`, `doc.ocr`, `page.render`, `doc.attachments`, `doc.save`, `doc.redact`) to `ToolSpec` and `Arc<dyn ToolHandler>`.
- **`validate_json`**: Validates runtime arguments against `input_schema` (`type`, `required`, `properties`, `additionalProperties`, `enum`, numeric bounds, array/string constraints).
- **`ToolContext`**: Holds shared `Registry` and optional `root_dir` confinement.

## Acceptance tests

- [x] `validates_nested_object_and_rejects_bad_types`: JSON Schema validator unit test.
- [x] `every_tool_example_matches_its_input_schema`: verifies every tool's `example_input` passes `validate_json` against its `input_schema`.
- [x] `all_full_registry_tools_have_valid_schemas_and_examples`: verifies the full `ecclesia-cli` registry including `doc.ocr` and `annot.export`.
- [x] `core_tools_execute_on_corpus_documents`: executes `doc.info`, `doc.extract_text`, `doc.outline`, `annot.list`, and `page.render` against `base14-outline-annot.pdf`.
