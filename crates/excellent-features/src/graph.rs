//! Knowledge graph model connecting documents, notes `[[links]]`, tags, and citations,
//! with a Barnes-Hut force-directed layout solver and cluster filtering (plan F.14).

pub mod layout;
pub mod query;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use excellent_store::notes::{LinkTarget, Note, extract_wiki_links, parse_link_target};

use crate::citations::CitationEntry;

use self::layout::{LayoutConfig, LayoutMetrics, step_barnes_hut};

/// Semantic category of a node in [`KnowledgeGraph`] (plan F.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum GraphNodeKind {
    /// A PDF or document in the library.
    Document,
    /// A Markdown note.
    Note,
    /// A shared tag `#tag`.
    Tag,
    /// A bibliographic citation record.
    Citation,
}

/// Semantic category of an edge in [`KnowledgeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum GraphEdgeKind {
    /// Note-to-note `[[wiki-link]]`.
    WikiLink,
    /// Note or citation anchored to a document.
    AnchoredTo,
    /// Document or note tagged with `#tag`.
    HasTag,
    /// Document or note citing a [`CitationEntry`].
    Cites,
}

/// A node inside [`KnowledgeGraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    /// Unique identifier (`doc:<path>`, `note:<id>`, `tag:<name>`, `cite:<key>`).
    pub id: String,
    /// Human-readable display label.
    pub label: String,
    /// Node category.
    pub kind: GraphNodeKind,
    /// Cluster label used for color-coding and filtering.
    pub cluster: String,
    /// World X position.
    pub x: f64,
    /// World Y position.
    pub y: f64,
    /// Current X velocity.
    pub vx: f64,
    /// Current Y velocity.
    pub vy: f64,
    /// Node mass for Barnes-Hut repulsion.
    pub mass: f64,
    /// Associated tags.
    pub tags: Vec<String>,
    /// Whether the node position is pinned during layout steps.
    pub pinned: bool,
}

/// A directed edge inside [`KnowledgeGraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdge {
    /// Source node id.
    pub source: String,
    /// Target node id.
    pub target: String,
    /// Relationship kind.
    pub kind: GraphEdgeKind,
    /// Spring weight (`1.0` default).
    pub weight: f64,
}

/// Document input descriptor for [`KnowledgeGraph::from_library`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentGraphInput {
    /// Document path.
    pub path: PathBuf,
    /// Display title.
    pub title: String,
    /// Document tags.
    pub tags: Vec<String>,
    /// Citation keys referenced by this document.
    pub cited_keys: Vec<String>,
    /// Optional collection or cluster name.
    pub cluster: Option<String>,
}

/// Interactive knowledge graph connecting documents, notes, tags, and citations (plan F.14).
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraph {
    nodes: Vec<GraphNode>,
    index_by_id: BTreeMap<String, usize>,
    edges: Vec<GraphEdge>,
    edge_keys: BTreeSet<(String, String, GraphEdgeKind)>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
}

impl KnowledgeGraph {
    /// Creates an empty [`KnowledgeGraph`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Whether the graph contains no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Inserts or updates `node`, preserving deterministic indexing.
    pub fn upsert_node(&mut self, node: GraphNode) {
        if let Some(&idx) = self.index_by_id.get(&node.id) {
            self.nodes[idx] = node;
        } else {
            let idx = self.nodes.len();
            self.index_by_id.insert(node.id.clone(), idx);
            self.adjacency.entry(node.id.clone()).or_default();
            self.nodes.push(node);
        }
    }

    /// Adds a deduplicated edge between existing `source` and `target` nodes.
    ///
    /// Returns `true` if inserted, `false` if either endpoint is missing or the edge already exists.
    pub fn add_edge(&mut self, edge: GraphEdge) -> bool {
        if !self.index_by_id.contains_key(&edge.source)
            || !self.index_by_id.contains_key(&edge.target)
        {
            return false;
        }
        let key = (edge.source.clone(), edge.target.clone(), edge.kind);
        if !self.edge_keys.insert(key) {
            return false;
        }
        self.adjacency
            .entry(edge.source.clone())
            .or_default()
            .insert(edge.target.clone());
        self.adjacency
            .entry(edge.target.clone())
            .or_default()
            .insert(edge.source.clone());
        self.edges.push(edge);
        true
    }

