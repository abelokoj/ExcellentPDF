//! Cross-document evidence chain DAG validation and citation path queries (plan X.3).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;

use crate::notes::export::DeepLink;

use super::{ArgumentRole, BoardError, CanvasBoard, ConnectorKind};

/// One node step in an evidence chain from a claim toward supporting sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceStep {
    /// Card identifier on the board.
    pub card_id: String,
    /// Card title or summary label.
    pub title: String,
    /// Argument role of this card.
    pub role: ArgumentRole,
    /// Optional source citation (document path + 1-based page + optional deep link).
    pub source_citation: Option<SourceCitation>,
}

/// Concrete source document citation attached to an evidence card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCitation {
    /// Canonical document path.
    pub document_path: String,
    /// One-based page number for display and deep linking.
    pub page_one_based: usize,
    /// Excerpt or quote snippet.
    pub quote: String,
    /// Deep link URI (`excellentpdf://open?...`).
    pub deep_link: String,
}

/// Complete ordered chain from a claim card down to an evidence leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceChain {
    /// Root claim card identifier.
    pub claim_id: String,
    /// Ordered steps from claim (`index 0`) to leaf evidence/source.
    pub steps: Vec<EvidenceStep>,
    /// Distinct sources cited along this chain.
    pub sources: Vec<SourceCitation>,
}

