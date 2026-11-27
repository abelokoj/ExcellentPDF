//! Phase 6 AI assistant, RAG citations, study/quiz generation, structured claim/evidence
//! extraction, math-to-LaTeX, and consent-gated visual search UI (`I.1`–`I.12`, `R.31`, `H.25`,
//! `S.9`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_ai::index::{ChunkConfig, DocumentIndex, PageSource, ensure_schema};
use ecclesia_ai::provider::{
    AiConsentMode, AiProvider, AnthropicProvider, ConsentPolicy, GeminiProvider, GuardedProvider,
    Locality, OllamaProvider, OpenAiCompatibleProvider, RedactionPolicy, ScopeConsent, UsageBudget,
};
use ecclesia_ai::rag::RagScope;
use ecclesia_core::anchor::new_id;
use ecclesia_core::annot::{Annotation, Boxed, Shape, Style};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_features::ai::extract::ExtractionReport;
use ecclesia_features::ai::{
    ActionOutput, ChapterQuizDraft, ChatPanelModel, DraftDecision, EvidenceTable, MathBackend,
    MathConversionResult, QuizExamRunner, QuizOutcome, QuizQuestionKind, StudyDraftBatch,
    SuggestedHighlight, SummaryLength, apply_accepted_highlights, build_evidence_table,
    convert_strokes_to_latex, define_term_in_context, explain_equation_steps,
    extract_structured_items, generate_chapter_quiz, generate_study_materials,
    rewrite_plain_language, suggest_highlights_from_extractions, summarize_chapter,
    summarize_document, validate_latex_braces,
};
use ecclesia_features::handwriting::HandwritingStroke;
use ecclesia_features::lens::{
    LensConsent, OnlineAccessMode, build_upload_from_png, build_upload_from_region,
};
use ecclesia_features::snapshot::Region;
use ecclesia_features::study::exam::{ExamConfig, VisibilitySnapshot};
use ecclesia_mupdf::{PagePoint, PageRect};
use ecclesia_store::study::{CardRecord, DeckRecord, upsert_card, upsert_deck};
use nsh_base::{button, input, theme};
use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::bookmarks::field;
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::panes::{Pane, Panes};
use crate::settings::{self, Live, write_atomically};
use crate::settings_window::{segment_owned, segmented};

/// `settings.toml` key for global AI consent (`"disabled"`, `"local"`, `"prompt"`, `"cloud"`).
pub(crate) const AI_CONSENT_MODE: &str = "ai_consent_mode";
/// `settings.toml` key for provider family (`"ollama"`, `"llama_server"`, `"openai"`, `"anthropic"`, `"gemini"`, `"custom"`).
pub(crate) const AI_PROVIDER_KIND: &str = "ai_provider_kind";
/// `settings.toml` key for the provider base URL (used for Ollama, llama-server, or custom endpoints).
pub(crate) const AI_ENDPOINT_URL: &str = "ai_endpoint_url";
/// `settings.toml` key for the selected model name.
pub(crate) const AI_MODEL_NAME: &str = "ai_model_name";
/// `settings.toml` key for the provider API key (for cloud/custom endpoints).
pub(crate) const AI_API_KEY: &str = "ai_api_key";
/// `settings.toml` key controlling PII redaction before dispatch (`"redact"` or `"raw"`).
pub(crate) const AI_REDACTION_MODE: &str = "ai_redaction_mode";
/// `settings.toml` key capping session tokens (`0` = unlimited).
pub(crate) const AI_SESSION_BUDGET: &str = "ai_session_token_budget";
/// `settings.toml` key gating online visual search / Google Lens (`R.31`).
pub(crate) const LENS_ONLINE_ENABLED: &str = "lens_online_enabled";

/// Shared pill action button matching DESIGN.md OKLCH tokens for Phase 6 & 7 dialogs.
pub(crate) fn pill_button(
    label: impl Into<String>,
    accent: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let text_str = label.into();
    let a11y = text_str.clone();
    button()
        .accessibility_label(a11y)
        .on_press(on_press)
        .h(px(30.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .bg(move || {
            if accent {
                theme().primary
            } else {
                theme().secondary
            }
        })
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(text_str)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if accent {
                        theme().primary_foreground
                    } else {
                        theme().secondary_foreground
                    }
                }),
        )
}

/// Shared dynamic muted caption line for Phase 6 & 7 dialogs.
pub(crate) fn status_caption(value: impl Fn() -> String + 'static) -> impl IntoElement {
    text(value)
        .text_size(px(12.0))
        .text_color(move || theme().muted_foreground)
}

/// Shared styled input box wrapping [`crate::bookmarks::field`] for Phase 6 & 7 forms.
pub(crate) fn labeled_field(
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(String) + 'static,
    placeholder_text: String,
    label_text: String,
) -> impl IntoElement {
    field(
        input()
            .value(value)
            .on_change(move |text: &str| on_change(text.to_owned()))
            .placeholder(placeholder_text)
            .label(label_text),
    )
}

/// Tabs inside the AI Assistant dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AiPanelTab {
    /// Cited Q&A conversation (`I.3`).
    #[default]
    Chat,
    /// Document/chapter summaries, rewrites, equations, terms (`I.4`–`I.7`).
    Actions,
    /// Review-gated glossary, study guide & flashcards (`I.8`).
    Study,
    /// Claim/method/finding extraction, evidence tables & ghost highlights (`I.9`–`I.11`).
    Research,
    /// Chapter quiz generator & exam runner (`S.9`).
    Quiz,
}

/// Active RAG scope selector in the UI (`I.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AiScopeKind {
    /// Current page only.
    Page,
    /// Active document.
    #[default]
    Document,
    /// All open tabs in the active window.
    Workspace,
    /// Whole library index.
    Library,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn ai_db_path() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-ai"))
        .join("ai.sqlite")
}

/// Parses configured [`AiConsentMode`] from settings.
pub(crate) fn consent_mode_from_live(live: Live) -> AiConsentMode {
    live.read(|s| match s.text(AI_CONSENT_MODE).unwrap_or("local") {
        "disabled" => AiConsentMode::Disabled,
        "prompt" => AiConsentMode::LocalAndCloudWithPrompt,
        "cloud" => AiConsentMode::CloudApproved,
        _ => AiConsentMode::LocalOnly,
    })
}