    /// Looks up a node by `id`.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.index_by_id.get(id).map(|&idx| &self.nodes[idx])
    }

    /// Iterates over all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.iter()
    }

    /// Iterates over all edges.
    pub fn edges(&self) -> impl Iterator<Item = &GraphEdge> {
        self.edges.iter()
    }

    /// Iterates over neighbor node ids connected to `id`.
    pub fn neighbors(&self, id: &str) -> impl Iterator<Item = &str> {
        self.adjacency
            .get(id)
            .into_iter()
            .flat_map(|set| set.iter().map(String::as_str))
    }

    /// Advances the force-directed layout by one Barnes-Hut iteration.
    pub fn step_layout(&mut self, config: &LayoutConfig) -> LayoutMetrics {
        let indexed_edges: Vec<(usize, usize, f64)> = self
            .edges
            .iter()
            .filter_map(|e| {
                let s = *self.index_by_id.get(&e.source)?;
                let t = *self.index_by_id.get(&e.target)?;
                Some((s, t, e.weight))
            })
            .collect();
        step_barnes_hut(&mut self.nodes, &indexed_edges, config)
    }

    /// Runs `iterations` layout steps and returns the final [`LayoutMetrics`].
    pub fn run_layout(&mut self, iterations: usize, config: &LayoutConfig) -> LayoutMetrics {
        let mut last = LayoutMetrics::default();
        for _ in 0..iterations {
            last = self.step_layout(config);
        }
        last
    }

    /// Builds a [`KnowledgeGraph`] from library `documents`, `notes` (extracting `[[links]]`), and `citations`.
    #[must_use]
    pub fn from_library(
        documents: &[DocumentGraphInput],
        notes: &[Note],
        citations: &[CitationEntry],
    ) -> Self {
        let mut graph = Self::new();
        let mut note_id_by_title = BTreeMap::<String, String>::new();
        let mut doc_id_by_name = BTreeMap::<String, String>::new();

        for (i, doc) in documents.iter().enumerate() {
            let doc_id = format!("doc:{}", doc.path.to_string_lossy());
            let cluster = doc
                .cluster
                .clone()
                .unwrap_or_else(|| "documents".to_owned());
            let (x, y) = golden_spiral_pos(i);
            graph.upsert_node(GraphNode {
                id: doc_id.clone(),
                label: doc.title.clone(),
                kind: GraphNodeKind::Document,
                cluster,
                x,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: 1.5,
                tags: doc.tags.clone(),
                pinned: false,
            });
            doc_id_by_name.insert(
                doc.path.to_string_lossy().to_ascii_lowercase(),
                doc_id.clone(),
            );
            if let Some(fname) = doc.path.file_name().and_then(|s| s.to_str()) {
                doc_id_by_name.insert(fname.to_ascii_lowercase(), doc_id.clone());
            }
            for tag in &doc.tags {
                let tid = ensure_tag_node(&mut graph, tag);
                graph.add_edge(GraphEdge {
                    source: doc_id.clone(),
                    target: tid,
                    kind: GraphEdgeKind::HasTag,
                    weight: 0.8,
                });
            }
        }

        for (i, cite) in citations.iter().enumerate() {
            let cid = format!("cite:{}", cite.key);
            let (x, y) = golden_spiral_pos(documents.len() + i);
            graph.upsert_node(GraphNode {
                id: cid,
                label: if cite.title.is_empty() {
                    cite.key.clone()
                } else {
                    cite.title.clone()
                },
                kind: GraphNodeKind::Citation,
                cluster: "citations".to_owned(),
                x,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: 1.2,
                tags: Vec::new(),
                pinned: false,
            });
        }

        for doc in documents {
            let doc_id = format!("doc:{}", doc.path.to_string_lossy());
            for key in &doc.cited_keys {
                let cid = format!("cite:{key}");
                graph.add_edge(GraphEdge {
                    source: doc_id.clone(),
                    target: cid,
                    kind: GraphEdgeKind::Cites,
                    weight: 1.0,
                });
            }
        }

        for (i, note) in notes.iter().enumerate() {
            let nid = format!("note:{}", note.id);
            note_id_by_title.insert(note.title.to_ascii_lowercase(), nid.clone());
            let (x, y) = golden_spiral_pos(documents.len() + citations.len() + i);
            graph.upsert_node(GraphNode {
                id: nid.clone(),
                label: note.title.clone(),
                kind: GraphNodeKind::Note,
                cluster: "notes".to_owned(),
                x,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: 1.0,
                tags: note.tags.clone(),
                pinned: false,
            });

            if let Some(file) = &note.file {
                let doc_key = file.to_string_lossy().to_ascii_lowercase();
                if let Some(target_doc) = doc_id_by_name.get(&doc_key) {
                    graph.add_edge(GraphEdge {
                        source: nid.clone(),
                        target: target_doc.clone(),
                        kind: GraphEdgeKind::AnchoredTo,
                        weight: 1.2,
                    });
                }
            }

            for tag in &note.tags {
                let tid = ensure_tag_node(&mut graph, tag);
                graph.add_edge(GraphEdge {
                    source: nid.clone(),
                    target: tid,
                    kind: GraphEdgeKind::HasTag,
                    weight: 0.7,
                });
            }
        }

        for note in notes {
            let src_id = format!("note:{}", note.id);
            for raw in extract_wiki_links(&note.body) {
                match parse_link_target(&raw) {
                    LinkTarget::Note { title } => {
                        if let Some(target_id) = note_id_by_title.get(&title.to_ascii_lowercase()) {
                            graph.add_edge(GraphEdge {
                                source: src_id.clone(),
                                target: target_id.clone(),
                                kind: GraphEdgeKind::WikiLink,
                                weight: 1.0,
                            });
                        }
                    }
                    LinkTarget::Document { doc, .. } => {
                        if let Some(target_id) = doc_id_by_name.get(&doc.to_ascii_lowercase()) {
                            graph.add_edge(GraphEdge {
                                source: src_id.clone(),
                                target: target_id.clone(),
                                kind: GraphEdgeKind::AnchoredTo,
                                weight: 1.0,
                            });
                        }
                    }
                }
            }
        }

        graph
    }
}

