//! Phase 8 & 9 Ecosystem, Tablet Scanner, Spatial Excerpt Board, Knowledge Graph, Sync & Relay,
//! Live Classroom, Sandboxed Plugins, and Sandbox Limits UI
//! (`O.21`, `X.1`–`X.4`, `F.14`, `Y.1`–`Y.4`, `W.13`, `G.14`, `G.15`, `E.5`, `K.22`, `K.23`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{SigningKey, VerifyingKey};
use excellent_core::anchor::new_id;
use excellent_core::export_doc::PdfPaperSize;
use excellent_core::sandbox::{OsSandboxConfig, OsSandboxGuard, SecurityLimits};
use excellent_features::camera_scan::{
    CleanupPreset, KeystoneTarget, RawImage, ScanPdfOptions, ScanPixelFormat, ScanSource,
    build_scanned_pdf, detect_document_quads,
};
use excellent_features::canvas_board::evidence::{
    export_evidence_chains_markdown, query_evidence_chains, validate_evidence_dag,
};
use excellent_features::canvas_board::export::{
    export_board_markdown, export_board_outline, export_board_png_summary, export_board_svg,
    export_exboard_zip, import_exboard_zip,
};
use excellent_features::canvas_board::{
    ArgumentRole, BoardCard, BoardConnector, BoardRect, CanvasBoard, CardKind, ConnectorKind,
    DocumentAnchor,
};
use excellent_features::classroom::poll::{PollHistogram, QuickPoll, VotePrivacy};
use excellent_features::classroom::protocol::{FollowMode, JoinCode};
use excellent_features::classroom::transport::InMemoryClassroomBus;
use excellent_features::classroom::{ClassroomHost, ClassroomStudent};
use excellent_features::graph::layout::LayoutConfig;
use excellent_features::graph::query::{GraphFilter, OrphanVisibility, filter_graph};
use excellent_features::graph::{DocumentGraphInput, GraphNodeKind, KnowledgeGraph};
use excellent_features::notes::Notes;
use excellent_features::plugins::PluginRegistry;
use excellent_features::plugins::manifest::{
    PluginCapability, PluginHook, PluginManifest, pack_signed_plugin, verify_signed_plugin,
};
use excellent_features::sync::bundle::{
    BundleNote, BundleVerification, ShareBundle, pack_exbundle, unpack_exbundle,
};
use excellent_features::sync::crdt::HlcTimestamp;
use excellent_features::sync::folder::{
    DevicePathMap, FolderSync, PayloadSecurity, export_workspaces_to_sync,
    import_workspaces_from_sync,
};
use excellent_features::sync::relay::{CoEditPeer, InviteLink, LocalRelayHub, PresenceCursor};
use excellent_features::sync::settings::{TargetOs, materialize_for_os, sanitize_local_settings};
use excellent_mupdf::{Background, Matrix};
use excellent_store::library::{LibraryFilter, list_items};
use nsh_base::theme;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::ai_ui::{labeled_field, pill_button, status_caption};
use crate::library as app_library;
use crate::list_dialog::{card, list_dialog};
use crate::panes::Panes;
use crate::settings::{Live, write_atomically};
use crate::settings_window::{segment_owned, segmented};
use crate::workspaces::Workspaces;

/// `settings.toml` key selecting strict `SecurityLimits` vs standard limits (`K.23`).
pub(crate) const SANDBOX_STRICT_MODE: &str = "sandbox_strict_mode";
/// `settings.toml` key overriding maximum permitted PDF page count (`K.23`).
pub(crate) const SANDBOX_MAX_PAGES: &str = "sandbox_max_pages";
/// `settings.toml` key overriding sandboxed worker memory cap in MiB (`K.22`).
pub(crate) const SANDBOX_MEMORY_CAP_MB: &str = "sandbox_memory_cap_mb";
/// `settings.toml` key overriding sandboxed worker watchdog timeout in seconds (`K.22`).
pub(crate) const SANDBOX_TIMEOUT_SECS: &str = "sandbox_timeout_secs";

/// Which Phase 8/9 dialog is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum EcoDialog {
    /// No Phase 8/9 dialog open.
    #[default]
    None,
    /// `O.21` Document camera scan import & perspective cleanup.
    CameraScan,
    /// `X.1`–`X.4` Infinite canvas excerpt board & evidence chains.
    ExcerptBoard,
    /// `F.14` Library knowledge graph view.
    KnowledgeGraph,
    /// `Y.1`–`Y.3`, `W.13` Folder sync, portable settings & `.exbundle` sharing.
    SyncAndShare,
    /// `Y.4` Live encrypted co-editing relay.
    LiveRelay,
    /// `G.14`, `G.15` Live classroom broadcast & quick polls.
    Classroom,
    /// `E.5` Sandboxed plugins & capability permissions manager.
    Plugins,
}

/// Active tab in the Sync & Share hub (`Y.1`–`Y.3`, `W.13`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SyncTab {
    /// `Y.1` & `W.13` CRDT folder & workspace sync.
    #[default]
    FolderSync,
    /// `Y.2` OS-portable settings sanitizer/exporter.
    PortableSettings,
    /// `Y.3` Signed `.exbundle` package export/import.
    ShareBundle,
}

#[derive(Clone)]
struct PendingPluginReview {
    manifest: PluginManifest,
    archive_bytes: Vec<u8>,
    verifying_key: VerifyingKey,
    approved_caps: BTreeSet<PluginCapability>,
}

/// Reactive state container for Phase 8 & 9 ecosystem dialogs.
#[derive(Clone, Copy)]
pub(crate) struct EcosystemUiState {
    active_dialog: Signal<EcoDialog>,
    status: Signal<String>,
    // O.21 Camera Scan
    scan_paths: Stored<Vec<PathBuf>>,
    scan_cleanup: Signal<CleanupPreset>,
    scan_paper: Signal<PdfPaperSize>,
    scan_rev: Signal<u64>,
    // X.1-X.4 Excerpt Board
    board: Stored<CanvasBoard>,
    board_rev: Signal<u64>,
    board_excerpt_title: Signal<String>,
    board_excerpt_text: Signal<String>,
    board_role: Signal<ArgumentRole>,
    board_conn_kind: Signal<ConnectorKind>,
    board_group_title: Signal<String>,
    // F.14 Knowledge Graph
    graph: Stored<KnowledgeGraph>,
    graph_rev: Signal<u64>,
    graph_kind: Signal<Option<GraphNodeKind>>,
    graph_search: Signal<String>,
    graph_hide_orphans: Signal<bool>,
    // Y.1-Y.4 & W.13 Sync / Relay
    sync_tab: Signal<SyncTab>,
    sync_folder: Signal<String>,
    sync_device_id: Signal<String>,
    sync_target_os: Signal<TargetOs>,
    relay_url: Signal<String>,
    relay_invite_uri: Signal<String>,
    relay_hub: Stored<LocalRelayHub>,
    relay_peer: Stored<Option<CoEditPeer>>,
    relay_rev: Signal<u64>,
    // G.14-G.15 Live Classroom
    classroom_is_host: Signal<bool>,
    classroom_title: Signal<String>,
    classroom_code: Signal<String>,
    classroom_bus: Stored<InMemoryClassroomBus>,
    classroom_host: Stored<
        Option<ClassroomHost<excellent_features::classroom::transport::InMemoryHostTransport>>,
    >,
    classroom_student: Stored<
        Option<
            ClassroomStudent<excellent_features::classroom::transport::InMemoryStudentTransport>,
        >,
    >,
    classroom_follow: Signal<FollowMode>,
    classroom_poll_question: Signal<String>,
    classroom_poll_options: Signal<String>,
    classroom_poll_named: Signal<bool>,
    classroom_last_histogram: Stored<Option<PollHistogram>>,
    classroom_rev: Signal<u64>,
    // E.5 Plugins
    plugins_registry: Stored<PluginRegistry>,
    plugins_pending: Stored<Option<PendingPluginReview>>,
    plugins_ids: Stored<Vec<String>>,
    plugins_key_hex: Signal<String>,
    plugins_rev: Signal<u64>,
}

