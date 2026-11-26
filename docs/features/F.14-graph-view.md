# Ph9: 4/13 | F.14: Graph view

**Tag:** F.14; **Priority:** P2; **Phase:** 9; **Needs:** F.12; **Code:** `excellent-features::graph`
**Plan:** §27 entry `#### Ph9: 4/13 | F.14`.

## Problem

As a library of PDFs, `[[wiki-linked]]` notes, tags, and citations grows, linear lists hide cross-document clusters and conceptual bridges.

## User story

As a researcher, I want an interactive force-directed knowledge graph connecting documents, notes, tags, and citations with cluster and neighborhood filtering so that I can explore relationships across thousands of items smoothly.

## UI sketch

- Graph View tab / panel rendered on NSH's canvas element with zoom/pan, node kind/cluster/tag filter chips, search box, and neighborhood depth slider.
- Accessible node list fallback and keyboard focus traversal.

## Mode

Research, Study, Notes.

## Speed and size impact

- Startup: graph construction and Barnes-Hut quadtree allocate only when Graph View opens.
- Budgets touched: O(N log N) Barnes-Hut step solver keeps 5,000+ nodes interactive.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::graph::{KnowledgeGraph, GraphNode, GraphEdge, GraphNodeKind, GraphEdgeKind}` + `graph::layout::LayoutConfig` (Barnes-Hut quadtree) + `graph::query::{GraphFilter, filter_graph, neighborhood, connected_components}`.

## Acceptance tests

- [ ] `library_builds_connected_graph_across_docs_notes_tags_and_citations`: Connects documents, `[[wiki-links]]`, tags, and citations with cluster filtering and neighborhood BFS.
- [ ] `barnes_hut_layout_keeps_5000_nodes_interactive`: Runs Barnes-Hut force-directed layout on 5,000 nodes cleanly within interactive latency bounds.