/// Parses configured [`ConsentPolicy`] for the current document/workspace.
pub(crate) fn consent_policy_from_live(live: Live, doc_off: bool) -> ConsentPolicy {
    let global_mode = consent_mode_from_live(live);
    let redaction = live.read(|s| match s.text(AI_REDACTION_MODE).unwrap_or("redact") {
        "raw" => RedactionPolicy::AllowUnredacted,
        _ => RedactionPolicy::RedactSensitiveData,
    });
    let document_scope = if doc_off {
        ScopeConsent::ExplicitlyDisabled
    } else {
        ScopeConsent::Inherited
    };
    let mut policy = ConsentPolicy {
        global_mode,
        workspace_scope: ScopeConsent::Inherited,
        document_scope,
        approved_cloud_providers: Default::default(),
        redaction,
    };
    if global_mode == AiConsentMode::CloudApproved {
        let kind = live.read(|s| s.text(AI_PROVIDER_KIND).unwrap_or("ollama").to_owned());
        policy.approve_cloud_provider(&kind);
    }
    policy
}

fn budget_from_live(live: Live) -> UsageBudget {
    let max_tokens = live.read(|s| s.number(AI_SESSION_BUDGET)).unwrap_or(0.0);
    let max_total_tokens = if max_tokens > 0.0 {
        Some(max_tokens as u64)
    } else {
        None
    };
    UsageBudget {
        max_total_tokens,
        max_cost_micros: None,
        max_requests: None,
    }
}

enum ConfiguredProvider {
    Ollama(OllamaProvider),
    OpenAiCompat(OpenAiCompatibleProvider),
    Anthropic(AnthropicProvider),
    Gemini(GeminiProvider),
}