impl EcosystemUiState {
    /// Initializes Phase 8 & 9 reactive UI state.
    pub(crate) fn new() -> Self {
        let bus = InMemoryClassroomBus::new();
        let seed_key = SigningKey::from_bytes(&[0x5Au8; 32]);
        let verifying_hex = bytes_to_hex(seed_key.verifying_key().as_bytes());
        Self {
            active_dialog: signal(EcoDialog::None),
            status: signal(String::new()),
            scan_paths: stored(Vec::new()),
            scan_cleanup: signal(CleanupPreset::DocumentColor),
            scan_paper: signal(PdfPaperSize::A4),
            scan_rev: signal(0),
            board: stored(CanvasBoard::new("board-1", "Research Canvas")),
            board_rev: signal(0),
            board_excerpt_title: signal(String::new()),
            board_excerpt_text: signal(String::new()),
            board_role: signal(ArgumentRole::Claim),
            board_conn_kind: signal(ConnectorKind::Supports),
            board_group_title: signal("Core Argument".to_owned()),
            graph: stored(KnowledgeGraph::new()),
            graph_rev: signal(0),
            graph_kind: signal(None),
            graph_search: signal(String::new()),
            graph_hide_orphans: signal(false),
            sync_tab: signal(SyncTab::FolderSync),
            sync_folder: signal(
                std::env::temp_dir()
                    .join("ExcellentPDF-Sync")
                    .to_string_lossy()
                    .into_owned(),
            ),
            sync_device_id: signal("desktop-1".to_owned()),
            sync_target_os: signal(current_target_os()),
            relay_url: signal("wss://relay.excellentpdf.local/ws".to_owned()),
            relay_invite_uri: signal(String::new()),
            relay_hub: stored(LocalRelayHub::new()),
            relay_peer: stored(None),
            relay_rev: signal(0),
            classroom_is_host: signal(true),
            classroom_title: signal("Lecture Session".to_owned()),
            classroom_code: signal(JoinCode::from_seed(2026).to_string()),
            classroom_bus: stored(bus),
            classroom_host: stored(None),
            classroom_student: stored(None),
            classroom_follow: signal(FollowMode::LockedToTeacher),
            classroom_poll_question: signal("Ready for the next section?".to_owned()),
            classroom_poll_options: signal("Yes; Need review; Example please".to_owned()),
            classroom_poll_named: signal(false),
            classroom_last_histogram: stored(None),
            classroom_rev: signal(0),
            plugins_registry: stored(PluginRegistry::new()),
            plugins_pending: stored(None),
            plugins_ids: stored(Vec::new()),
            plugins_key_hex: signal(verifying_hex),
            plugins_rev: signal(0),
        }
    }

    /// Whether any Phase 8/9 modal hides the native page canvas.
    pub(crate) fn any_dialog_open(self) -> bool {
        self.active_dialog.get() != EcoDialog::None
    }

    /// Closes any active Phase 8/9 modal.
    pub(crate) fn close(self) {
        self.active_dialog.set(EcoDialog::None);
    }

    /// Opens `dialog`, refreshing graph/session state from `panes`.
    pub(crate) fn open_dialog(self, dialog: EcoDialog, panes: Panes) {
        self.status.set(String::new());
        if dialog == EcoDialog::KnowledgeGraph {
            rebuild_knowledge_graph(self, panes);
        }
        self.active_dialog.set(dialog);
    }
}

/// Active sandbox limits read from `live` settings (`K.22`, `K.23`).
pub(crate) fn security_limits_from_live(live: Live) -> SecurityLimits {
    live.read(|s| {
        let mut limits = if s.flag(SANDBOX_STRICT_MODE).unwrap_or(false) {
            SecurityLimits::strict()
        } else {
            SecurityLimits::default()
        };
        if let Some(pages) = s.number(SANDBOX_MAX_PAGES)
            && pages >= 1.0
        {
            limits.max_pages = pages as usize;
        }
        if let Some(mb) = s.number(SANDBOX_MEMORY_CAP_MB)
            && mb >= 64.0
        {
            limits.worker_memory_cap_bytes = (mb as u64).saturating_mul(1024 * 1024);
        }
        if let Some(secs) = s.number(SANDBOX_TIMEOUT_SECS)
            && secs >= 1.0
        {
            limits.worker_timeout = std::time::Duration::from_secs(secs as u64);
        }
        limits
    })
}

/// Human-readable summary of the active OS sandbox mechanism & token state (`K.22`).
pub(crate) fn sandbox_status_summary(live: Live) -> String {
    let limits = security_limits_from_live(live);
    let os_cfg = OsSandboxConfig {
        memory_cap_bytes: limits.worker_memory_cap_bytes,
        ..OsSandboxConfig::default()
    };
    match OsSandboxGuard::create(os_cfg) {
        Ok(guard) => {
            let att = guard.attachment();
            format!(
                "{:?} | {:?} | {} MiB | {}s | {}p",
                att.kind,
                att.token_state,
                limits.worker_memory_cap_bytes / (1024 * 1024),
                limits.worker_timeout.as_secs(),
                limits.max_pages
            )
        }
        Err(err) => err.to_string(),
    }
}