fn ensure_tag_node(graph: &mut KnowledgeGraph, tag: &str) -> String {
    let tid = format!("tag:{}", tag.to_ascii_lowercase());
    if graph.node(&tid).is_none() {
        let (x, y) = golden_spiral_pos(graph.node_count());
        graph.upsert_node(GraphNode {
            id: tid.clone(),
            label: format!("#{tag}"),
            kind: GraphNodeKind::Tag,
            cluster: "tags".to_owned(),
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            mass: 0.9,
            tags: vec![tag.to_owned()],
            pinned: false,
        });
    }
    tid
}

fn golden_spiral_pos(index: usize) -> (f64, f64) {
    let angle = (index as f64) * 2.399_963_229_728_653;
    let radius = 18.0 * ((index + 1) as f64).sqrt();
    (radius * angle.cos(), radius * angle.sin())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::citations::EntryKind;

    use super::query::{
        GraphFilter, OrphanVisibility, connected_components, filter_graph, neighborhood,
    };
    use super::*;

    #[test]
    fn library_builds_connected_graph_across_docs_notes_tags_and_citations() {
        let docs = vec![DocumentGraphInput {
            path: PathBuf::from("papers/crdt.pdf"),
            title: "Conflict-Free Replicated Data Types".to_owned(),
            tags: vec!["distributed".to_owned()],
            cited_keys: vec!["shapiro2011".to_owned()],
            cluster: Some("systems".to_owned()),
        }];
        let notes = vec![
            Note {
                id: "n1".to_owned(),
                title: "CRDT Notes".to_owned(),
                body: "Builds on [[RGA]] and [[crdt.pdf#page=2]].".to_owned(),
                file: Some(PathBuf::from("papers/crdt.pdf")),
                page: Some(1),
                annotation_id: None,
                tags: vec!["distributed".to_owned()],
                created_at: 1,
                modified_at: 1,
            },
            Note {
                id: "n2".to_owned(),
                title: "RGA".to_owned(),
                body: "Sequence CRDT".to_owned(),
                file: None,
                page: None,
                annotation_id: None,
                tags: Vec::new(),
                created_at: 2,
                modified_at: 2,
            },
        ];
        let citations = vec![CitationEntry {
            key: "shapiro2011".to_owned(),
            kind: EntryKind::TechReport,
            title: "CRDTs".to_owned(),
            ..CitationEntry::default()
        }];

        let mut graph = KnowledgeGraph::from_library(&docs, &notes, &citations);
        assert_eq!(graph.node_count(), 5);
        assert!(graph.edge_count() >= 5);

        let metrics = graph.run_layout(5, &LayoutConfig::default());
        assert!(metrics.kinetic_energy.is_finite());

        let hood = neighborhood(&graph, "note:n1", 2);
        assert!(hood.contains(&"note:n2".to_owned()));
        assert!(hood.contains(&"doc:papers/crdt.pdf".to_owned()));

        let mut kinds = BTreeSet::new();
        kinds.insert(GraphNodeKind::Note);
        let filtered = filter_graph(
            &graph,
            &GraphFilter {
                kinds,
                orphan_visibility: OrphanVisibility::Include,
                ..GraphFilter::default()
            },
        );
        assert_eq!(filtered.nodes.len(), 2);
        assert_eq!(connected_components(&graph).len(), 1);
    }

    #[test]
    fn barnes_hut_layout_keeps_5000_nodes_interactive() {
        let mut graph = KnowledgeGraph::new();
        for i in 0..5000 {
            let (x, y) = golden_spiral_pos(i);
            graph.upsert_node(GraphNode {
                id: format!("n-{i}"),
                label: format!("Node {i}"),
                kind: GraphNodeKind::Note,
                cluster: format!("c-{}", i % 8),
                x,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: 1.0,
                tags: Vec::new(),
                pinned: false,
            });
            if i > 0 {
                graph.add_edge(GraphEdge {
                    source: format!("n-{i}"),
                    target: format!("n-{}", i / 2),
                    kind: GraphEdgeKind::WikiLink,
                    weight: 1.0,
                });
            }
        }
        assert_eq!(graph.node_count(), 5000);

        let start = Instant::now();
        let metrics = graph.step_layout(&LayoutConfig::default());
        let elapsed = start.elapsed();

        assert!(metrics.max_displacement.is_finite());
        assert!(
            elapsed.as_millis() < 2000,
            "5000-node Barnes-Hut step should complete interactively, took {elapsed:?}"
        );
    }
}
