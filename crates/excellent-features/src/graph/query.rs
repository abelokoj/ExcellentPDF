//! Cluster filtering, neighborhood queries, and connected components for [`super::KnowledgeGraph`] (plan F.14).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{GraphEdge, GraphNode, GraphNodeKind, KnowledgeGraph};

/// Controls whether isolated nodes without edges are retained by [`GraphFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum OrphanVisibility {
    /// Keep isolated nodes.
    #[default]
    Include,
    /// Hide nodes that have no visible edges.
    Exclude,
}

/// Filter criteria for extracting an interactive subgraph view (plan F.14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphFilter {
    /// Restrict to these node kinds; empty allows all kinds.
    pub kinds: BTreeSet<GraphNodeKind>,
    /// Restrict to these cluster names; empty allows all clusters.
    pub clusters: BTreeSet<String>,
    /// Require at least one matching tag; empty skips tag filtering.
    pub tags: BTreeSet<String>,
    /// Optional case-insensitive substring matched against node label or id.
    pub search_query: Option<String>,
    /// Whether isolated nodes without visible edges remain in the result.
    pub orphan_visibility: OrphanVisibility,
}

/// Filtered projection of a [`KnowledgeGraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphView {
    /// Matching nodes in deterministic id order.
    pub nodes: Vec<GraphNode>,
    /// Edges whose endpoints are both in `nodes`.
    pub edges: Vec<GraphEdge>,
    /// Node count per cluster in this filtered view.
    pub cluster_counts: BTreeMap<String, usize>,
}

/// Filters `graph` according to `filter` and returns the matching subgraph and cluster breakdown.
#[must_use]
pub fn filter_graph(graph: &KnowledgeGraph, filter: &GraphFilter) -> GraphView {
    let needle = filter
        .search_query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);

    let mut matched_ids = BTreeSet::new();
    for node in graph.nodes() {
        if !filter.kinds.is_empty() && !filter.kinds.contains(&node.kind) {
            continue;
        }
        if !filter.clusters.is_empty() && !filter.clusters.contains(&node.cluster) {
            continue;
        }
        if !filter.tags.is_empty() && !node.tags.iter().any(|t| filter.tags.contains(t)) {
            continue;
        }
        if let Some(q) = &needle
            && !node.label.to_ascii_lowercase().contains(q)
            && !node.id.to_ascii_lowercase().contains(q)
        {
            continue;
        }
        matched_ids.insert(node.id.clone());
    }

    let mut edges: Vec<GraphEdge> = graph
        .edges()
        .filter(|e| matched_ids.contains(&e.source) && matched_ids.contains(&e.target))
        .cloned()
        .collect();

    if matches!(filter.orphan_visibility, OrphanVisibility::Exclude) {
        let mut degree_ids = BTreeSet::new();
        for edge in &edges {
            degree_ids.insert(edge.source.as_str());
            degree_ids.insert(edge.target.as_str());
        }
        matched_ids.retain(|id| degree_ids.contains(id.as_str()));
        edges.retain(|e| matched_ids.contains(&e.source) && matched_ids.contains(&e.target));
    }

    let mut nodes = Vec::with_capacity(matched_ids.len());
    let mut cluster_counts = BTreeMap::new();
    for id in &matched_ids {
        if let Some(node) = graph.node(id) {
            *cluster_counts.entry(node.cluster.clone()).or_insert(0) += 1;
            nodes.push(node.clone());
        }
    }

    GraphView {
        nodes,
        edges,
        cluster_counts,
    }
}

/// Returns all node ids reachable within `max_hops` undirected steps from `center_id`.
#[must_use]
pub fn neighborhood(graph: &KnowledgeGraph, center_id: &str, max_hops: usize) -> Vec<String> {
    if graph.node(center_id).is_none() {
        return Vec::new();
    }
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(center_id.to_owned());
    queue.push_back((center_id, 0_usize));

    while let Some((curr, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        for neighbor in graph.neighbors(curr) {
            if visited.insert(neighbor.to_owned()) {
                queue.push_back((neighbor, depth + 1));
            }
        }
    }
    visited.into_iter().collect()
}

/// Computes deterministic undirected connected components across `graph`.
#[must_use]
pub fn connected_components(graph: &KnowledgeGraph) -> Vec<Vec<String>> {
    let mut visited = BTreeSet::new();
    let mut components = Vec::new();

    for node in graph.nodes() {
        if !visited.insert(node.id.clone()) {
            continue;
        }
        let mut comp = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(node.id.as_str());

        while let Some(curr) = queue.pop_front() {
            comp.push(curr.to_owned());
            for neighbor in graph.neighbors(curr) {
                if visited.insert(neighbor.to_owned()) {
                    queue.push_back(neighbor);
                }
            }
        }
        comp.sort();
        components.push(comp);
    }
    components
}
