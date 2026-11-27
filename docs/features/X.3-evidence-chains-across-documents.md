# Ph9: 7/13 | X.3: Evidence chains across documents

**Tag:** X.3; **Priority:** P1; **Phase:** 9; **Needs:** X.2; **Code:** `ecclesia-features::canvas_board::evidence`
**Plan:** §27 entry `#### Ph9: 7/13 | X.3` and §11.10.

## Problem

Complex arguments spanning multiple PDFs require tracing how claims rest on intermediate synthesis cards and primary evidence, and checking that reasoning is not circular.

## User story

As a researcher, I want a Chain view that validates my argument graph as a DAG and lists ordered claim -> evidence -> source chains with full citations across all documents.

## UI sketch

- "Evidence Chains" inspector tab inside Canvas Board showing topological validation status, cycle warnings if any, and ordered source-cited chain outlines.

## Mode

Research.

## Speed and size impact

- Startup: runs on demand when opening the Chain view or exporting chains.
- Budgets touched: O(V + E) Kahn's topological sort and bounded path traversal.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::canvas_board::evidence::{validate_evidence_dag, query_evidence_chains, export_evidence_chains_markdown, EvidenceChain, EvidenceStep, SourceCitation}`.

## Acceptance tests

- [ ] `evidence_chains_validate_dag_and_export_cross_doc_citations`: Validates acyclic evidence chains, exports deep-linked source citations, and rejects directed cycles with `BoardError::CycleDetected`.
