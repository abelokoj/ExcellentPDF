//! Cited AI chat panel model and canvas jump-to-citation resolution (plan I.3).

use ecclesia_ai::provider::{GuardedProvider, Locality, ProviderBadge};
use ecclesia_ai::rag::{Citation, RagAnswer, RagEngine, RagError, RagScope};
use ecclesia_mupdf::{PagePoint, Quad};

/// Target on the PDF canvas scrolled to and pulsed when clicking a citation marker `[1]` (plan I.3).
#[derive(Debug, Clone, PartialEq)]
pub struct CitationJumpTarget {
    /// Source document identifier.
    pub doc_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Exact PDF quads to pulse-highlight on the canvas.
    pub quads: Vec<Quad>,
    /// Verbatim quote anchored by the citation.
    pub quote: String,
}

/// One turn in the AI chat panel conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTurn {
    /// User's prompt or action label.
    pub prompt: String,
    /// Scope used to answer the turn.
    pub scope: RagScope,
    /// Grounded answer with verified citations.
    pub answer: RagAnswer,
    /// Unix timestamp in seconds.
    pub created_at: i64,
}

/// State model for the AI chat sidebar panel (plan I.3).
#[derive(Debug, Clone)]
pub struct ChatPanelModel {
    turns: Vec<ChatTurn>,
    active_scope: RagScope,
    badge: Option<ProviderBadge>,
    engine: RagEngine,
}

impl ChatPanelModel {
    /// Creates an empty chat panel scoped to `initial_scope`.
    #[must_use]
    pub fn new(initial_scope: RagScope) -> Self {
        Self {
            turns: Vec::new(),
            active_scope: initial_scope,
            badge: None,
            engine: RagEngine::new(6),
        }
    }

    /// Updates the retrieval scope (selection, page, document, workspace, or library).
    pub fn set_scope(&mut self, scope: RagScope) {
        self.active_scope = scope;
    }

    /// Currently selected retrieval scope.
    #[must_use]
    pub fn scope(&self) -> &RagScope {
        &self.active_scope
    }

    /// Recorded conversation turns, oldest first.
    #[must_use]
    pub fn turns(&self) -> &[ChatTurn] {
        &self.turns
    }

    /// Latest visible provider locality badge (Local vs Cloud), if a turn has run.
    #[must_use]
    pub fn badge(&self) -> Option<&ProviderBadge> {
        self.badge.as_ref()
    }

    /// Formats the status badge text (`"Local: ..."` or `"Cloud: ..."`).
    #[must_use]
    pub fn locality_indicator(&self) -> String {
        match &self.badge {
            Some(badge) => {
                let kind = match badge.locality {
                    Locality::Local => "Local",
                    Locality::Cloud => "Cloud",
                    _ => "AI",
                };
                format!("{kind}: {} ({})", badge.display_name, badge.model)
            }
            None => "AI Off".to_owned(),
        }
    }

    /// Submits `question` through `provider` over `conn` and appends the verified [`ChatTurn`].
    ///
    /// # Errors
    ///
    /// Returns [`RagError`] when consent is denied, the question is empty, or retrieval/provider fails.
    pub fn ask(
        &mut self,
        conn: &rusqlite::Connection,
        provider: &mut GuardedProvider<'_>,
        question: &str,
        now: i64,
    ) -> Result<&ChatTurn, RagError> {
        let answer = self
            .engine
            .ask(conn, provider, &self.active_scope, question)?;
        self.badge = Some(answer.badge.clone());
        self.turns.push(ChatTurn {
            prompt: question.trim().to_owned(),
            scope: self.active_scope.clone(),
            answer,
            created_at: now,
        });
        Ok(self
            .turns
            .last()
            .expect("turns is non-empty immediately after push"))
    }

    /// Resolves a clicked citation marker (such as `"[1]"`) in `turn_index` to canvas quads.
    #[must_use]
    pub fn resolve_citation(&self, turn_index: usize, marker: &str) -> Option<CitationJumpTarget> {
        let turn = self.turns.get(turn_index)?;
        let cit = turn.answer.citations.iter().find(|c| c.marker == marker)?;
        Some(citation_to_jump_target(cit))
    }

    /// Clears conversation history.
    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

/// Converts an `ecclesia-ai` [`Citation`] into an `ecclesia-mupdf` [`CitationJumpTarget`].
#[must_use]
pub fn citation_to_jump_target(citation: &Citation) -> CitationJumpTarget {
    let quads = citation
        .quads
        .iter()
        .map(|q| Quad {
            ul: PagePoint {
                x: q.ul[0],
                y: q.ul[1],
            },
            ur: PagePoint {
                x: q.ur[0],
                y: q.ur[1],
            },
            ll: PagePoint {
                x: q.ll[0],
                y: q.ll[1],
            },
            lr: PagePoint {
                x: q.lr[0],
                y: q.lr[1],
            },
        })
        .collect();
    CitationJumpTarget {
        doc_id: citation.doc_id.clone(),
        page: citation.page,
        quads,
        quote: citation.quote.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_ai::index::{ChunkConfig, DocumentIndex, PageSource};
    use ecclesia_ai::provider::{ConsentPolicy, Locality, TestProvider, TokenUsage, UsageBudget};
    use rusqlite::Connection;
    use serde_json::json;

    #[test]
    fn chat_panel_resolves_clicked_citation_to_mupdf_quads() {
        let conn = Connection::open_in_memory().expect("sqlite opens");
        let index = DocumentIndex::new();
        let pages = vec![PageSource::from_plain_text(
            1,
            "Incremental save appends new revisions without rewriting earlier bytes.",
            Some("Persistence"),
        )];
        index
            .index_document(&conn, "spec", &pages, &ChunkConfig::default(), None, 0)
            .expect("indexed");

        let provider = TestProvider::new("ollama", Locality::Local);
        provider.push_response(
            json!({
                "insufficient_evidence": false,
                "claims": [{
                    "text": "Incremental save appends revisions.",
                    "chunk_id": "spec#p2c0",
                    "quote": "Incremental save appends new revisions"
                }]
            })
            .to_string(),
            TokenUsage::default(),
        );

        let mut guarded = GuardedProvider::new(
            &provider,
            ConsentPolicy::local_only(),
            UsageBudget::default(),
        );
        let mut panel = ChatPanelModel::new(RagScope::Document {
            doc_id: "spec".to_owned(),
            page_range: None,
        });
        panel
            .ask(&conn, &mut guarded, "How does incremental save work?", 100)
            .expect("ask succeeds");

        let jump = panel
            .resolve_citation(0, "[1]")
            .expect("citation [1] resolves");
        assert_eq!(jump.page, 1);
        assert!(!jump.quads.is_empty());
        assert!(panel.locality_indicator().starts_with("Local:"));
    }
}