/// Renders whichever Phase 8/9 ecosystem dialog is active.
pub(crate) fn ecosystem_dialogs(
    state: EcosystemUiState,
    panes: Panes,
    workspaces: Workspaces,
    live: Live,
) -> impl IntoElement {
    dynamic(move || match state.active_dialog.get() {
        EcoDialog::None => view().into_element(),
        EcoDialog::CameraScan => camera_scan_dialog(state, workspaces).into_element(),
        EcoDialog::ExcerptBoard => excerpt_board_dialog(state, panes, workspaces).into_element(),
        EcoDialog::KnowledgeGraph => {
            knowledge_graph_dialog(state, panes, workspaces).into_element()
        }
        EcoDialog::SyncAndShare => sync_share_dialog(state, panes, workspaces, live).into_element(),
        EcoDialog::LiveRelay => live_relay_dialog(state, panes).into_element(),
        EcoDialog::Classroom => classroom_dialog(state, panes).into_element(),
        EcoDialog::Plugins => plugins_dialog(state).into_element(),
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn current_target_os() -> TargetOs {
    if cfg!(target_os = "macos") {
        TargetOs::MacOs
    } else if cfg!(target_os = "linux") {
        TargetOs::Linux
    } else {
        TargetOs::Windows
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn parse_verifying_key_hex(hex: &str) -> Result<VerifyingKey, String> {
    let cleaned: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() != 64 {
        return Err("Ed25519 verifying key must be 64 hex chars".to_owned());
    }
    let mut arr = [0u8; 32];
    for (i, slot) in arr.iter_mut().enumerate() {
        let pair = &cleaned[i * 2..i * 2 + 2];
        *slot = u8::from_str_radix(pair, 16)
            .map_err(|_| "invalid hex byte in verifying key".to_owned())?;
    }
    VerifyingKey::from_bytes(&arr).map_err(|e| e.to_string())
}

fn decode_path_to_raw_image(path: &PathBuf) -> Result<RawImage, String> {
    let doc = excellent_mupdf::Document::open(path).map_err(|e| e.to_string())?;
    let page = doc.load_page(0).map_err(|e| e.to_string())?;
    let pixmap = page
        .render(&Matrix::IDENTITY, Background::White)
        .map_err(|e| e.to_string())?;
    let w = pixmap.width();
    let h = pixmap.height();
    let stride = pixmap.stride();
    let comps = pixmap.components();
    let samples = pixmap.samples();
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for y in 0..(h as usize) {
        let row = &samples[y * stride..];
        for x in 0..(w as usize) {
            let off = x * comps;
            if off + 2 < row.len() {
                rgb.push(row[off]);
                rgb.push(row[off + 1]);
                rgb.push(row[off + 2]);
            }
        }
    }
    RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb).map_err(|e| e.to_string())
}

fn camera_scan_dialog(state: EcosystemUiState, workspaces: Workspaces) -> impl IntoElement {
    let close = move || state.close();

    let pick_scans = move || {
        let dialog = OpenDialog::files()
            .title(t!("eco.scan.pick"))
            .multiple(true)
            .filter(FileFilter::new(
                t!("eco.scan.filter"),
                ["png", "jpg", "jpeg", "bmp", "webp", "pdf"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await {
                let count = paths.len();
                state.scan_paths.with(|slot| *slot = paths);
                state.scan_rev.update(|r| *r = r.wrapping_add(1));
                state.status.set(t!("eco.scan.queued", count = count));
            }
        });
    };

    let build_scan_pdf = move || {
        let paths = state.scan_paths.with(|p| p.clone());
        if paths.is_empty() {
            state.status.set(t!("eco.scan.empty"));
            return;
        }
        let cleanup = state.scan_cleanup.get_untracked();
        let paper = state.scan_paper.get_untracked();
        let save = SaveDialog::new()
            .title(t!("eco.scan.save"))
            .file_name("scanned-document.pdf")
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(save);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let res = blocking(move || -> Result<(Vec<u8>, usize, usize), String> {
                let mut sources = Vec::with_capacity(paths.len());
                let mut quad_hits = 0usize;
                for p in &paths {
                    let img = decode_path_to_raw_image(p)?;
                    let quads = detect_document_quads(&img).unwrap_or_default();
                    let chosen = quads.first().map(|q| q.corners);
                    if chosen.is_some() {
                        quad_hits += 1;
                    }
                    sources.push(ScanSource::RawCameraFrame {
                        image: img,
                        quad: chosen,
                    });
                }
                let opts = ScanPdfOptions {
                    paper,
                    cleanup,
                    keystone: KeystoneTarget::AutoAspect,
                    ..ScanPdfOptions::default()
                };
                let out = build_scanned_pdf(&sources, &opts, None).map_err(|e| e.to_string())?;
                Ok((out.pdf_bytes, out.page_count, quad_hits))
            })
            .await;
            match res {
                Ok(Ok((pdf_bytes, page_count, quads))) => {
                    if write_atomically(&dest, &pdf_bytes).is_ok() {
                        state
                            .status
                            .set(t!("eco.scan.built", pages = page_count, quads = quads));
                        workspaces.open_paths(vec![dest]);
                    }
                }
                Ok(Err(err)) => state.status.set(err),
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let body = card(
        view()
            .w(px(560.0))
            .gap_3()
            .child(status_caption(move || {
                let _r = state.scan_rev.get();
                let count = state.scan_paths.with(|p| p.len());
                t!("eco.scan.queued", count = count)
            }))
            .child(
                segmented()
                    .child(segment_owned(
                        t!("eco.scan.preset_color"),
                        move || state.scan_cleanup.get() == CleanupPreset::DocumentColor,
                        move || state.scan_cleanup.set(CleanupPreset::DocumentColor),
                    ))
                    .child(segment_owned(
                        t!("eco.scan.preset_whiteboard"),
                        move || state.scan_cleanup.get() == CleanupPreset::Whiteboard,
                        move || state.scan_cleanup.set(CleanupPreset::Whiteboard),
                    ))
                    .child(segment_owned(
                        t!("eco.scan.preset_bw"),
                        move || state.scan_cleanup.get() == CleanupPreset::BlackAndWhite,
                        move || state.scan_cleanup.set(CleanupPreset::BlackAndWhite),
                    ))
                    .child(segment_owned(
                        t!("eco.scan.preset_original"),
                        move || state.scan_cleanup.get() == CleanupPreset::Original,
                        move || state.scan_cleanup.set(CleanupPreset::Original),
                    )),
            )
            .child(
                segmented()
                    .child(segment_owned(
                        "A4".to_owned(),
                        move || state.scan_paper.get() == PdfPaperSize::A4,
                        move || state.scan_paper.set(PdfPaperSize::A4),
                    ))
                    .child(segment_owned(
                        "Letter".to_owned(),
                        move || state.scan_paper.get() == PdfPaperSize::Letter,
                        move || state.scan_paper.set(PdfPaperSize::Letter),
                    ))
                    .child(segment_owned(
                        t!("eco.scan.paper_fit"),
                        move || state.scan_paper.get() == PdfPaperSize::FitToImage,
                        move || state.scan_paper.set(PdfPaperSize::FitToImage),
                    )),
            )
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .justify_between()
                    .gap_2()
                    .child(pill_button(t!("eco.scan.pick_btn"), false, pick_scans))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(pill_button(t!("action.close"), false, close))
                            .child(pill_button(t!("eco.scan.build_btn"), true, build_scan_pdf)),
                    ),
            ),
    );
    list_dialog(t!("eco.scan.title"), close, body)
}

fn excerpt_board_dialog(
    state: EcosystemUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();

    let add_excerpt = move || {
        let title_draft = state.board_excerpt_title.get_untracked();
        let body_draft = state.board_excerpt_text.get_untracked();
        let role = state.board_role.get_untracked();
        let active_path = panes.active().tabs.shown_path();
        let page = panes.active().pages.current();
        let text_val = if !body_draft.trim().is_empty() {
            body_draft.trim().to_owned()
        } else if let Some(ref path) = active_path
            && let Ok(doc) = excellent_mupdf::Document::open(path)
            && let Ok(pg) = doc.load_page(page)
            && let Ok(tp) = pg.text(None)
            && let Ok(raw) = tp.text()
            && !raw.trim().is_empty()
        {
            raw.trim().chars().take(160).collect()
        } else {
            format!("Excerpt from page {}", page + 1)
        };
        let card_id = format!("card-{}", new_id());
        let heading = if title_draft.trim().is_empty() {
            format!("Note p.{}", page + 1)
        } else {
            title_draft.trim().to_owned()
        };
        let count = state.board.with(|b| b.card_count());
        let col = (count % 4) as f64;
        let row = (count / 4) as f64;
        let kind = match active_path {
            Some(path) => CardKind::TextExcerpt {
                anchor: DocumentAnchor {
                    document_path: path.clone(),
                    document_id: path.to_string_lossy().into_owned(),
                    page,
                    quads: Vec::new(),
                    quote: None,
                },
                excerpt: text_val,
            },
            None => CardKind::StickyNote { markdown: text_val },
        };
        state.board.with(|b| {
            b.upsert_card(BoardCard {
                id: card_id,
                title: heading,
                bounds: BoardRect::new(40.0 + col * 240.0, 40.0 + row * 160.0, 210.0, 125.0),
                role,
                kind,
                group_id: None,
                color: None,
                tags: Vec::new(),
            });
        });
        state.board_excerpt_title.set(String::new());
        state.board_excerpt_text.set(String::new());
        state.board_rev.update(|r| *r = r.wrapping_add(1));
    };

    let group_all_cards = move || {
        let gtitle = state.board_group_title.get_untracked();
        let label = if gtitle.trim().is_empty() {
            "Argument Group".to_owned()
        } else {
            gtitle.trim().to_owned()
        };
        state.board.with(|b| {
            let ids: Vec<String> = b.card_ids().map(str::to_owned).collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            if let Err(err) = b.upsert_group("grp-main", &label, "#3b82f6", &refs) {
                state.status.set(err.to_string());
            } else {
                state
                    .status
                    .set(t!("eco.board.grouped", count = refs.len()));
            }
        });
        state.board_rev.update(|r| *r = r.wrapping_add(1));
    };

    let connect_last_two = move || {
        let kind = state.board_conn_kind.get_untracked();
        state.board.with(|b| {
            let ids: Vec<String> = b.card_ids().map(str::to_owned).collect();
            if ids.len() < 2 {
                state.status.set(t!("eco.board.need_two"));
                return;
            }
            let from = ids[ids.len() - 1].clone();
            let to = ids[ids.len() - 2].clone();
            let cid = format!("conn-{}", new_id());
            match b.connect(BoardConnector {
                id: cid,
                from_card: from,
                to_card: to,
                label: format!("{kind:?}"),
                kind,
                control_offset: (0.0, 0.0),
            }) {
                Ok(()) => state.status.set(t!("eco.board.connected")),
                Err(err) => state.status.set(err.to_string()),
            }
        });
        state.board_rev.update(|r| *r = r.wrapping_add(1));
    };

    let validate_chains = move || {
        let summary = state.board.with(|b| match validate_evidence_dag(b) {
            Ok(order) => {
                let first_chains = order
                    .iter()
                    .find_map(|id| {
                        let chains = query_evidence_chains(b, id).ok()?;
                        (!chains.is_empty()).then_some(chains.len())
                    })
                    .unwrap_or(0);
                format!(
                    "DAG OK ({} nodes, {} evidence chains)",
                    order.len(),
                    first_chains
                )
            }
            Err(err) => err.to_string(),
        });
        state.status.set(summary);
    };

    let export_board = move |kind: &'static str| {
        let (ext, payload): (&str, Result<Vec<u8>, String>) = state.board.with(|b| match kind {
            "exboard" => ("exboard", export_exboard_zip(b).map_err(|e| e.to_string())),
            "svg" => ("svg", Ok(export_board_svg(b).into_bytes())),
            "png" => (
                "png",
                export_board_png_summary(b, 1200, 800).map_err(|e| e.to_string()),
            ),
            "chains" => (
                "md",
                export_evidence_chains_markdown(b)
                    .map(String::into_bytes)
                    .map_err(|e| e.to_string()),
            ),
            _ => ("md", Ok(export_board_markdown(b).into_bytes())),
        });
        let data = match payload {
            Ok(bytes) => bytes,
            Err(err) => {
                state.status.set(err);
                return;
            }
        };
        let dialog = SaveDialog::new()
            .title(t!("eco.board.export"))
            .file_name(format!("research-board.{ext}"))
            .filter(FileFilter::new(ext.to_uppercase(), [ext]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(path)) = answer.await
                && write_atomically(&path, &data).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    let import_board = move || {
        let dialog = OpenDialog::files()
            .title(t!("eco.board.import"))
            .filter(FileFilter::new("ExcellentPDF Board", ["exboard"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            if let Ok(bytes) = std::fs::read(&path) {
                match import_exboard_zip(&bytes) {
                    Ok(loaded) => {
                        let count = loaded.card_count();
                        state.board.with(|slot| *slot = loaded);
                        state.board_rev.update(|r| *r = r.wrapping_add(1));
                        state.status.set(t!("eco.board.imported", count = count));
                    }
                    Err(err) => state.status.set(err.to_string()),
                }
            }
        });
    };

    let body = card(
        view()
            .w(px(660.0))
            .max_h(px(560.0))
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(labeled_field(
                        move || state.board_excerpt_title.get(),
                        move |v| state.board_excerpt_title.set(v),
                        t!("eco.board.title_placeholder"),
                        t!("eco.board.title_label"),
                    ))
                    .child(labeled_field(
                        move || state.board_excerpt_text.get(),
                        move |v| state.board_excerpt_text.set(v),
                        t!("eco.board.text_placeholder"),
                        t!("eco.board.text_label"),
                    )),
            )
            .child(
                segmented()
                    .child(segment_owned(
                        t!("eco.board.role_claim"),
                        move || state.board_role.get() == ArgumentRole::Claim,
                        move || state.board_role.set(ArgumentRole::Claim),
                    ))
                    .child(segment_owned(
                        t!("eco.board.role_support"),
                        move || state.board_role.get() == ArgumentRole::Support,
                        move || state.board_role.set(ArgumentRole::Support),
                    ))
                    .child(segment_owned(
                        t!("eco.board.role_objection"),
                        move || state.board_role.get() == ArgumentRole::Objection,
                        move || state.board_role.set(ArgumentRole::Objection),
                    ))
                    .child(segment_owned(
                        t!("eco.board.role_evidence"),
                        move || state.board_role.get() == ArgumentRole::Evidence,
                        move || state.board_role.set(ArgumentRole::Evidence),
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("eco.board.add_card"), true, add_excerpt))
                    .child(pill_button(
                        t!("eco.board.group_btn"),
                        false,
                        group_all_cards,
                    ))
                    .child(pill_button(
                        t!("eco.board.connect_btn"),
                        false,
                        connect_last_two,
                    ))
                    .child(pill_button(
                        t!("eco.board.validate_dag"),
                        false,
                        validate_chains,
                    ))
                    .child(pill_button(t!("eco.board.zoom_in"), false, move || {
                        state
                            .board
                            .with(|b| b.viewport_mut().zoom_at(1.25, 300.0, 200.0));
                        state.board_rev.update(|r| *r = r.wrapping_add(1));
                    })),
            )
            .child(scroll().max_h(px(190.0)).child(dynamic(move || {
                let _rev = state.board_rev.get();
                state.board.with(|b| {
                    let outline = export_board_outline(b);
                    let anchors: Vec<(String, PathBuf, usize)> = b
                        .cards()
                        .filter_map(|c| {
                            c.anchor().map(|a| {
                                (
                                    format!("{} (p.{})", c.title, a.page + 1),
                                    a.document_path.clone(),
                                    a.page,
                                )
                            })
                        })
                        .collect();
                    let mut col = view().gap_2().child(
                        text(outline)
                            .text_size(px(12.0))
                            .text_color(move || theme().foreground),
                    );
                    for (label, path, page) in anchors {
                        col = col.child(pill_button(
                            t!("eco.board.jump_source", item = label),
                            false,
                            move || {
                                workspaces.open_paths(vec![path.clone()]);
                                panes.active().go_to(page);
                            },
                        ));
                    }
                    col
                })
            })))
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .child(pill_button("SVG", false, move || export_board("svg")))
                            .child(pill_button("PNG", false, move || export_board("png")))
                            .child(pill_button("MD", false, move || export_board("md")))
                            .child(pill_button(
                                t!("eco.board.export_chains"),
                                false,
                                move || export_board("chains"),
                            ))
                            .child(pill_button(".exboard", false, move || {
                                export_board("exboard")
                            }))
                            .child(pill_button(t!("eco.board.import"), false, import_board)),
                    )
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );
    list_dialog(t!("eco.board.title"), close, body)
}

fn rebuild_knowledge_graph(state: EcosystemUiState, panes: Panes) {
    let mut docs = Vec::new();
    let mut notes = Vec::new();
    if let Some(store) = app_library::store() {
        if let Ok(items) = store.read(|conn| list_items(conn, &LibraryFilter::default())) {
            for item in items {
                docs.push(DocumentGraphInput {
                    title: if item.title.is_empty() {
                        item.path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    } else {
                        item.title
                    },
                    path: item.path,
                    tags: item.tags,
                    cited_keys: Vec::new(),
                    cluster: Some("library".to_owned()),
                });
            }
        }
        if let Ok(all_notes) = Notes::new(store).list() {
            notes = all_notes;
        }
    }
    if let Some(active) = panes.active().tabs.shown_path()
        && !docs.iter().any(|d| d.path == active)
    {
        docs.push(DocumentGraphInput {
            title: active
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Active PDF".to_owned()),
            path: active,
            tags: vec!["open".to_owned()],
            cited_keys: Vec::new(),
            cluster: Some("open".to_owned()),
        });
    }
    let mut built = KnowledgeGraph::from_library(&docs, &notes, &[]);
    let _metrics = built.run_layout(30, &LayoutConfig::default());
    state.graph.with(|g| *g = built);
    state.graph_rev.update(|r| *r = r.wrapping_add(1));
}

fn knowledge_graph_dialog(
    state: EcosystemUiState,
    panes: Panes,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || state.close();
    let body = card(
        view()
            .w(px(620.0))
            .max_h(px(540.0))
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(labeled_field(
                        move || state.graph_search.get(),
                        move |v| state.graph_search.set(v),
                        t!("eco.graph.search_placeholder"),
                        t!("eco.graph.search"),
                    ))
                    .child(pill_button(t!("eco.graph.refresh"), false, move || {
                        rebuild_knowledge_graph(state, panes)
                    }))
                    .child(pill_button(
                        t!("eco.graph.toggle_orphans"),
                        false,
                        move || state.graph_hide_orphans.update(|v| *v = !*v),
                    )),
            )
            .child(
                segmented()
                    .child(segment_owned(
                        t!("eco.graph.all"),
                        move || state.graph_kind.get().is_none(),
                        move || state.graph_kind.set(None),
                    ))
                    .child(segment_owned(
                        t!("eco.graph.docs"),
                        move || state.graph_kind.get() == Some(GraphNodeKind::Document),
                        move || state.graph_kind.set(Some(GraphNodeKind::Document)),
                    ))
                    .child(segment_owned(
                        t!("eco.graph.notes"),
                        move || state.graph_kind.get() == Some(GraphNodeKind::Note),
                        move || state.graph_kind.set(Some(GraphNodeKind::Note)),
                    ))
                    .child(segment_owned(
                        t!("eco.graph.tags"),
                        move || state.graph_kind.get() == Some(GraphNodeKind::Tag),
                        move || state.graph_kind.set(Some(GraphNodeKind::Tag)),
                    ))
                    .child(segment_owned(
                        t!("eco.graph.citations"),
                        move || state.graph_kind.get() == Some(GraphNodeKind::Citation),
                        move || state.graph_kind.set(Some(GraphNodeKind::Citation)),
                    )),
            )
            .child(scroll().max_h(px(260.0)).child(dynamic(move || {
                let _rev = state.graph_rev.get();
                let mut filter = GraphFilter::default();
                if let Some(kind) = state.graph_kind.get() {
                    filter.kinds.insert(kind);
                }
                let q = state.graph_search.get();
                if !q.trim().is_empty() {
                    filter.search_query = Some(q);
                }
                if state.graph_hide_orphans.get() {
                    filter.orphan_visibility = OrphanVisibility::Exclude;
                }
                let projection = state.graph.with(|g| filter_graph(g, &filter));
                let summary = format!(
                    "{} nodes / {} edges | clusters: {}",
                    projection.nodes.len(),
                    projection.edges.len(),
                    projection.cluster_counts.len()
                );
                let mut list = view()
                    .gap_2()
                    .child(status_caption(move || summary.clone()));
                for node in projection.nodes.into_iter().take(30) {
                    let doc_path = node
                        .id
                        .strip_prefix("doc:")
                        .map(PathBuf::from)
                        .filter(|p| p.exists());
                    let line = format!(
                        "[{:?}] {} ({}) @ ({:.0}, {:.0})",
                        node.kind, node.label, node.cluster, node.x, node.y
                    );
                    let row = view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(text(line).text_size(px(12.0)));
                    list = list.child(if let Some(path) = doc_path {
                        row.child(pill_button(t!("action.open"), false, move || {
                            workspaces.open_paths(vec![path.clone()]);
                        }))
                    } else {
                        row
                    });
                }
                list
            })))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );
    list_dialog(t!("eco.graph.title"), close, body)
}

fn sync_share_dialog(
    state: EcosystemUiState,
    panes: Panes,
    workspaces: Workspaces,
    live: Live,
) -> impl IntoElement {
    let close = move || state.close();

    let pick_sync_dir = move || {
        let dialog = OpenDialog::folders().title(t!("eco.sync.pick_folder"));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(dirs)) = answer.await
                && let Some(dir) = dirs.into_iter().next()
            {
                state.sync_folder.set(dir.to_string_lossy().into_owned());
            }
        });
    };

    let push_workspaces = move || {
        let Some(store) = app_library::store() else {
            state.status.set(t!("eco.sync.no_store"));
            return;
        };
        let root = state.sync_folder.get_untracked();
        let dev = state.sync_device_id.get_untracked();
        let fsync = FolderSync::new(&root, "workspaces", &dev, PayloadSecurity::Plaintext);
        let mut clock = HlcTimestamp::new(&dev, now_ms());
        let path_map = DevicePathMap::new();
        match export_workspaces_to_sync(store, &fsync, &path_map, &mut clock, now_ms()) {
            Ok(n) => state.status.set(t!("eco.sync.pushed", count = n)),
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let pull_workspaces = move || {
        let Some(store) = app_library::store() else {
            state.status.set(t!("eco.sync.no_store"));
            return;
        };
        let root = state.sync_folder.get_untracked();
        let dev = state.sync_device_id.get_untracked();
        let fsync = FolderSync::new(&root, "workspaces", &dev, PayloadSecurity::Plaintext);
        let cleaned = fsync.clean_conflicted_copies().unwrap_or(0);
        let path_map = DevicePathMap::new();
        match import_workspaces_from_sync(store, &fsync, &path_map) {
            Ok(n) => state
                .status
                .set(t!("eco.sync.pulled", count = n, cleaned = cleaned)),
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let export_portable = move || {
        let target_os = state.sync_target_os.get_untracked();
        let raw_json = live.read(|s| {
            serde_json::json!({
                "theme": s.text("theme").unwrap_or_default(),
                "language": s.text("language").unwrap_or_default(),
                "cad_enhance_mode": s.text(super::pro_pdf_ui::CAD_ENHANCE_MODE).unwrap_or("auto"),
                "ai_consent_mode": s.text(super::ai_ui::AI_CONSENT_MODE).unwrap_or("local"),
            })
        });
        let portable = sanitize_local_settings(&raw_json, current_target_os());
        let materialized = materialize_for_os(&portable, target_os);
        let bytes = serde_json::to_vec_pretty(&materialized).unwrap_or_default();
        let dialog = SaveDialog::new()
            .title(t!("eco.sync.export_settings"))
            .file_name("excellentpdf-portable-settings.json")
            .filter(FileFilter::new("JSON", ["json"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, &bytes).is_ok()
            {
                state.status.set(t!("pro.saved"));
            }
        });
    };

    let import_portable = move || {
        let dialog = OpenDialog::files()
            .title(t!("eco.sync.import_settings"))
            .filter(FileFilter::new("JSON", ["json"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
            {
                let portable = sanitize_local_settings(&val, current_target_os());
                let applied = materialize_for_os(&portable, current_target_os());
                let keys_count = applied.as_object().map_or(0, serde_json::Map::len);
                state
                    .status
                    .set(t!("eco.sync.imported_settings", count = keys_count));
            }
        });
    };

    let export_bundle = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let Ok(pdf_bytes) = std::fs::read(&path) else {
            return;
        };
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "bundle".to_owned());
        let dev = state.sync_device_id.get_untracked();
        let bundle = ShareBundle {
            bundle_id: new_id(),
            title: title.clone(),
            doc_id: path.to_string_lossy().into_owned(),
            author_device: dev.clone(),
            created_at: (now_ms() / 1000) as i64,
            pdf_bytes,
            annotations: Vec::new(),
            notes: vec![BundleNote {
                note_id: new_id(),
                title: title.clone(),
                body: format!("Shared from {}", path.display()),
                file_token: None,
                page: Some(panes.active().pages.current()),
                tags: Vec::new(),
                stamp: HlcTimestamp::new(&dev, now_ms()),
            }],
            boards: std::collections::BTreeMap::new(),
            audio_clips: std::collections::BTreeMap::new(),
        };
        let key = SigningKey::from_bytes(&[0x33u8; 32]);
        let Ok(zip_bytes) = pack_exbundle(&bundle, Some(&key)) else {
            return;
        };
        let dialog = SaveDialog::new()
            .title(t!("eco.sync.save_bundle"))
            .file_name(format!("{title}.exbundle"))
            .filter(FileFilter::new("ExcellentPDF Bundle", ["exbundle"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, &zip_bytes).is_ok()
            {
                state.status.set(t!("eco.sync.bundle_saved"));
            }
        });
    };

    let import_bundle = move || {
        let dialog = OpenDialog::files()
            .title(t!("eco.sync.open_bundle"))
            .filter(FileFilter::new("ExcellentPDF Bundle", ["exbundle"]));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            match unpack_exbundle(&bytes, BundleVerification::DigestOnly) {
                Ok(bundle) => {
                    let out_pdf = std::env::temp_dir().join(format!("{}.pdf", bundle.bundle_id));
                    if write_atomically(&out_pdf, &bundle.pdf_bytes).is_ok() {
                        workspaces.open_paths(vec![out_pdf]);
                        state.status.set(t!(
                            "eco.sync.bundle_opened",
                            title = bundle.title,
                            notes = bundle.notes.len()
                        ));
                    }
                }
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let tab_content = dynamic(move || match state.sync_tab.get() {
        SyncTab::FolderSync => view()
            .gap_3()
            .child(labeled_field(
                move || state.sync_folder.get(),
                move |v| state.sync_folder.set(v),
                t!("eco.sync.folder_placeholder"),
                t!("eco.sync.folder_label"),
            ))
            .child(labeled_field(
                move || state.sync_device_id.get(),
                move |v| state.sync_device_id.set(v),
                "desktop-1".to_owned(),
                t!("eco.sync.device_label"),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(
                        t!("eco.sync.pick_folder"),
                        false,
                        pick_sync_dir,
                    ))
                    .child(pill_button(t!("eco.sync.push_btn"), true, push_workspaces))
                    .child(pill_button(t!("eco.sync.pull_btn"), false, pull_workspaces)),
            )
            .into_element(),
        SyncTab::PortableSettings => view()
            .gap_3()
            .child(
                segmented()
                    .child(segment_owned(
                        "Windows".to_owned(),
                        move || state.sync_target_os.get() == TargetOs::Windows,
                        move || state.sync_target_os.set(TargetOs::Windows),
                    ))
                    .child(segment_owned(
                        "macOS".to_owned(),
                        move || state.sync_target_os.get() == TargetOs::MacOs,
                        move || state.sync_target_os.set(TargetOs::MacOs),
                    ))
                    .child(segment_owned(
                        "Linux".to_owned(),
                        move || state.sync_target_os.get() == TargetOs::Linux,
                        move || state.sync_target_os.set(TargetOs::Linux),
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(pill_button(
                        t!("eco.sync.export_settings"),
                        true,
                        export_portable,
                    ))
                    .child(pill_button(
                        t!("eco.sync.import_settings"),
                        false,
                        import_portable,
                    )),
            )
            .into_element(),
        SyncTab::ShareBundle => view()
            .gap_3()
            .child(status_caption(move || t!("eco.sync.bundle_hint")))
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(pill_button(t!("eco.sync.save_bundle"), true, export_bundle))
                    .child(pill_button(
                        t!("eco.sync.open_bundle"),
                        false,
                        import_bundle,
                    )),
            )
            .into_element(),
    });

    let body = card(
        view()
            .w(px(580.0))
            .gap_3()
            .child(
                segmented()
                    .child(segment_owned(
                        t!("eco.sync.tab_folder"),
                        move || state.sync_tab.get() == SyncTab::FolderSync,
                        move || state.sync_tab.set(SyncTab::FolderSync),
                    ))
                    .child(segment_owned(
                        t!("eco.sync.tab_settings"),
                        move || state.sync_tab.get() == SyncTab::PortableSettings,
                        move || state.sync_tab.set(SyncTab::PortableSettings),
                    ))
                    .child(segment_owned(
                        t!("eco.sync.tab_bundle"),
                        move || state.sync_tab.get() == SyncTab::ShareBundle,
                        move || state.sync_tab.set(SyncTab::ShareBundle),
                    )),
            )
            .child(tab_content)
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );
    list_dialog(t!("eco.sync.title"), close, body)
}

fn live_relay_dialog(state: EcosystemUiState, panes: Panes) -> impl IntoElement {
    let close = move || state.close();

    let host_room = move || {
        let link = InviteLink {
            relay_url: state.relay_url.get_untracked(),
            room_id: format!("room-{}", &new_id()[..8]),
            key: [0x7Bu8; 32],
        };
        let uri = link.to_uri();
        state.relay_invite_uri.set(uri);
        let dev = state.sync_device_id.get_untracked();
        let peer = CoEditPeer::from_invite(&dev, &link);
        state.relay_hub.with(|h| h.join_room(&link.room_id, &dev));
        state.relay_peer.with(|slot| *slot = Some(peer));
        state.relay_rev.update(|r| *r = r.wrapping_add(1));
        state.status.set(t!("eco.relay.hosted"));
    };

    let join_invite = move || {
        let raw = state.relay_invite_uri.get_untracked();
        match InviteLink::parse(raw.trim()) {
            Ok(link) => {
                let dev = state.sync_device_id.get_untracked();
                let peer = CoEditPeer::from_invite(&dev, &link);
                state.relay_hub.with(|h| h.join_room(&link.room_id, &dev));
                state.relay_peer.with(|slot| *slot = Some(peer));
                state.relay_rev.update(|r| *r = r.wrapping_add(1));
                state.status.set(t!("eco.relay.joined"));
            }
            Err(err) => state.status.set(err.to_string()),
        }
    };

    let broadcast_cursor = move || {
        let page = panes.active().pages.current();
        let dev = state.sync_device_id.get_untracked();
        let res = state.relay_peer.with(|opt| {
            let Some(peer) = opt else {
                return Err(t!("eco.relay.not_joined"));
            };
            let cursor = PresenceCursor {
                device_id: dev.clone(),
                display_name: dev.clone(),
                page,
                x: 120.0,
                y: 200.0,
                color: "#2563eb".to_owned(),
                updated_ms: now_ms(),
            };
            let frame = peer.emit_presence(cursor).map_err(|e| e.to_string())?;
            let room = peer.room_id.clone();
            state
                .relay_hub
                .with(|h| h.forward_frame(&room, &dev, frame));
            Ok::<_, String>(())
        });
        match res {
            Ok(()) => state.status.set(t!("eco.relay.sent_presence")),
            Err(err) => state.status.set(err),
        }
        state.relay_rev.update(|r| *r = r.wrapping_add(1));
    };

    let body = card(
        view()
            .w(px(560.0))
            .gap_3()
            .child(labeled_field(
                move || state.relay_url.get(),
                move |v| state.relay_url.set(v),
                "wss://relay.example.org/ws".to_owned(),
                t!("eco.relay.url_label"),
            ))
            .child(labeled_field(
                move || state.relay_invite_uri.get(),
                move |v| state.relay_invite_uri.set(v),
                "excellentpdf://collab/...".to_owned(),
                t!("eco.relay.invite_label"),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill_button(t!("eco.relay.host_btn"), true, host_room))
                    .child(pill_button(t!("eco.relay.join_btn"), false, join_invite))
                    .child(pill_button(
                        t!("eco.relay.presence_btn"),
                        false,
                        broadcast_cursor,
                    )),
            )
            .child(status_caption(move || {
                let _r = state.relay_rev.get();
                state.relay_peer.with(|opt| match opt {
                    Some(peer) => format!(
                        "Room {} | Peer {} | Cursors: {}",
                        peer.room_id,
                        peer.device_id,
                        peer.cursors.len()
                    ),
                    None => t!("eco.relay.disconnected"),
                })
            }))
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );
    list_dialog(t!("eco.relay.title"), close, body)
}

fn classroom_dialog(state: EcosystemUiState, panes: Panes) -> impl IntoElement {
    let close = move || state.close();

    let start_host = move || {
        let title = state.classroom_title.get_untracked();
        let code = JoinCode::try_new(&state.classroom_code.get_untracked())
            .unwrap_or_else(|_| JoinCode::from_seed(now_ms()));
        state.classroom_code.set(code.to_string());
        let transport = state.classroom_bus.with(|b| b.host_transport());
        let host = ClassroomHost::new(&title, code, transport);
        state.classroom_host.with(|slot| *slot = Some(host));
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
        state.status.set(t!("eco.class.host_started"));
    };

    let broadcast_page = move || {
        let page = panes.active().pages.current();
        let doc_id = panes
            .active()
            .tabs
            .shown_path()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "presentation.pdf".to_owned());
        let res = state.classroom_host.with(|opt| {
            let Some(host) = opt else {
                return Err(t!("eco.class.not_hosting"));
            };
            let _ = host.process_incoming();
            host.broadcast_focus(&doc_id, page, 0.0, 1.0, None)
                .map_err(|e| e.to_string())
        });
        match res {
            Ok(focus) => state
                .status
                .set(t!("eco.class.broadcasted", page = focus.page + 1)),
            Err(err) => state.status.set(err),
        }
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
    };

    let launch_poll = move || {
        let q = state.classroom_poll_question.get_untracked();
        let raw_opts = state.classroom_poll_options.get_untracked();
        let labels: Vec<String> = raw_opts
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let opt_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        if opt_refs.is_empty() {
            return;
        }
        let privacy = if state.classroom_poll_named.get_untracked() {
            VotePrivacy::Named
        } else {
            VotePrivacy::Anonymous
        };
        let poll = QuickPoll::with_privacy("poll-live", &q, &opt_refs, privacy);
        let res = state.classroom_host.with(|opt| {
            let Some(host) = opt else {
                return Err(t!("eco.class.not_hosting"));
            };
            host.publish_poll(poll).map_err(|e| e.to_string())
        });
        match res {
            Ok(()) => state.status.set(t!("eco.class.poll_published")),
            Err(err) => state.status.set(err),
        }
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
    };

    let close_poll = move || {
        let res = state.classroom_host.with(|opt| {
            let Some(host) = opt else {
                return Err(t!("eco.class.not_hosting"));
            };
            let _ = host.process_incoming();
            host.close_poll("poll-live").map_err(|e| e.to_string())
        });
        match res {
            Ok(hist) => {
                state
                    .classroom_last_histogram
                    .with(|slot| *slot = Some(hist));
                state.status.set(t!("eco.class.poll_closed"));
            }
            Err(err) => state.status.set(err),
        }
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
    };

    let join_as_student = move || {
        let Ok(code) = JoinCode::try_new(&state.classroom_code.get_untracked()) else {
            state.status.set(t!("eco.class.invalid_code"));
            return;
        };
        let transport = state.classroom_bus.with(|b| b.connect_student());
        let mut student = ClassroomStudent::new("student-local", Some("Student"), transport);
        student.set_follow_mode(state.classroom_follow.get_untracked());
        let res = student.join(&code).map_err(|e| e.to_string());
        let _ = state
            .classroom_host
            .with(|opt| opt.as_mut().map(|h| h.process_incoming()));
        let _ = student.sync_updates();
        state.classroom_student.with(|slot| *slot = Some(student));
        match res {
            Ok(()) => state.status.set(t!("eco.class.joined")),
            Err(err) => state.status.set(err),
        }
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
    };

    let student_vote_first = move || {
        let res = state.classroom_student.with(|opt| {
            let Some(stu) = opt else {
                return Err(t!("eco.class.not_joined"));
            };
            let _ = stu.sync_updates();
            stu.submit_vote("poll-live", 0).map_err(|e| e.to_string())
        });
        let _ = state
            .classroom_host
            .with(|opt| opt.as_mut().map(|h| h.process_incoming()));
        match res {
            Ok(()) => state.status.set(t!("eco.class.voted")),
            Err(err) => state.status.set(err),
        }
        state.classroom_rev.update(|r| *r = r.wrapping_add(1));
    };

    let role_view = dynamic(move || {
        if state.classroom_is_host.get() {
            view()
                .gap_3()
                .child(labeled_field(
                    move || state.classroom_title.get(),
                    move |v| state.classroom_title.set(v),
                    "Lecture".to_owned(),
                    t!("eco.class.session_title"),
                ))
                .child(labeled_field(
                    move || state.classroom_code.get(),
                    move |v| state.classroom_code.set(v),
                    "ABC123".to_owned(),
                    t!("eco.class.join_code"),
                ))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(pill_button(t!("eco.class.start_host"), true, start_host))
                        .child(pill_button(
                            t!("eco.class.broadcast_page"),
                            false,
                            broadcast_page,
                        )),
                )
                .child(labeled_field(
                    move || state.classroom_poll_question.get(),
                    move |v| state.classroom_poll_question.set(v),
                    t!("eco.class.poll_q_placeholder"),
                    t!("eco.class.poll_q"),
                ))
                .child(labeled_field(
                    move || state.classroom_poll_options.get(),
                    move |v| state.classroom_poll_options.set(v),
                    "Yes; No".to_owned(),
                    t!("eco.class.poll_opts"),
                ))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(pill_button(
                            t!("eco.class.publish_poll"),
                            false,
                            launch_poll,
                        ))
                        .child(pill_button(t!("eco.class.close_poll"), false, close_poll))
                        .child(pill_button(
                            t!("eco.class.toggle_named"),
                            false,
                            move || state.classroom_poll_named.update(|v| *v = !*v),
                        )),
                )
                .into_element()
        } else {
            view()
                .gap_3()
                .child(labeled_field(
                    move || state.classroom_code.get(),
                    move |v| state.classroom_code.set(v),
                    "ABC123".to_owned(),
                    t!("eco.class.join_code"),
                ))
                .child(
                    segmented()
                        .child(segment_owned(
                            t!("eco.class.follow_lock"),
                            move || state.classroom_follow.get() == FollowMode::LockedToTeacher,
                            move || state.classroom_follow.set(FollowMode::LockedToTeacher),
                        ))
                        .child(segment_owned(
                            t!("eco.class.follow_free"),
                            move || state.classroom_follow.get() == FollowMode::FreeBrowse,
                            move || state.classroom_follow.set(FollowMode::FreeBrowse),
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(pill_button(t!("eco.class.join_btn"), true, join_as_student))
                        .child(pill_button(
                            t!("eco.class.vote_btn"),
                            false,
                            student_vote_first,
                        )),
                )
                .into_element()
        }
    });

    let body = card(
        view()
            .w(px(580.0))
            .gap_3()
            .child(
                segmented()
                    .child(segment_owned(
                        t!("eco.class.tab_host"),
                        move || state.classroom_is_host.get(),
                        move || state.classroom_is_host.set(true),
                    ))
                    .child(segment_owned(
                        t!("eco.class.tab_student"),
                        move || !state.classroom_is_host.get(),
                        move || state.classroom_is_host.set(false),
                    )),
            )
            .child(role_view)
            .child(status_caption(move || {
                let _r = state.classroom_rev.get();
                let students = state
                    .classroom_host
                    .with(|opt| opt.as_ref().map_or(0, ClassroomHost::student_count));
                let hist = state.classroom_last_histogram.with(|h| {
                    h.as_ref().map_or_else(String::new, |r| {
                        let bars: Vec<String> = r
                            .bars
                            .iter()
                            .map(|b| format!("{}: {} ({:.0}%)", b.label, b.votes, b.percentage))
                            .collect();
                        format!(" | Poll ({}): {}", r.total_votes, bars.join(", "))
                    })
                });
                format!("Students: {students}{hist}")
            }))
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );
    list_dialog(t!("eco.class.title"), close, body)
}

fn plugins_dialog(state: EcosystemUiState) -> impl IntoElement {
    let close = move || state.close();

    let pick_plugin = move || {
        let dialog = OpenDialog::files()
            .title(t!("eco.plugins.pick"))
            .filter(FileFilter::new("ExcellentPDF Plugin", ["explugin"]));
        let answer = pick_paths(dialog);
        let key_hex = state.plugins_key_hex.get_untracked();
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            let vk = match parse_verifying_key_hex(&key_hex) {
                Ok(v) => v,
                Err(err) => {
                    state.status.set(err);
                    return;
                }
            };
            match verify_signed_plugin(&bytes, &vk) {
                Ok(pkg) => {
                    let caps = pkg.manifest.requested_capabilities.clone();
                    state.plugins_pending.with(|slot| {
                        *slot = Some(PendingPluginReview {
                            manifest: pkg.manifest.clone(),
                            archive_bytes: bytes,
                            verifying_key: vk,
                            approved_caps: caps,
                        });
                    });
                    state.plugins_rev.update(|r| *r = r.wrapping_add(1));
                    state.status.set(t!("eco.plugins.verified_pending"));
                }
                Err(err) => state.status.set(err.to_string()),
            }
        });
    };

    let create_sample_plugin = move || {
        let signing = SigningKey::from_bytes(&[0x5Au8; 32]);
        let payload = br#"{"name":"sample-helper"}"#;
        let mut requested = BTreeSet::new();
        requested.insert(PluginCapability::ReadDocs);
        let mut hooks = BTreeSet::new();
        hooks.insert(PluginHook::OnDocumentOpen);
        let manifest = PluginManifest {
            id: "sample.reader-helper".to_owned(),
            name: "Reader Helper".to_owned(),
            version: "1.0.0".to_owned(),
            entry_command: "reader-helper".to_owned(),
            requested_capabilities: requested.clone(),
            hooks,
            priority: 10,
            payload_sha256: String::new(),
        };
        if let Ok(archive) = pack_signed_plugin(&manifest, payload, &signing) {
            state.plugins_pending.with(|slot| {
                *slot = Some(PendingPluginReview {
                    manifest,
                    archive_bytes: archive,
                    verifying_key: signing.verifying_key(),
                    approved_caps: requested,
                });
            });
            state.plugins_rev.update(|r| *r = r.wrapping_add(1));
            state.status.set(t!("eco.plugins.verified_pending"));
        }
    };

    let approve_and_install = move || {
        let pending = state.plugins_pending.with(Option::take);
        let Some(p) = pending else {
            return;
        };
        let res = state.plugins_registry.with(|reg| {
            reg.install_signed(&p.archive_bytes, &p.verifying_key, p.approved_caps)
                .map_err(|e| e.to_string())
        });
        match res {
            Ok(m) => {
                let id = m.id.clone();
                state.plugins_ids.with(|ids| {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                });
                state.status.set(t!("eco.plugins.installed", name = m.name));
            }
            Err(err) => state.status.set(err),
        }
        state.plugins_rev.update(|r| *r = r.wrapping_add(1));
    };

    let body = card(
        view()
            .w(px(600.0))
            .max_h(px(540.0))
            .gap_3()
            .child(labeled_field(
                move || state.plugins_key_hex.get(),
                move |v| state.plugins_key_hex.set(v),
                "64 hex Ed25519 verifying key".to_owned(),
                t!("eco.plugins.key_label"),
            ))
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(pill_button(t!("eco.plugins.pick_btn"), true, pick_plugin))
                    .child(pill_button(
                        t!("eco.plugins.stage_demo"),
                        false,
                        create_sample_plugin,
                    )),
            )
            .child(dynamic(move || {
                let _rev = state.plugins_rev.get();
                let pending = state.plugins_pending.with(|p| {
                    p.as_ref().map(|x| {
                        let caps: Vec<String> = x
                            .manifest
                            .requested_capabilities
                            .iter()
                            .map(|c| c.as_str().to_owned())
                            .collect();
                        (
                            x.manifest.name.clone(),
                            x.manifest.version.clone(),
                            caps.join(", "),
                        )
                    })
                });
                let mut col = view().gap_2();
                if let Some((name, ver, caps)) = pending {
                    col = col.child(
                        view()
                            .p_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || theme().border)
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(status_caption(move || format!("{name} v{ver} [{caps}]")))
                            .child(pill_button(
                                t!("eco.plugins.approve_install"),
                                true,
                                approve_and_install,
                            )),
                    );
                }
                let reg_summary = state.plugins_registry.with(|reg| {
                    let hook_subs = reg.subscribers_for(PluginHook::OnDocumentOpen);
                    t!(
                        "eco.plugins.count",
                        total = reg.len(),
                        hooks = hook_subs.len()
                    )
                });
                col = col.child(status_caption(move || reg_summary.clone()));
                let ids = state.plugins_ids.with(|v| v.clone());
                for pid in ids {
                    let info = state.plugins_registry.with(|reg| {
                        reg.plugin(&pid).map(|pl| {
                            let enabled = matches!(
                                pl.lifecycle,
                                excellent_features::plugins::PluginLifecycle::Enabled
                            );
                            let caps = reg.sandbox().grants_for(&pid).len();
                            (
                                pl.manifest.name.clone(),
                                pl.manifest.version.clone(),
                                enabled,
                                caps,
                            )
                        })
                    });
                    let Some((pname, pver, enabled, caps)) = info else {
                        continue;
                    };
                    let id_toggle = pid.clone();
                    let id_uninstall = pid.clone();
                    let toggle_label = if enabled {
                        t!("eco.plugins.disable")
                    } else {
                        t!("eco.plugins.enable")
                    };
                    col = col.child(
                        view()
                            .p_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || theme().border)
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(status_caption(move || {
                                format!("{pname} v{pver} | active={enabled} | grants={caps}")
                            }))
                            .child(
                                view()
                                    .flex_row()
                                    .gap_2()
                                    .child(pill_button(toggle_label, false, move || {
                                        state.plugins_registry.with(|reg| {
                                            if enabled {
                                                let _ = reg.disable(&id_toggle);
                                            } else {
                                                let _ = reg.enable(&id_toggle);
                                            }
                                        });
                                        state.plugins_rev.update(|r| *r = r.wrapping_add(1));
                                    }))
                                    .child(pill_button(
                                        t!("eco.plugins.remove"),
                                        false,
                                        move || {
                                            state.plugins_registry.with(|reg| {
                                                let _ = reg.uninstall(&id_uninstall);
                                            });
                                            state
                                                .plugins_ids
                                                .with(|v| v.retain(|x| x != &id_uninstall));
                                            state.plugins_rev.update(|r| *r = r.wrapping_add(1));
                                        },
                                    )),
                            ),
                    );
                }
                col
            }))
            .child(when(
                move || !state.status.get().is_empty(),
                move || status_caption(move || state.status.get()),
            ))
            .child(view().flex_row().justify_end().child(pill_button(
                t!("action.close"),
                false,
                close,
            ))),
    );
    list_dialog(t!("eco.plugins.title"), close, body)
}