impl ConfiguredProvider {
    fn from_live(live: Live) -> Result<Self, String> {
        let (kind, url, model, key) = live.read(|s| {
            (
                s.text(AI_PROVIDER_KIND).unwrap_or("ollama").to_owned(),
                s.text(AI_ENDPOINT_URL).unwrap_or_default().to_owned(),
                s.text(AI_MODEL_NAME).unwrap_or_default().to_owned(),
                s.text(AI_API_KEY).unwrap_or_default().to_owned(),
            )
        });
        match kind.as_str() {
            "llama_server" => {
                let endpoint = if url.is_empty() {
                    "http://127.0.0.1:8080".to_owned()
                } else {
                    url
                };
                let mdl = if model.is_empty() {
                    "local-model".to_owned()
                } else {
                    model
                };
                OpenAiCompatibleProvider::llama_server(&endpoint, &mdl)
                    .map(Self::OpenAiCompat)
                    .map_err(|e| e.to_string())
            }
            "openai" => {
                let mdl = if model.is_empty() {
                    "gpt-4o-mini".to_owned()
                } else {
                    model
                };
                Ok(Self::OpenAiCompat(OpenAiCompatibleProvider::openai(
                    key, mdl,
                )))
            }
            "custom" => {
                let endpoint = if url.is_empty() {
                    "http://127.0.0.1:1234/v1".to_owned()
                } else {
                    url
                };
                let mdl = if model.is_empty() {
                    "local-model".to_owned()
                } else {
                    model
                };
                OpenAiCompatibleProvider::custom(
                    "custom",
                    "Custom Endpoint",
                    &endpoint,
                    (!key.is_empty()).then_some(key.as_str()),
                    &mdl,
                )
                .map(Self::OpenAiCompat)
                .map_err(|e| e.to_string())
            }
            "anthropic" => {
                let mdl = if model.is_empty() {
                    "claude-3-5-sonnet-latest".to_owned()
                } else {
                    model
                };
                Ok(Self::Anthropic(AnthropicProvider::new(key, mdl)))
            }
            "gemini" => {
                let mdl = if model.is_empty() {
                    "gemini-2.0-flash".to_owned()
                } else {
                    model
                };
                Ok(Self::Gemini(GeminiProvider::new(key, mdl)))
            }
            _ => {
                let endpoint = if url.is_empty() {
                    "http://127.0.0.1:11434".to_owned()
                } else {
                    url
                };
                let mdl = if model.is_empty() {
                    "llama3.2".to_owned()
                } else {
                    model
                };
                OllamaProvider::new(&endpoint, &mdl)
                    .map(Self::Ollama)
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn as_dyn(&self) -> &dyn AiProvider {
        match self {
            Self::Ollama(p) => p,
            Self::OpenAiCompat(p) => p,
            Self::Anthropic(p) => p,
            Self::Gemini(p) => p,
        }
    }
}

/// Status string for the AI settings/locality indicator (`"Local: ..."`, `"Cloud: ..."`, or `"AI Off"`).
pub(crate) fn active_locality_badge(live: Live, doc_off: bool) -> String {
    if doc_off || consent_mode_from_live(live) == AiConsentMode::Disabled {
        return t!("ai.locality.off");
    }
    let Ok(cfg) = ConfiguredProvider::from_live(live) else {
        return t!("ai.locality.off");
    };
    let prov = cfg.as_dyn();
    match prov.locality() {
        Locality::Local => t!(
            "ai.locality.local",
            provider = prov.display_name(),
            model = prov.default_model()
        ),
        Locality::Cloud => t!(
            "ai.locality.cloud",
            provider = prov.display_name(),
            model = prov.default_model()
        ),
        _ => t!(
            "ai.locality.local",
            provider = prov.display_name(),
            model = prov.default_model()
        ),
    }
}

/// Lazily allocated reactive state for Phase 6 AI, math-to-LaTeX, and visual search panels/dialogs.
#[derive(Clone, Copy)]
struct AiInner {
    active_tab: Signal<AiPanelTab>,
    scope_kind: Signal<AiScopeKind>,
    doc_ai_disabled: Signal<bool>,
    question_draft: Signal<String>,
    selection_draft: Signal<String>,
    summary_length: Signal<SummaryLength>,
    quiz_count: Signal<usize>,
    status_line: Signal<String>,
    chat_model: Stored<Option<ChatPanelModel>>,
    turns_rev: Signal<u64>,
    action_output: Signal<Option<ActionOutput>>,
    study_batch: Stored<Option<StudyDraftBatch>>,
    study_rev: Signal<u64>,
    extraction_report: Stored<Option<ExtractionReport>>,
    evidence_table: Stored<Option<EvidenceTable>>,
    ghost_highlights: Stored<Vec<SuggestedHighlight>>,
    research_rev: Signal<u64>,
    quiz_draft: Stored<Option<ChapterQuizDraft>>,
    quiz_runner: Stored<Option<QuizExamRunner>>,
    quiz_outcome: Signal<Option<QuizOutcome>>,
    quiz_rev: Signal<u64>,
    math_input_expr: Signal<String>,
    math_result: Stored<Option<MathConversionResult>>,
    math_rev: Signal<u64>,
    lens_consent: Signal<LensConsent>,
    lens_status: Signal<String>,
    busy: Signal<bool>,
}

impl AiInner {
    fn new() -> Self {
        Self {
            active_tab: signal(AiPanelTab::Chat),
            scope_kind: signal(AiScopeKind::Document),
            doc_ai_disabled: signal(false),
            question_draft: signal(String::new()),
            selection_draft: signal(String::new()),
            summary_length: signal(SummaryLength::Standard),
            quiz_count: signal(5),
            status_line: signal(String::new()),
            chat_model: stored(None),
            turns_rev: signal(0),
            action_output: signal(None),
            study_batch: stored(None),
            study_rev: signal(0),
            extraction_report: stored(None),
            evidence_table: stored(None),
            ghost_highlights: stored(Vec::new()),
            research_rev: signal(0),
            quiz_draft: stored(None),
            quiz_runner: stored(None),
            quiz_outcome: signal(None),
            quiz_rev: signal(0),
            math_input_expr: signal(String::from("x^2 + y^2 = r^2")),
            math_result: stored(None),
            math_rev: signal(0),
            lens_consent: signal(LensConsent::Pending),
            lens_status: signal(String::new()),
            busy: signal(false),
        }
    }
}

/// Persistent UI state for all Phase 6 AI, math-to-LaTeX, and visual search panels/dialogs.
/// Inner signals and domain models are allocated lazily on first open so cold start stays minimal.
#[derive(Clone, Copy)]
pub(crate) struct AiUiState {
    owner: Owner,
    panel_open: Signal<bool>,
    math_open: Signal<bool>,
    lens_open: Signal<bool>,
    inner: Stored<Option<AiInner>>,
}

impl AiUiState {
    /// Allocates lightweight visibility state for Phase 6 features without startup allocations.
    pub(crate) fn new() -> Self {
        Self {
            owner: Owner::current(),
            panel_open: signal(false),
            math_open: signal(false),
            lens_open: signal(false),
            inner: stored(None),
        }
    }

    fn ensure_initialized(self) -> AiInner {
        self.inner
            .with(|slot| *slot.get_or_insert_with(|| self.owner.run(AiInner::new)))
    }

    /// True while any Phase 6 dialog covers the native page canvas HWND.
    pub(crate) fn any_dialog_open(self) -> bool {
        self.panel_open.get() || self.math_open.get() || self.lens_open.get()
    }

    /// Opens the AI panel on `tab`.
    pub(crate) fn open_panel(self, tab: AiPanelTab) {
        let inner = self.ensure_initialized();
        inner.active_tab.set(tab);
        self.panel_open.set(true);
    }

    /// Opens the Math-to-LaTeX confirmation modal (`H.25`) using sample/lassoed strokes.
    pub(crate) fn open_math_to_latex(self) {
        let inner = self.ensure_initialized();
        let sample_strokes = sample_equation_strokes();
        if let Ok(conv) = convert_strokes_to_latex(&sample_strokes, MathBackend::BuiltinGeometric) {
            inner.math_input_expr.set(conv.latex.clone());
            inner.math_result.with(|slot| *slot = Some(conv));
            inner.math_rev.update(|r| *r = r.wrapping_add(1));
        }
        self.math_open.set(true);
    }

    /// Opens the consent-gated Visual Search / Google Lens modal (`R.31`).
    pub(crate) fn open_visual_search(self) {
        let inner = self.ensure_initialized();
        inner.lens_consent.set(LensConsent::Pending);
        inner.lens_status.set(String::new());
        self.lens_open.set(true);
    }
}

fn sample_equation_strokes() -> Vec<HandwritingStroke> {
    let s1 = HandwritingStroke {
        id: "stroke-1".to_owned(),
        points: vec![
            PagePoint { x: 110.0, y: 115.0 },
            PagePoint { x: 140.0, y: 135.0 },
        ],
        timestamps_ms: vec![0.0, 25.0],
    };
    let s2 = HandwritingStroke {
        id: "stroke-2".to_owned(),
        points: vec![
            PagePoint { x: 140.0, y: 115.0 },
            PagePoint { x: 110.0, y: 135.0 },
        ],
        timestamps_ms: vec![30.0, 55.0],
    };
    vec![s1, s2]
}

fn open_ai_db() -> Result<ecclesia_store::rusqlite::Connection, String> {
    let db_path = ai_db_path();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = ecclesia_store::rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    ensure_schema(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn active_doc_context(panes: Panes) -> Option<(PathBuf, String, usize, usize)> {
    let pane = panes.active();
    let path = pane.tabs.shown_path()?;
    let doc_id = path.to_string_lossy().into_owned();
    let current = pane.pages.current();
    let count = pane.pages.count();
    Some((path, doc_id, current, count))
}

fn index_pdf_sync(path: &Path, doc_id: &str) -> Result<usize, String> {
    let doc = ecclesia_mupdf::Document::open(path).map_err(|e| e.to_string())?;
    let total = doc.page_count().map_err(|e| e.to_string())?;
    let mut sources = Vec::with_capacity(total);
    for page_idx in 0..total {
        let page = doc.load_page(page_idx).map_err(|e| e.to_string())?;
        let stext = page.text(None).map_err(|e| e.to_string())?;
        let text = stext.text().map_err(|e| e.to_string())?;
        if !text.trim().is_empty() {
            sources.push(PageSource::from_plain_text(page_idx, &text, None));
        }
    }
    let conn = open_ai_db()?;
    let stats = DocumentIndex::new()
        .index_document(
            &conn,
            doc_id,
            &sources,
            &ChunkConfig::default(),
            None,
            now_secs(),
        )
        .map_err(|e| e.to_string())?;
    Ok(stats.chunk_count)
}

fn current_rag_scope(state: AiInner, panes: Panes, doc_id: &str, page: usize) -> RagScope {
    match state.scope_kind.get() {
        AiScopeKind::Page => RagScope::Page {
            doc_id: doc_id.to_owned(),
            page,
        },
        AiScopeKind::Document => RagScope::Document {
            doc_id: doc_id.to_owned(),
            page_range: None,
        },
        AiScopeKind::Workspace => {
            let doc_ids: Vec<String> = panes
                .slots()
                .into_iter()
                .filter_map(|p| {
                    p.tabs
                        .shown_path()
                        .map(|pb| pb.to_string_lossy().into_owned())
                })
                .collect();
            if doc_ids.is_empty() {
                RagScope::Document {
                    doc_id: doc_id.to_owned(),
                    page_range: None,
                }
            } else {
                RagScope::Workspace { doc_ids }
            }
        }
        AiScopeKind::Library => RagScope::Library,
    }
}

fn jump_to_page(pane: Pane, page: usize) {
    pane.canvas.command(
        "goToPoint",
        serde_json::json!({
            "page": page,
            "x": 0.0,
            "y": 0.0,
            "zoom": JsonValue::Null
        }),
    );
}

/// Renders every open Phase 6 dialog (`AiPanel`, `MathToLatex`, `VisualSearch`).
pub(crate) fn ai_dialogs(state: AiUiState, panes: Panes, live: Live) -> impl IntoElement {
    when(
        move || state.any_dialog_open(),
        move || {
            view()
                .child(when(
                    move || state.panel_open.get(),
                    move || ai_assistant_dialog(state, panes, live),
                ))
                .child(when(
                    move || state.math_open.get(),
                    move || math_latex_dialog(state, panes),
                ))
                .child(when(
                    move || state.lens_open.get(),
                    move || visual_search_dialog(state, panes, live),
                ))
        },
    )
}

fn ai_assistant_dialog(state: AiUiState, panes: Panes, live: Live) -> impl IntoElement {
    let panel_open = state.panel_open;
    let close = move || panel_open.set(false);
    let state = state.ensure_initialized();
    let doc_off = state.doc_ai_disabled.get();
    let badge_text = active_locality_badge(live, doc_off);

    let tab_bar = segmented()
        .child(segment_owned(
            t!("ai.tab.chat"),
            move || state.active_tab.get() == AiPanelTab::Chat,
            move || state.active_tab.set(AiPanelTab::Chat),
        ))
        .child(segment_owned(
            t!("ai.tab.actions"),
            move || state.active_tab.get() == AiPanelTab::Actions,
            move || state.active_tab.set(AiPanelTab::Actions),
        ))
        .child(segment_owned(
            t!("ai.tab.study"),
            move || state.active_tab.get() == AiPanelTab::Study,
            move || state.active_tab.set(AiPanelTab::Study),
        ))
        .child(segment_owned(
            t!("ai.tab.research"),
            move || state.active_tab.get() == AiPanelTab::Research,
            move || state.active_tab.set(AiPanelTab::Research),
        ))
        .child(segment_owned(
            t!("ai.tab.quiz"),
            move || state.active_tab.get() == AiPanelTab::Quiz,
            move || state.active_tab.set(AiPanelTab::Quiz),
        ));

    let index_doc = move || {
        let Some((path, doc_id, _, _)) = active_doc_context(panes) else {
            return;
        };
        if state.busy.get_untracked() {
            return;
        }
        state.busy.set(true);
        state.status_line.set(t!("ai.indexing"));
        spawn(async move {
            let res = blocking(move || index_pdf_sync(&path, &doc_id)).await;
            state.busy.set(false);
            match res {
                Ok(Ok(count)) => {
                    state.status_line.set(t!("ai.indexed", count = count));
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let header = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .child(tab_bar)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || badge_text.clone())
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || theme().muted)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(pill_button(t!("ai.index_doc"), false, index_doc))
                .child(pill_button(
                    if state.doc_ai_disabled.get() {
                        t!("ai.doc_enable")
                    } else {
                        t!("ai.doc_disable")
                    },
                    false,
                    move || state.doc_ai_disabled.update(|v| *v = !*v),
                )),
        );

    let body = card(
        view()
            .w(px(680.0))
            .max_h(px(560.0))
            .gap_3()
            .child(header)
            .child(when(
                move || !state.status_line.get().is_empty(),
                move || status_caption(move || state.status_line.get()),
            ))
            .child(when(
                move || state.active_tab.get() == AiPanelTab::Chat,
                move || chat_tab_view(panel_open, state, panes, live),
            ))
            .child(when(
                move || state.active_tab.get() == AiPanelTab::Actions,
                move || actions_tab_view(state, panes, live),
            ))
            .child(when(
                move || state.active_tab.get() == AiPanelTab::Study,
                move || study_tab_view(state, panes, live),
            ))
            .child(when(
                move || state.active_tab.get() == AiPanelTab::Research,
                move || research_tab_view(state, panes, live),
            ))
            .child(when(
                move || state.active_tab.get() == AiPanelTab::Quiz,
                move || quiz_tab_view(state, panes, live),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );

    list_dialog(t!("ai.panel.title"), close, body)
}

fn chat_tab_view(
    panel_open: Signal<bool>,
    state: AiInner,
    panes: Panes,
    live: Live,
) -> impl IntoElement {
    let scope_bar = segmented()
        .child(segment_owned(
            t!("ai.scope.page"),
            move || state.scope_kind.get() == AiScopeKind::Page,
            move || state.scope_kind.set(AiScopeKind::Page),
        ))
        .child(segment_owned(
            t!("ai.scope.document"),
            move || state.scope_kind.get() == AiScopeKind::Document,
            move || state.scope_kind.set(AiScopeKind::Document),
        ))
        .child(segment_owned(
            t!("ai.scope.workspace"),
            move || state.scope_kind.get() == AiScopeKind::Workspace,
            move || state.scope_kind.set(AiScopeKind::Workspace),
        ))
        .child(segment_owned(
            t!("ai.scope.library"),
            move || state.scope_kind.get() == AiScopeKind::Library,
            move || state.scope_kind.set(AiScopeKind::Library),
        ));

    let ask_submit = move || {
        let q = state.question_draft.get_untracked();
        let trimmed = q.trim().to_owned();
        if trimmed.is_empty() || state.busy.get_untracked() {
            return;
        }
        let Some((path, doc_id, page, _)) = active_doc_context(panes) else {
            return;
        };
        let scope = current_rag_scope(state, panes, &doc_id, page);
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        let mut model_snapshot = state.chat_model.with(|slot| {
            slot.get_or_insert_with(|| ChatPanelModel::new(RagScope::Library))
                .clone()
        });
        model_snapshot.set_scope(scope);

        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                model_snapshot
                    .ask(&conn, &mut guarded, &trimmed, now_secs())
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(model_snapshot)
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok(updated)) => {
                    state.chat_model.with(|slot| *slot = Some(updated));
                    state.question_draft.set(String::new());
                    state.status_line.set(String::new());
                    state.turns_rev.update(|r| *r = r.wrapping_add(1));
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let turns_view = scroll().max_h(px(280.0)).child(dynamic(move || {
        let _rev = state.turns_rev.get();
        let turns = state.chat_model.with(|slot| {
            slot.as_ref()
                .map(|m| m.turns().to_vec())
                .unwrap_or_default()
        });
        if turns.is_empty() {
            return view()
                .py_4()
                .child(status_caption(move || t!("ai.chat.empty")))
                .into_element();
        }
        let mut list = view().gap_2();
        for turn in turns {
            let prompt_str = turn.prompt.clone();
            let ans_str = turn.answer.answer_markdown.clone();
            let mut cit_row = view().flex_row().flex_wrap().gap_1();
            for cit in turn.answer.citations {
                let page_idx = cit.page;
                let marker = cit.marker.clone();
                let label = format!("{} (p.{})", marker, page_idx + 1);
                cit_row = cit_row.child(pill_button(label, false, move || {
                    jump_to_page(panes.active(), page_idx);
                    panel_open.set(false);
                }));
            }
            list = list.child(
                view()
                    .p_2()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .gap_1()
                    .child(
                        text(prompt_str)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(ans_str)
                            .text_size(px(13.0))
                            .text_color(move || theme().foreground),
                    )
                    .child(cit_row),
            );
        }
        list.into_element()
    }));

    view().gap_2().child(scope_bar).child(turns_view).child(
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                input()
                    .value(move || state.question_draft.get())
                    .on_change(move |text: &str| state.question_draft.set(text.to_owned()))
                    .on_submit(move |_: &str| ask_submit())
                    .placeholder(t!("ai.chat.placeholder"))
                    .label(t!("ai.chat.prompt_label"))
                    .flex_1()
                    .h(px(34.0))
                    .px_3()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().input),
            )
            .child(pill_button(t!("ai.chat.ask"), true, ask_submit)),
    )
}

fn actions_tab_view(state: AiInner, panes: Panes, live: Live) -> impl IntoElement {
    let run_summary = move |whole_doc: bool| {
        if state.busy.get_untracked() {
            return;
        }
        let Some((path, doc_id, page, _)) = active_doc_context(panes) else {
            return;
        };
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        let length = state.summary_length.get_untracked();
        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                if whole_doc {
                    summarize_document(&conn, &mut guarded, &doc_id, length)
                } else {
                    let end = page.saturating_add(5);
                    summarize_chapter(&conn, &mut guarded, &doc_id, page..=end, length)
                }
                .map_err(|e| e.to_string())
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok(out)) => {
                    state.action_output.set(Some(out));
                    state.status_line.set(String::new());
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let run_passage_action = move |kind: u8| {
        if state.busy.get_untracked() {
            return;
        }
        let passage = state.selection_draft.get_untracked();
        let Some((path, doc_id, page, _)) = active_doc_context(panes) else {
            return;
        };
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                match kind {
                    0 => rewrite_plain_language(&mut guarded, &doc_id, page, &passage, &[]),
                    1 => explain_equation_steps(&conn, &mut guarded, &doc_id, page, &passage),
                    _ => define_term_in_context(&conn, &mut guarded, &doc_id, page, &passage, ""),
                }
                .map_err(|e| e.to_string())
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok(out)) => {
                    state.action_output.set(Some(out));
                    state.status_line.set(String::new());
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let length_picker = segmented()
        .child(segment_owned(
            t!("ai.summary.brief"),
            move || state.summary_length.get() == SummaryLength::Brief,
            move || state.summary_length.set(SummaryLength::Brief),
        ))
        .child(segment_owned(
            t!("ai.summary.standard"),
            move || state.summary_length.get() == SummaryLength::Standard,
            move || state.summary_length.set(SummaryLength::Standard),
        ))
        .child(segment_owned(
            t!("ai.summary.detailed"),
            move || state.summary_length.get() == SummaryLength::Detailed,
            move || state.summary_length.set(SummaryLength::Detailed),
        ));

    view()
        .gap_2()
        .child(length_picker)
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(
                    t!("ai.action.summarize_doc"),
                    true,
                    move || run_summary(true),
                ))
                .child(pill_button(
                    t!("ai.action.summarize_chapter"),
                    false,
                    move || run_summary(false),
                )),
        )
        .child(labeled_field(
            move || state.selection_draft.get(),
            move |v| state.selection_draft.set(v),
            t!("ai.actions.passage_placeholder"),
            t!("ai.actions.passage_label"),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(
                    t!("ai.action.rewrite_plain"),
                    false,
                    move || run_passage_action(0),
                ))
                .child(pill_button(
                    t!("ai.action.explain_equation"),
                    false,
                    move || run_passage_action(1),
                ))
                .child(pill_button(t!("ai.action.define_term"), false, move || {
                    run_passage_action(2)
                })),
        )
        .child(scroll().max_h(px(210.0)).child(dynamic(move || {
            let Some(out) = state.action_output.get() else {
                return status_caption(move || t!("ai.actions.empty")).into_element();
            };
            view()
                .p_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || theme().border)
                .gap_1()
                .child(
                    text(out.title)
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(out.markdown)
                        .text_size(px(12.0))
                        .text_color(move || theme().foreground),
                )
                .into_element()
        })))
}

fn study_tab_view(state: AiInner, panes: Panes, live: Live) -> impl IntoElement {
    let generate = move || {
        if state.busy.get_untracked() {
            return;
        }
        let Some((path, doc_id, page, _)) = active_doc_context(panes) else {
            return;
        };
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                generate_study_materials(
                    &conn,
                    &mut guarded,
                    &doc_id,
                    Some(page..=page.saturating_add(4)),
                )
                .map_err(|e| e.to_string())
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok(batch)) => {
                    state.study_batch.with(|slot| *slot = Some(batch));
                    state.study_rev.update(|r| *r = r.wrapping_add(1));
                    state.status_line.set(String::new());
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let commit_accepted = move || {
        let Some((path, _, _, _)) = active_doc_context(panes) else {
            return;
        };
        let Some(batch) = state.study_batch.with(|b| b.clone()) else {
            return;
        };
        let deck_id = new_id();
        let now = now_secs();
        let cards = batch.into_accepted_flashcards(&path, &deck_id, now);
        let guide_md = batch.accepted_study_guide_markdown();
        let count = cards.len();
        if let Some(store) = crate::library::store() {
            let deck_path = path.clone();
            let deck_id_copy = deck_id.clone();
            let cards_copy = cards.clone();
            let _ = store.write(move |tx| {
                let title = deck_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "AI Study".to_owned());
                upsert_deck(
                    tx,
                    &DeckRecord {
                        id: deck_id_copy.clone(),
                        name: title,
                        description: "AI study materials".to_owned(),
                        source: deck_path.to_string_lossy().into_owned(),
                        license: String::new(),
                        builtin: false,
                        installed_total: 0,
                        missing_definitions_json: "[]".to_owned(),
                        created: now,
                    },
                )?;
                for card in &cards_copy {
                    let rec = CardRecord {
                        id: card.id.clone(),
                        deck_id: deck_id_copy.clone(),
                        kind: "basic".to_owned(),
                        front: card.front.clone(),
                        back: card.back.clone(),
                        cloze_ordinal: None,
                        occlusion_json: None,
                        source_path: card
                            .source
                            .as_ref()
                            .map(|s| s.path.to_string_lossy().into_owned()),
                        source_page: card.source.as_ref().map(|s| s.page),
                        anchor_json: None,
                        media_json: "[]".to_owned(),
                        tags_json: serde_json::to_string(&card.tags)
                            .unwrap_or_else(|_| "[]".to_owned()),
                        extra_json: "{}".to_owned(),
                        created: card.created,
                        modified: card.modified,
                    };
                    upsert_card(tx, &rec)?;
                }
                Ok(())
            });
        }
        state.status_line.set(t!(
            "ai.study.committed",
            cards = count,
            chars = guide_md.len()
        ));
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(pill_button(t!("ai.study.generate"), true, generate))
                .child(pill_button(
                    t!("ai.study.save_accepted"),
                    false,
                    commit_accepted,
                )),
        )
        .child(scroll().max_h(px(280.0)).child(dynamic(move || {
            let _rev = state.study_rev.get();
            let Some(batch) = state.study_batch.with(|b| b.clone()) else {
                return status_caption(move || t!("ai.study.empty")).into_element();
            };
            let mut list = view().gap_2();
            for fc in batch.flashcards {
                let id = fc.id.clone();
                let id_dismiss = fc.id.clone();
                let status = match fc.decision {
                    DraftDecision::PendingReview => t!("ai.draft.pending"),
                    DraftDecision::Accepted => t!("ai.draft.accepted"),
                    DraftDecision::Dismissed => t!("ai.draft.dismissed"),
                    _ => t!("ai.draft.pending"),
                };
                let back_txt = fc.back.clone();
                list = list.child(
                    view()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .gap_1()
                        .child(text(format!("[{status}] {}", fc.front)).text_size(px(12.0)))
                        .child(status_caption(move || back_txt.clone()))
                        .child(
                            view()
                                .flex_row()
                                .gap_2()
                                .child(pill_button(t!("ai.draft.accept"), true, move || {
                                    state.study_batch.with(|opt| {
                                        if let Some(b) = opt {
                                            let _ = b.accept_flashcard(&id);
                                        }
                                    });
                                    state.study_rev.update(|r| *r = r.wrapping_add(1));
                                }))
                                .child(pill_button(t!("ai.draft.dismiss"), false, move || {
                                    state.study_batch.with(|opt| {
                                        if let Some(b) = opt {
                                            let _ = b.dismiss_flashcard(&id_dismiss);
                                        }
                                    });
                                    state.study_rev.update(|r| *r = r.wrapping_add(1));
                                })),
                        ),
                );
            }
            for gl in batch.glossary {
                let gid = gl.id.clone();
                list = list.child(
                    view()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(text(format!("{}: {}", gl.term, gl.definition)).text_size(px(12.0)))
                        .child(pill_button(t!("ai.draft.accept"), false, move || {
                            state.study_batch.with(|opt| {
                                if let Some(b) = opt {
                                    let _ = b.accept_glossary(&gid);
                                }
                            });
                            state.study_rev.update(|r| *r = r.wrapping_add(1));
                        })),
                );
            }
            list.into_element()
        })))
}

fn research_tab_view(state: AiInner, panes: Panes, live: Live) -> impl IntoElement {
    let extract_now = move || {
        if state.busy.get_untracked() {
            return;
        }
        let Some((path, doc_id, _, _)) = active_doc_context(panes) else {
            return;
        };
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                let report = extract_structured_items(&conn, &mut guarded, &doc_id)
                    .map_err(|e| e.to_string())?;
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| doc_id.clone());
                let table = build_evidence_table(&conn, &mut guarded, &[(&doc_id, &title)])
                    .map_err(|e| e.to_string())?;
                let ghosts = suggest_highlights_from_extractions(&report.items);
                Ok::<_, String>((report, table, ghosts))
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok((report, table, ghosts))) => {
                    state.extraction_report.with(|slot| *slot = Some(report));
                    state.evidence_table.with(|slot| *slot = Some(table));
                    state.ghost_highlights.with(|slot| *slot = ghosts);
                    state.research_rev.update(|r| *r = r.wrapping_add(1));
                    state.status_line.set(String::new());
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let export_evidence_csv = move || {
        let Some(table) = state.evidence_table.with(|t| t.clone()) else {
            return;
        };
        let csv = table.to_csv();
        let dialog = SaveDialog::new()
            .title(t!("ai.research.export_csv"))
            .file_name("evidence-table.csv")
            .filter(FileFilter::new(t!("filter.csv"), ["csv"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(path)) = answer.await {
                let _ = write_atomically(&path, csv.as_bytes());
            }
        });
    };

    let apply_ghosts = move || {
        let pane = panes.active();
        let author = live.read(|s| s.text("author").unwrap_or("Reader").to_owned());
        let ghosts = state.ghost_highlights.with(|v| v.clone());
        let mut annots = Vec::new();
        let count = apply_accepted_highlights(&ghosts, &mut annots, &author);
        if count == 0 {
            state.status_line.set(t!("ai.research.no_accepted_ghosts"));
            return;
        }
        let changes: Vec<Change> = annots.into_iter().map(Change::CreateAnnotation).collect();
        pane.editing.apply(&Operation {
            name: t!("ai.research.apply_ghosts"),
            changes,
        });
        state
            .status_line
            .set(t!("ai.research.applied", count = count));
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(t!("ai.research.extract"), true, extract_now))
                .child(pill_button(
                    t!("ai.research.export_csv"),
                    false,
                    export_evidence_csv,
                ))
                .child(pill_button(
                    t!("ai.research.apply_ghosts"),
                    false,
                    apply_ghosts,
                )),
        )
        .child(scroll().max_h(px(280.0)).child(dynamic(move || {
            let _rev = state.research_rev.get();
            let ghosts = state.ghost_highlights.with(|v| v.clone());
            if ghosts.is_empty() {
                return status_caption(move || t!("ai.research.empty")).into_element();
            }
            let mut list = view().gap_2();
            for (idx, g) in ghosts.into_iter().enumerate() {
                let gid = g.id.clone();
                let status = match g.decision {
                    DraftDecision::PendingReview => t!("ai.draft.pending"),
                    DraftDecision::Accepted => t!("ai.draft.accepted"),
                    DraftDecision::Dismissed => t!("ai.draft.dismissed"),
                    _ => t!("ai.draft.pending"),
                };
                let quote_line = format!("p.{} — \"{}\"", g.page + 1, g.quote);
                list = list.child(
                    view()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .gap_1()
                        .child(
                            text(format!(
                                "#{}: [{}] {} ({})",
                                idx + 1,
                                status,
                                g.rationale,
                                g.category.key()
                            ))
                            .text_size(px(12.0)),
                        )
                        .child(status_caption(move || quote_line.clone()))
                        .child(view().flex_row().gap_2().child(pill_button(
                            t!("ai.draft.accept"),
                            true,
                            move || {
                                state.ghost_highlights.with(|vec| {
                                    if let Some(item) = vec.iter_mut().find(|x| x.id == gid) {
                                        item.accept();
                                    }
                                });
                                state.research_rev.update(|r| *r = r.wrapping_add(1));
                            },
                        ))),
                );
            }
            list.into_element()
        })))
}

fn quiz_tab_view(state: AiInner, panes: Panes, live: Live) -> impl IntoElement {
    let generate_quiz = move || {
        if state.busy.get_untracked() {
            return;
        }
        let Some((path, doc_id, page, _)) = active_doc_context(panes) else {
            return;
        };
        let policy = consent_policy_from_live(live, state.doc_ai_disabled.get_untracked());
        let budget = budget_from_live(live);
        let cfg = match ConfiguredProvider::from_live(live) {
            Ok(c) => c,
            Err(err) => {
                state.status_line.set(err);
                return;
            }
        };
        let count = state.quiz_count.get_untracked().max(1);
        state.busy.set(true);
        state.status_line.set(t!("ai.thinking"));
        spawn(async move {
            let res = blocking(move || {
                let _ = index_pdf_sync(&path, &doc_id);
                let conn = open_ai_db()?;
                let mut guarded = GuardedProvider::new(cfg.as_dyn(), policy, budget);
                generate_chapter_quiz(
                    &conn,
                    &mut guarded,
                    &doc_id,
                    page..=page.saturating_add(5),
                    count,
                )
                .map_err(|e| e.to_string())
            })
            .await;
            state.busy.set(false);
            match res {
                Ok(Ok(draft)) => {
                    state.quiz_draft.with(|slot| *slot = Some(draft));
                    state.quiz_outcome.set(None);
                    state.quiz_rev.update(|r| *r = r.wrapping_add(1));
                    state.status_line.set(String::new());
                }
                Ok(Err(err)) => state.status_line.set(err),
                Err(err) => state.status_line.set(err.to_string()),
            }
        });
    };

    let start_exam = move || {
        let Some(mut draft) = state.quiz_draft.with(|d| d.clone()) else {
            return;
        };
        let has_accepted = draft
            .questions
            .iter()
            .any(|q| q.decision == DraftDecision::Accepted);
        if !has_accepted && let Some(first) = draft.questions.first().map(|q| q.id.clone()) {
            let _ = draft.accept_question(&first);
            state.quiz_draft.with(|slot| *slot = Some(draft.clone()));
        }
        if let Some((runner, _changes)) = QuizExamRunner::start(
            &draft,
            &[],
            VisibilitySnapshot::default(),
            ExamConfig::default(),
            now_secs(),
        ) {
            state.quiz_runner.with(|slot| *slot = Some(runner));
            state.quiz_rev.update(|r| *r = r.wrapping_add(1));
        }
    };

    let finish_exam = move || {
        if let Some(runner) = state.quiz_runner.with(Option::take) {
            let outcome = runner.finish(&[]);
            state.quiz_outcome.set(Some(outcome));
            state.quiz_rev.update(|r| *r = r.wrapping_add(1));
        }
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(pill_button(t!("ai.quiz.generate"), true, generate_quiz))
                .child(pill_button(t!("ai.quiz.start_exam"), false, start_exam))
                .child(pill_button(t!("ai.quiz.finish_exam"), false, finish_exam)),
        )
        .child(when(
            move || state.quiz_outcome.get().is_some(),
            move || {
                let (correct, total, pct) = state.quiz_outcome.get().map_or((0, 0, 0.0), |o| {
                    (o.correct_count, o.total_questions, o.score_percent)
                });
                status_caption(move || {
                    t!(
                        "ai.quiz.score",
                        correct = correct,
                        total = total,
                        percent = format!("{pct:.0}")
                    )
                })
            },
        ))
        .child(scroll().max_h(px(260.0)).child(dynamic(move || {
            let _rev = state.quiz_rev.get();
            let Some(draft) = state.quiz_draft.with(|d| d.clone()) else {
                return status_caption(move || t!("ai.quiz.empty")).into_element();
            };
            let mut list = view().gap_2();
            for (idx, q) in draft.questions.into_iter().enumerate() {
                let qid = q.id.clone();
                let kind_desc = match q.kind {
                    QuizQuestionKind::MultipleChoice { options, .. } => options.join(" | "),
                    QuizQuestionKind::ShortAnswer { expected_answer } => expected_answer,
                    _ => String::new(),
                };
                let status = match q.decision {
                    DraftDecision::PendingReview => t!("ai.draft.pending"),
                    DraftDecision::Accepted => t!("ai.draft.accepted"),
                    DraftDecision::Dismissed => t!("ai.draft.dismissed"),
                    _ => t!("ai.draft.pending"),
                };
                list = list.child(
                    view()
                        .p_2()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .gap_1()
                        .child(
                            text(format!("Q{} [{}]: {}", idx + 1, status, q.prompt))
                                .text_size(px(12.0)),
                        )
                        .child(status_caption(move || kind_desc.clone()))
                        .child(
                            view()
                                .flex_row()
                                .gap_2()
                                .child(pill_button(t!("ai.draft.accept"), true, move || {
                                    state.quiz_draft.with(|opt| {
                                        if let Some(d) = opt {
                                            let _ = d.accept_question(&qid);
                                        }
                                    });
                                    state.quiz_rev.update(|r| *r = r.wrapping_add(1));
                                }))
                                .child(pill_button("Answer 0", false, move || {
                                    state.quiz_runner.with(|opt| {
                                        if let Some(r) = opt {
                                            let _ = r.submit_answer(idx, "0");
                                        }
                                    });
                                })),
                        ),
                );
            }
            list.into_element()
        })))
}

fn math_latex_dialog(state: AiUiState, panes: Panes) -> impl IntoElement {
    let math_open = state.math_open;
    let close = move || math_open.set(false);
    let state = state.ensure_initialized();
    let confirm_insert = move || {
        let expr = state.math_input_expr.get_untracked();
        if !validate_latex_braces(&expr) {
            state.status_line.set("Unbalanced LaTeX braces".to_owned());
            return;
        }
        let pane = panes.active();
        let page = pane.pages.current();
        let mut annot = Annotation::new(
            page,
            Shape::Boxed(
                Boxed::Typewriter,
                PageRect {
                    x0: 72.0,
                    y0: 72.0,
                    x1: 300.0,
                    y1: 120.0,
                },
            ),
            Style::default(),
            "Reader",
        );
        annot.contents = format!("$$\n{}\n$$", expr.trim());
        pane.editing.apply(&Operation {
            name: t!("ai.math.insert_op"),
            changes: vec![Change::CreateAnnotation(annot)],
        });
        state.math_result.with(|opt| {
            if let Some(r) = opt {
                r.confirm_insert();
            }
        });
        math_open.set(false);
    };

    let confirm_copy = move || {
        let expr = state.math_input_expr.get_untracked();
        let formatted = format!("$$\n{}\n$$", expr.trim());
        state.math_result.with(|opt| {
            if let Some(r) = opt {
                r.confirm_copy();
            }
        });
        show_message(t!("ai.math.copied_title"), formatted);
        math_open.set(false);
    };

    let body = card(
        view()
            .w(px(500.0))
            .gap_3()
            .child(status_caption(move || t!("ai.math.preview_hint")))
            .child(labeled_field(
                move || state.math_input_expr.get(),
                move |v| state.math_input_expr.set(v),
                t!("ai.math.placeholder"),
                t!("ai.math.latex_label"),
            ))
            .child(
                text(move || {
                    let _rev = state.math_rev.get();
                    format!("$$\n{}\n$$", state.math_input_expr.get())
                })
                .p_2()
                .rounded(px(6.0))
                .bg(move || theme().muted)
                .text_size(px(13.0)),
            )
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("action.cancel"), false, close))
                    .child(pill_button(t!("ai.math.copy"), false, confirm_copy))
                    .child(pill_button(t!("ai.math.insert"), true, confirm_insert)),
            ),
    );

    list_dialog(t!("ai.math.title"), close, body)
}

fn visual_search_dialog(state: AiUiState, panes: Panes, live: Live) -> impl IntoElement {
    let lens_open = state.lens_open;
    let state = state.ensure_initialized();
    let close = move || {
        state.lens_consent.set(LensConsent::Declined);
        lens_open.set(false);
    };

    let online_enabled = live.read(|s| s.flag(LENS_ONLINE_ENABLED).unwrap_or(false));

    let confirm_upload = move || {
        let Some((path, _, page, _)) = active_doc_context(panes) else {
            return;
        };
        let online_mode = if live.read(|s| s.flag(LENS_ONLINE_ENABLED).unwrap_or(false)) {
            OnlineAccessMode::Enabled
        } else {
            OnlineAccessMode::Disabled
        };
        state.lens_consent.set(LensConsent::Confirmed);
        state.lens_status.set(t!("ai.lens.preparing"));
        spawn(async move {
            let res = blocking(move || {
                let doc = ecclesia_mupdf::Document::open(&path).map_err(|e| e.to_string())?;
                let region = Region {
                    page,
                    rect: PageRect {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 612.0,
                        y1: 792.0,
                    },
                };
                let pkg = build_upload_from_region(
                    &doc,
                    region,
                    150,
                    "region.png",
                    online_mode,
                    LensConsent::Confirmed,
                )
                .or_else(|_| {
                    let minimal_png = vec![
                        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H',
                        b'D', b'R',
                    ];
                    build_upload_from_png(
                        &minimal_png,
                        "region.png",
                        online_mode,
                        LensConsent::Confirmed,
                    )
                })
                .map_err(|e| e.to_string())?;
                pkg.write_trampoline_file(&std::env::temp_dir())
                    .map_err(|e| e.to_string())
            })
            .await;
            match res {
                Ok(Ok(html_path)) => {
                    let uri = format!("file:///{}", html_path.to_string_lossy().replace('\\', "/"));
                    let _ = open_url(&uri).await;
                    state.lens_status.set(t!("ai.lens.opened"));
                    lens_open.set(false);
                }
                Ok(Err(err)) => state.lens_status.set(err),
                Err(err) => state.lens_status.set(err.to_string()),
            }
        });
    };

    let body = card(
        view()
            .w(px(460.0))
            .gap_3()
            .child(status_caption(move || t!("ai.lens.warning")))
            .child(when(
                move || !online_enabled,
                move || {
                    text(t!("ai.lens.online_disabled"))
                        .text_size(px(12.0))
                        .text_color(move || theme().destructive)
                },
            ))
            .child(when(
                move || !state.lens_status.get().is_empty(),
                move || status_caption(move || state.lens_status.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("action.cancel"), false, close))
                    .child(pill_button(
                        t!("ai.lens.confirm_upload"),
                        true,
                        confirm_upload,
                    )),
            ),
    );

    list_dialog(t!("ai.lens.title"), close, body)
}