/// Checks whether the directed evidence graph formed by `board` connectors is acyclic (a valid DAG).
///
/// Returns cards in topological order (claims/dependents before their supporting sources) when valid,
/// or [`BoardError::CycleDetected`] listing the cycle participants.
///
/// # Errors
///
/// Returns [`BoardError::CycleDetected`] if any directed cycle exists among evidence/support links.
pub fn validate_evidence_dag(board: &CanvasBoard) -> Result<Vec<String>, BoardError> {
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut in_degree: BTreeMap<&str, usize> = BTreeMap::new();

    for id in board.card_ids() {
        adj.entry(id).or_default();
        in_degree.entry(id).or_insert(0);
    }

    for conn in board.connectors() {
        if !is_dag_edge(conn.kind) {
            continue;
        }
        let Some((dependent, support)) = directed_edge(conn.kind, &conn.from_card, &conn.to_card)
        else {
            continue;
        };
        if dependent == support {
            return Err(BoardError::CycleDetected(vec![dependent.to_owned()]));
        }
        if adj.entry(dependent).or_default().insert(support) {
            *in_degree.entry(support).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter_map(|(id, deg)| (*deg == 0).then_some(*id))
        .collect();
    let mut order = Vec::with_capacity(in_degree.len());

    while let Some(node) = queue.pop_front() {
        order.push(node.to_owned());
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                if let Some(deg) = in_degree.get_mut(next) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    if order.len() != in_degree.len() {
        let cycle_nodes: Vec<String> = in_degree
            .into_iter()
            .filter(|&(_, deg)| deg > 0)
            .map(|(id, _)| id.to_owned())
            .collect();
        return Err(BoardError::CycleDetected(cycle_nodes));
    }
    Ok(order)
}

/// Queries all evidence chains rooted at `claim_card_id` across documents.
///
/// # Errors
///
/// Returns [`BoardError::CardNotFound`] if `claim_card_id` does not exist, or
/// [`BoardError::CycleDetected`] if the evidence graph contains a cycle.
pub fn query_evidence_chains(
    board: &CanvasBoard,
    claim_card_id: &str,
) -> Result<Vec<EvidenceChain>, BoardError> {
    if board.card(claim_card_id).is_none() {
        return Err(BoardError::CardNotFound(claim_card_id.to_owned()));
    }
    let _ = validate_evidence_dag(board)?;

    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for conn in board.connectors() {
        if let Some((dependent, support)) = directed_edge(conn.kind, &conn.from_card, &conn.to_card)
        {
            adj.entry(dependent).or_default().insert(support);
        }
    }

    let mut chains = Vec::new();
    let mut stack = vec![claim_card_id];
    collect_chains(board, &adj, claim_card_id, &mut stack, &mut chains);
    Ok(chains)
}

/// Exports every claim's evidence chains across all documents in Markdown with all sources cited (plan X.3).
///
/// # Errors
///
/// Returns [`BoardError::CycleDetected`] if the board contains an evidence cycle.
pub fn export_evidence_chains_markdown(board: &CanvasBoard) -> Result<String, BoardError> {
    let topo = validate_evidence_dag(board)?;
    let mut claims: Vec<&str> = topo
        .iter()
        .map(String::as_str)
        .filter(|id| {
            board
                .card(id)
                .is_some_and(|c| matches!(c.role, ArgumentRole::Claim))
        })
        .collect();
    if claims.is_empty() {
        claims = topo.iter().map(String::as_str).collect();
    }

    let mut out = String::new();
    let _ = writeln!(out, "# Evidence Chains — {}\n", board.title());
    let mut all_sources = BTreeMap::<String, SourceCitation>::new();

    for claim_id in claims {
        let chains = query_evidence_chains(board, claim_id)?;
        let Some(claim) = board.card(claim_id) else {
            continue;
        };
        let _ = writeln!(out, "## Claim: {} (`{claim_id}`)\n", claim.title);
        for (idx, chain) in chains.iter().enumerate() {
            let _ = writeln!(out, "### Chain {}", idx + 1);
            for (step_idx, step) in chain.steps.iter().enumerate() {
                let indent = "  ".repeat(step_idx);
                let _ = write!(
                    out,
                    "{indent}- **{:?}**: {} (`{}`)",
                    step.role, step.title, step.card_id
                );
                if let Some(src) = &step.source_citation {
                    let _ = write!(
                        out,
                        " — [{} p.{}]({})",
                        src.document_path, src.page_one_based, src.deep_link
                    );
                    all_sources.insert(
                        format!("{}:{}", src.document_path, src.page_one_based),
                        src.clone(),
                    );
                }
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if !all_sources.is_empty() {
        out.push_str("## Cited Sources\n\n");
        for src in all_sources.values() {
            let _ = writeln!(
                out,
                "- [{} (p. {})]({}): \"{}\"",
                src.document_path,
                src.page_one_based,
                src.deep_link,
                src.quote.replace('\n', " ")
            );
        }
    }
    Ok(out)
}

fn is_dag_edge(kind: ConnectorKind) -> bool {
    matches!(
        kind,
        ConnectorKind::Supports | ConnectorKind::DerivesFrom | ConnectorKind::Cites
    )
}

fn directed_edge<'a>(
    kind: ConnectorKind,
    from: &'a str,
    to: &'a str,
) -> Option<(&'a str, &'a str)> {
    match kind {
        // Support card -> Claim card means Claim depends on Support.
        ConnectorKind::Supports => Some((to, from)),
        // Claim/Note -> Source/Evidence means From depends on To.
        ConnectorKind::DerivesFrom | ConnectorKind::Cites => Some((from, to)),
        ConnectorKind::Relates | ConnectorKind::Objects => None,
    }
}

fn collect_chains<'a>(
    board: &'a CanvasBoard,
    adj: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    claim_id: &str,
    path: &mut Vec<&'a str>,
    out: &mut Vec<EvidenceChain>,
) {
    let Some(&current) = path.last() else {
        return;
    };
    let nexts = adj.get(current).map(BTreeSet::len).unwrap_or(0);
    if nexts == 0 {
        let mut steps = Vec::with_capacity(path.len());
        let mut sources = Vec::new();
        for &id in path.iter() {
            if let Some(card) = board.card(id) {
                let source_citation = card.anchor().map(|anchor| {
                    let page_one_based = anchor.page + 1;
                    let deep = DeepLink {
                        file: anchor.document_path.clone(),
                        page: Some(page_one_based),
                        annot: None,
                    };
                    let quote_text = anchor
                        .quote
                        .as_ref()
                        .map(|q| q.exact.clone())
                        .unwrap_or_else(|| card.summary_text());
                    let citation = SourceCitation {
                        document_path: anchor.document_path.to_string_lossy().into_owned(),
                        page_one_based,
                        quote: quote_text,
                        deep_link: deep.to_uri(),
                    };
                    sources.push(citation.clone());
                    citation
                });
                steps.push(EvidenceStep {
                    card_id: card.id.clone(),
                    title: card.title.clone(),
                    role: card.role,
                    source_citation,
                });
            }
        }
        out.push(EvidenceChain {
            claim_id: claim_id.to_owned(),
            steps,
            sources,
        });
        return;
    }

    if let Some(children) = adj.get(current) {
        for &child in children {
            if !path.contains(&child) {
                path.push(child);
                collect_chains(board, adj, claim_id, path, out);
                path.pop();
            }
        }
    }
}
