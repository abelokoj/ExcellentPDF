//! The command table (plan K.16): every action the app has, written once. The command palette's items and the
//! keyboard shortcuts are both built from it, so no command is in one and missing from the other. The keys a command
//! runs from are its defaults here with the user's own over them ([`crate::keymap`]).
//!
//! A command has keys for the whole window and keys for the pages: plain keys such as the arrows, Space and single
//! letters belong to whatever has the keyboard focus, so they reach the pages only while the pages have it (Enhanced's
//! rule for its canvas). A command that needs a document is left out of the palette while none is open.

use ecclesia_features::modes::Mode;
use ecclesia_features::outline::{Restyle, Shift, Swatch};
use nsh_base::CommandItem;
use nsh_core::prelude::*;

use ecclesia_features::split::Follow;
use ecclesia_ink::erase::Eraser;
use ecclesia_ink::pens::PenKind;

use crate::bookmarks::BookmarkAction;
use crate::find::FindAction;
use crate::keymap::{self, Keymap, Scope};
use crate::outline::OutlineAction;
use crate::panes::{PaneAction, Side};
use crate::tabs::{Action as TabAction, Step};
use crate::workspaces::{Start, Transfer, WorkspaceAction};

/// What a command does; [`crate::shell`] carries it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandId {
    OpenPalette,
    /// Opens the settings window (K.16).
    OpenSettings,
    ChooseTheme,
    ChooseAppearance,
    ToggleSidebar,
    ToggleInspector,
    /// Opens the go-to box (R.6).
    GoToPage,
    /// Full screen on or off (R.19); F11 does it too, done by the platform itself.
    ToggleFullScreen,
    /// The reading ruler tints its band, or dims the page around it (R.20).
    ToggleRulerInvert,
    /// The pages in the theme's colours, or in their own (R.14).
    ToggleDocumentColors,
    /// Opens the reading-theme bar (R.15).
    ChooseReadingTheme,
    /// Opens files in new tabs (R.12).
    OpenFile,
    /// The tab list (R.12): every tab, searchable.
    TabList,
    /// Offers the app for its document types and opens the system's default-apps settings (R.11).
    SetDefaultViewer,
    /// Something done to the shown tab (R.12).
    Tab(TabAction),
    /// Something done to the split view's panes (V.1 to V.6).
    Pane(PaneAction),
    /// Something done to the workspaces (W.1 to W.5).
    Workspace(WorkspaceAction),
    /// Escape: out of full screen, focus mode, the reading ruler and auto-scroll (R.19).
    LeaveModes,
    /// A page-canvas command, by its name in the canvas's protocol (`zoomIn`, `layoutBook`, `rotateClockwise`).
    Canvas(&'static str),
    /// Something done to the shown document's outline (B.1 to B.5).
    Outline(OutlineAction),
    /// Something done to the shown document's personal bookmarks (B.6).
    Bookmark(BookmarkAction),
    /// Find in the shown document (F.1).
    Find(FindAction),
    /// Opens the markup tools' colour dialog (A.3).
    AnnotationColors,
    /// Shows the notes in a column beside the pages, or hides it (A.11).
    MarginNotes,
    /// Shows the list of the document's annotations (A.14).
    AnnotationList,
    /// Adds another version's annotations to the shown document (A.16).
    TransferAnnotations,
    /// Writes the shown PDF's annotations to an XFDF or FDF file (A.24).
    ExportAnnotations,
    /// Merges an XFDF or FDF file's annotations into the shown PDF (A.24).
    ImportAnnotations,
    /// Saves a copy of the shown PDF with its annotations drawn into the pages (A.25).
    ExportFlattened,
    /// Shows the document's layers, at the top of the annotation list (A.22).
    Layers,
    /// Puts every annotation on no layer on its author's (A.22).
    LayersByAuthor,
    /// Places a sticky note where the page is clicked next (A.5).
    StickyNote,
    /// Places a text box where the page is clicked next (A.6).
    TextBox,
    /// Places typewriter text where the page is clicked next (A.6).
    Typewriter,
    /// Places a callout pointing at where the page is clicked next (A.7).
    Callout,
    /// Draws a line by the next drag on the pages (A.8).
    Line,
    /// Draws an arrow by the next drag on the pages (A.8).
    Arrow,
    /// Draws a rectangle by the next drag on the pages (A.9).
    Rectangle,
    /// Draws an ellipse by the next drag on the pages (A.9).
    Ellipse,
    /// Draws a regular polygon by the next drag on the pages (A.10).
    RegularPolygon,
    /// Draws a star by the next drag on the pages (A.10).
    Star,
    /// Places the stamp chosen in the inspector where the page is clicked next (A.20).
    Stamp,
    /// Draws a polygon by the next clicks on the pages, corner by corner (A.10).
    Polygon,
    /// Draws a polyline by the next clicks on the pages (A.10).
    PolyLine,
    /// Draws a cloud by the next clicks on the pages (A.10).
    Cloud,
    /// The pen ink draws with from now on (H.2).
    Pen(PenKind),
    /// The eraser the pen uses from now on, instead of drawing (H.5).
    Eraser(Eraser),
    /// Turns the lasso on or off: a drag on the pages draws a loop that selects what it encloses (H.6).
    Lasso,
    /// Toggles rectangular block selection mode: drag to select and highlight arbitrary regions.
    BlockSelect,
    /// Lays tape: the next drag on a page covers that strip; a click on tape reveals it or hides it again (H.4).
    Tape,
    /// Turns the zoom writing pad on or off: write large at the bottom, it lands small on the page (H.12).
    ZoomPad,
    /// Lays a ruler on the page, or puts it away: strokes along it are drawn straight (H.11).
    Ruler,
    /// Lays a protractor on the page, or puts it away: strokes out from its centre land on a whole degree (H.11).
    Protractor,
    /// Lays a compass on the page, or puts it away: strokes round it are drawn as exact arcs (H.11).
    Compass,
    /// Turns the selected annotations a quarter turn anticlockwise (H.6).
    RotateLeft,
    /// Turns the selected annotations a quarter turn clockwise (H.6).
    RotateRight,
    /// Makes the selected annotations' lines thinner (H.6).
    Thinner,
    /// Makes the selected annotations' lines thicker (H.6).
    Thicker,
    /// Duplicates the selected annotations (H.6).
    DuplicateSelection,
    /// Turns snapshot mode on: a drag on a page renders that region, to copy or save (O.23).
    Snapshot,
    /// Lists the PDF's attachments, to save, open or add to (O.25).
    Attachments,
    /// Saves the pen in hand again as a new favourite and opens it to edit (H.7).
    SavePen,
    /// Opens the pen in hand to edit its kind, colour, width and pressure (H.7).
    EditPen,
    /// Removes the pen in hand from the favourites bar (H.7).
    RemovePen,
    /// Saves the favourites bar to a JSON file (H.7).
    ExportPens,
    /// Replaces the favourites bar with the pens of a JSON file (H.7).
    ImportPens,
    /// Copies the selected annotations and deletes them (A.13).
    CutAnnotations,
    /// Pastes the copied annotations onto the page shown (A.13).
    PasteAnnotations,
    /// Deletes the selected annotations (A.13).
    DeleteAnnotations,
    /// Takes back the shown document's last edit (K.14).
    Undo,
    /// Makes the last undone edit again (K.14).
    Redo,
    /// Saves the edited documents now (K.15); they are saved on their own too.
    Save,
    /// Prints the shown document (O.22).
    Print,
    /// Shows the first-run guide again (U.9).
    ReplayGuide,
    /// Switches to a mode and its toolbar (K.18).
    Mode(Mode),
    /// Switches to the next offered mode, or with `true` the previous one (K.18).
    StepMode(bool),
    /// Opens the toolbar's customize dialog (K.18).
    CustomizeToolbar,
    /// Saves every mode's toolbar as a preset file (K.18).
    ExportToolbars,
    /// Reads toolbars from a preset file (K.18).
    ImportToolbars,
    /// Straightens handwritten ink strokes onto level baselines (H.13).
    StraightenInk,
    /// Opens bulk ink restyle dialog (H.15).
    RestyleInk,
    /// Opens fill & gradient dialog for closed strokes/shapes (H.16).
    InkFill,
    /// Toggles the page color eyedropper sampler (H.17).
    Eyedropper,
    /// Opens saved color palettes dialog (H.18).
    ColorPalettes,
    /// Opens stroke replay transport dialog (H.22).
    InkReplay,
    /// Converts recognized shape strokes to vector PDF annotations (H.26).
    VectorizeInk,
    /// Creates a new templated `.expdf` notebook (N.1, N.4).
    NewNotebook,
    /// Inserts a blank or templated page before/after current page (N.2).
    InsertPage,
    /// Expands or restores margins on the current page (N.3).
    ExpandMargins,
    /// Exports `.expdf` notebook to standalone PDF (N.4).
    ExportNotebookPdf,
    /// Opens infinite canvas whiteboard dialog (N.5).
    InfiniteCanvas,
    /// Inserts a typed text block with IME support (N.6, U.8).
    InsertTextBlock,
    /// Inserts an image or sticker as PDF stamp art (N.7).
    InsertImage,
    /// Opens notebook covers and colored sections organizer (N.8).
    NotebookSections,
    /// Opens internal page & deep links manager (N.9).
    NotebookLinks,
    /// Opens a fresh quick-note page in `Inbox.expdf` (N.10).
    QuickNote,
    /// Opens Markdown notes editor and live preview (N.14).
    MarkdownNotes,
    /// Opens lecture audio recording & timeline sync dialog (L.1).
    LectureAudio,
    /// Imports slide PDF and condenses 1/2/3 slides per note page (L.3).
    CondenseSlides,
    /// Toggles laser pointer overlay in Present mode (G.2).
    PresentLaser,
    /// Toggles spotlight overlay in Present mode (G.3).
    PresentSpotlight,
    /// Toggles magnifier lens overlay in Present mode (G.4).
    PresentMagnifier,
    /// Toggles solid black blank screen in Present mode (G.1).
    PresentBlankBlack,
    /// Toggles solid white blank screen in Present mode (G.1).
    PresentBlankWhite,
    /// Switches between temporary and fading slide ink in Present mode (G.5).
    PresentFadingInk,
    /// Opens temporary whiteboard page in Present mode (G.6).
    PresentWhiteboard,
    /// Toggles curtain reveal overlay in Present mode (G.7).
    PresentCurtain,
    /// Opens secondary Presenter View window (G.8).
    PresenterView,
    /// Creates a flashcard from selection/note with source anchor (S.1).
    MakeFlashcard,
    /// Creates image occlusion cards (S.2).
    ImageOcclusion,
    /// Creates cloze deletion cards (S.3).
    MakeCloze,
    /// Starts an FSRS-5 daily review session (S.4, S.5).
    DailyReview,
    /// Opens offline dictionary lookup (S.6).
    Dictionary,
    /// Opens vocabulary library and practice activities (S.7).
    Vocabulary,
    /// Exports study notes in 7 formats (S.8).
    StudyExport,
    /// Toggles Exam Mode (S.10).
    ExamMode,
    /// Imports an Anki `.apkg` archive (S.11).
    ImportAnki,
    /// Exports flashcards to Anki `.apkg` (S.11).
    ExportAnki,
    /// Opens study goals, focus timer, and statistics dashboard (S.12).
    StudyStatistics,
    /// Toggles Read Aloud TTS bar (U.5).
    ReadAloud,
    /// Opens Library, FTS5 global search, saved searches, collections & metadata dialog (F.2–F.7, F.9, W.10).
    OpenLibrary,
    /// Scans the library for exact & near-duplicate documents (F.8).
    FindDuplicates,
    /// Opens Citations (BibTeX/RIS/CSL-JSON) & Zotero sync dialog (F.10, F.11).
    Citations,
    /// Opens Markdown Notes with `[[backlinks]]` & Obsidian export (F.12, F.13).
    LibraryNotes,
    /// Opens Workspace Templates & Path Auto-Routing Rules dialog (W.12).
    WorkspaceRules,
    /// Opens CLI, MCP server & UI Control Channel docs dialog (E.1–E.4).
    CliMcpDocs,
    /// Toggles Reflow (Liquid Mode) reading view (R.16).
    ReflowMode,
    /// Toggles Bionic Reading fixation emphasis (R.22).
    BionicReading,
    /// Advances along detected two-column reading layout (R.24).
    TwoColumnScroll,
    /// Opens SyncTeX forward & inverse search dialog (R.27).
    SyncTex,
    /// Opens extra container viewer (DjVu, CHM, CBZ, CBR) (R.29).
    ExtraFormats,
    /// Opens a PDF/document from a remote HTTP(S) URL with progressive loading (R.30).
    OpenUrl,
    /// Generates document outline from headings & numbering rules with preview (B.8).
    GenerateOutline,
    /// Imports or exports outline in Indented Text, JSON, pdftk, or Markdown (B.11, B.13).
    OutlineIo,
    /// Exports an annotation summary report (Markdown, HTML, CSV, PDF) (A.26).
    AnnotationReport,
    /// Opens Annotation History Timeline scrubber & point-in-time restore (A.27).
    AnnotationHistory,
    /// Beautifies selected handwriting strokes reversibly (H.19).
    BeautifyInk,
    /// Opens Pattern Brush picker & preview dialog (H.20).
    PatternBrush,
    /// Opens Symmetry axis & mirror strokes dialog (H.21).
    Symmetry,
    /// Searches handwritten ink via offline FTS5 handwriting index (H.23).
    HandwritingSearch,
    /// Converts lassoed handwriting ink to FreeText or text (H.24).
    HandwritingToText,
    /// Runs OCR to create a searchable PDF (O.4).
    OcrDocument,
    /// Generates a hyperlinked yearly/monthly/weekly/daily planner `.expdf` (N.11).
    Planner,
    /// Opens downloadable/hot-reloaded Template Packs catalog (N.12).
    TemplatePacks,
    /// Opens per-notebook `.expdf` Version History snapshots & restore (N.13).
    NotebookVersions,
    /// Opens Offline Audio Transcription & SRT/VTT search dialog (L.2).
    AudioTranscript,
    /// Builds offline highlight & note summaries grouped by heading (S.13).
    HighlightSummary,
    /// Opens Assignments & due-date checklist tracker (S.14).
    Assignments,
    /// Opens Language Learning word lookup, pronunciation & parallel pane (S.15).
    LanguageLearning,
    /// Opens Translate Selection popover with 73 languages & Bergamot packs (S.16).
    TranslateSelection,
    /// Opens AI Assistant panel with cited RAG answers & scope/locality badge (I.1–I.3).
    AiPanel,
    /// Summarizes the document or current chapter with citations (I.4–I.7).
    AiSummarize,
    /// Generates review-gated flashcards, glossary & study guide (I.8).
    AiStudyMaterials,
    /// Extracts structured claims, methods, findings, limitations & evidence tables (I.9–I.11).
    AiResearchExtract,
    /// Generates and runs an interactive chapter quiz (S.9).
    AiQuiz,
    /// Converts handwritten math strokes to LaTeX with confirmation preview (H.25).
    MathToLatex,
    /// Consent-gated Google Lens visual search for a page or region (R.31).
    VisualSearch,
    /// Opens interactive AcroForm filling, validation & autofill profiles (O.11, O.12).
    FormsPanel,
    /// Flattens form fields into a non-editable copy (O.13).
    FormsFlatten,
    /// Opens digital signatures verification, signed-version viewer & signing wizard (O.14–O.16, O.20).
    SignaturesPanel,
    /// Opens Page Organizer, drag reorder, split, merge, crop & page labels (O.1–O.3, R.18).
    PageOrganizer,
    /// Opens text run editor, embedded image replacer, true redaction & export/convert (O.5–O.7, O.9, O.10).
    EditTextAndImages,
    /// Opens True Redaction marking & permanent apply dialog (O.7).
    TrueRedaction,
    /// Opens Compress & PDF Security/Encryption/Sanitization dialog (O.8, O.19).
    CompressPdf,
    /// Opens Batch Images<->PDF and Markdown/DOCX export dialog (O.9, O.10).
    ExportAndConvert,
    /// Opens Bates numbering, headers/footers & watermarks dialog (O.17, O.18).
    BatesAndHeaders,
    /// Opens permissions, encryption & PDF sanitization dialog (O.19).
    SecurityPermissions,
    /// Opens print production preflight, output intents & ink separations (O.24).
    PrintProduction,
    /// Toggles CAD engineering drawing view, dark-blueprint theme & OCG layers (R.26).
    CadView,
    /// Opens extended outline actions inspector/editor (B.12).
    OutlineActions,
    /// Opens calibrated vector measurement tools & scale setup (A.23).
    MeasureTool,
    /// Opens side-by-side PDF comparison & diff report export (F.15).
    ComparePdfs,
    /// Opens structured table & figure extraction (F.16).
    ExtractTablesFigures,
    /// Opens screen & pen synchronized video recording (L.4).
    ScreencastRecord,
    /// Opens grading stamps, rubric, comment bank, batch submissions & worksheet maker (G.9–G.13).
    GradingSuite,
    /// Imports & perspective-rectifies camera scans into searchable PDFs (O.21).
    CameraScanImport,
    /// Opens the Infinite Canvas Excerpt Board, groups & evidence chains (X.1–X.4).
    ExcerptBoard,
    /// Opens the Library Knowledge Graph with cluster filter (F.14).
    KnowledgeGraph,
    /// Opens CRDT Folder/Workspace Sync, Portable Settings & `.exbundle` sharing (Y.1–Y.3, W.13).
    SyncAndShare,
    /// Opens Live Encrypted Co-Editing Relay session dialog (Y.4).
    LiveRelay,
    /// Opens Live Classroom Broadcast & Quick Polls host/student panel (G.14, G.15).
    LiveClassroom,
    /// Opens Sandboxed Plugins & capability permissions manager (E.5).
    PluginsManager,
}

impl CommandId {
    /// Whether it does anything now: page commands need a document.
    pub(crate) fn is_available(self, has_document: bool) -> bool {
        has_document
            || !matches!(
                self,
                Self::Canvas(_)
                    | Self::GoToPage
                    | Self::Outline(_)
                    | Self::Bookmark(_)
                    | Self::Find(_)
                    | Self::StickyNote
                    | Self::TextBox
                    | Self::Typewriter
                    | Self::Callout
                    | Self::Line
                    | Self::Arrow
                    | Self::Rectangle
                    | Self::Ellipse
                    | Self::RegularPolygon
                    | Self::Star
                    | Self::Stamp
                    | Self::Polygon
                    | Self::PolyLine
                    | Self::Cloud
                    | Self::BlockSelect
                    | Self::CutAnnotations
                    | Self::TransferAnnotations
                    | Self::ExportAnnotations
                    | Self::ImportAnnotations
                    | Self::ExportFlattened
                    | Self::LayersByAuthor
                    | Self::PasteAnnotations
                    | Self::DeleteAnnotations
                    | Self::StraightenInk
                    | Self::RestyleInk
                    | Self::InkFill
                    | Self::Eyedropper
                    | Self::InkReplay
                    | Self::VectorizeInk
                    | Self::InsertPage
                    | Self::ExpandMargins
                    | Self::ExportNotebookPdf
                    | Self::InsertTextBlock
                    | Self::InsertImage
                    | Self::NotebookSections
                    | Self::NotebookLinks
                    | Self::LectureAudio
                    | Self::PresentLaser
                    | Self::PresentSpotlight
                    | Self::PresentMagnifier
                    | Self::PresentBlankBlack
                    | Self::PresentBlankWhite
                    | Self::PresentFadingInk
                    | Self::PresentWhiteboard
                    | Self::PresentCurtain
                    | Self::PresenterView
                    | Self::MakeFlashcard
                    | Self::ImageOcclusion
                    | Self::MakeCloze
                    | Self::StudyExport
                    | Self::ExamMode
                    | Self::ReadAloud
                    | Self::ReflowMode
                    | Self::BionicReading
                    | Self::TwoColumnScroll
                    | Self::SyncTex
                    | Self::GenerateOutline
                    | Self::OutlineIo
                    | Self::AnnotationReport
                    | Self::AnnotationHistory
                    | Self::BeautifyInk
                    | Self::PatternBrush
                    | Self::Symmetry
                    | Self::HandwritingSearch
                    | Self::HandwritingToText
                    | Self::OcrDocument
                    | Self::NotebookVersions
                    | Self::AudioTranscript
                    | Self::HighlightSummary
                    | Self::Undo
                    | Self::Redo
                    | Self::Save
                    | Self::Print
            ) && !matches!(self, Self::Tab(action) if action != TabAction::ReopenClosed)
                && !matches!(
                    self,
                    Self::Pane(
                        PaneAction::Split(_) | PaneAction::MoveTab(_) | PaneAction::TileSelected
                    ) | Self::Workspace(
                        WorkspaceAction::New(Start::AllTabs | Start::ShownTab)
                            | WorkspaceAction::SendTab(_)
                    )
                )
    }
}

/// One row of the table.
#[derive(Debug)]
pub(crate) struct Command {
    pub(crate) id: CommandId,
    /// Its stable name: the palette's item id, and later its key in `settings.toml`.
    pub(crate) name: &'static str,
    /// The localization key of its label.
    pub(crate) label: &'static str,
    /// The localization key of its palette section.
    group: &'static str,
    /// Its default shortcuts anywhere in the window, as NSH's keymap writes them; the first is the one hints show.
    pub(crate) keys: &'static [&'static str],
    /// Its default shortcuts while the pages have the keyboard focus; hints show the first when `keys` is empty.
    pub(crate) page_keys: &'static [&'static str],
}

/// Every command. Defaults follow SumatraPDF Enhanced where it has the command (`sumatra_pdf_enhanced_plan.md`).
pub(crate) const COMMANDS: &[Command] = &[
    Command {
        id: CommandId::OpenPalette,
        name: "palette.open",
        label: "command.open_palette",
        group: "command.group.general",
        // Enhanced's CmdCommandPalette.
        keys: &["mod+k", "mod+shift+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenFile,
        name: "file.open",
        label: "command.open_file",
        group: "command.group.file",
        // Enhanced's CmdOpenFile.
        keys: &["mod+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Save,
        name: "file.save",
        label: "command.save",
        group: "command.group.file",
        // Enhanced's CmdSave.
        keys: &["mod+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Print,
        name: "file.print",
        label: "command.print",
        group: "command.group.file",
        // Enhanced's CmdPrint.
        keys: &["mod+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::SetDefaultViewer,
        name: "app.set_default_viewer",
        label: "command.set_default_viewer",
        group: "command.group.file",
        // Enhanced's "Make default PDF reader" button (its options dialog).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Close),
        name: "tab.close",
        label: "command.close_tab",
        group: "command.group.tabs",
        // Enhanced's CmdClose.
        keys: &["mod+w", "mod+f4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::ReopenClosed),
        name: "tab.reopen_closed",
        label: "command.reopen_closed_tab",
        group: "command.group.tabs",
        // Enhanced's CmdReopenLastClosedFile.
        keys: &["mod+shift+t"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Cycle(Step::Next)),
        name: "tab.next",
        label: "command.next_tab",
        group: "command.group.tabs",
        // Enhanced's CmdNextTab.
        keys: &["ctrl+tab", "ctrl+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Cycle(Step::Previous)),
        name: "tab.previous",
        label: "command.previous_tab",
        group: "command.group.tabs",
        // Enhanced's CmdPrevTab.
        keys: &["ctrl+shift+tab", "ctrl+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::TabList,
        name: "tab.list",
        label: "command.tab_list",
        group: "command.group.tabs",
        // Vivaldi's and Edge's tab search.
        keys: &["mod+shift+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Move(Step::Previous)),
        name: "tab.move_left",
        label: "command.move_tab_left",
        group: "command.group.tabs",
        // Enhanced's CmdMoveTabLeft.
        keys: &["ctrl+shift+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::Move(Step::Next)),
        name: "tab.move_right",
        label: "command.move_tab_right",
        group: "command.group.tabs",
        // Enhanced's CmdMoveTabRight.
        keys: &["ctrl+shift+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::TogglePin),
        name: "tab.pin",
        label: "command.pin_tab",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseOthers),
        name: "tab.close_others",
        label: "command.close_other_tabs",
        group: "command.group.tabs",
        // Enhanced's CmdCloseOtherTabs.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseToTheRight),
        name: "tab.close_right",
        label: "command.close_tabs_right",
        group: "command.group.tabs",
        // Enhanced's CmdCloseTabsToTheRight.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseToTheLeft),
        name: "tab.close_left",
        label: "command.close_tabs_left",
        group: "command.group.tabs",
        // Enhanced's CmdCloseTabsToTheLeft.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseAll),
        name: "tab.close_all",
        label: "command.close_all_tabs",
        group: "command.group.tabs",
        // Enhanced's CmdCloseAllTabs.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::CloseDuplicates),
        name: "tab.close_duplicates",
        label: "command.close_duplicate_tabs",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::NewGroup),
        name: "tab.group.new",
        label: "command.new_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::EditGroup),
        name: "tab.group.edit",
        label: "command.edit_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::LeaveGroup),
        name: "tab.group.leave",
        label: "command.leave_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Tab(TabAction::ToggleGroup),
        name: "tab.group.toggle",
        label: "command.toggle_tab_group",
        group: "command.group.tabs",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenSettings,
        name: "app.settings",
        label: "command.settings",
        group: "command.group.general",
        // The common desktop key for an app's settings (Enhanced opens its file from a menu).
        keys: &["mod+,"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ReplayGuide,
        name: "help.guide",
        label: "command.replay_guide",
        group: "command.group.general",
        // Enhanced's CmdHelpOpenManual (F1) and CmdToggleKeyboardHelp (?).
        keys: &["f1"],
        page_keys: &["shift+/"],
    },
    Command {
        id: CommandId::ChooseTheme,
        name: "view.theme",
        label: "command.choose_theme",
        group: "command.group.view",
        // No default shortcut (Enhanced has none); K.16 lets the user set one.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ChooseAppearance,
        name: "view.appearance",
        label: "command.choose_appearance",
        group: "command.group.view",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleSidebar,
        name: "view.sidebar",
        label: "command.toggle_sidebar",
        group: "command.group.view",
        // Enhanced's CmdToggleBookmarks: its sidebar.
        keys: &["f12"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleInspector,
        name: "view.inspector",
        label: "command.toggle_inspector",
        group: "command.group.view",
        // Enhanced has no inspector; VS Code's secondary side bar.
        keys: &["mod+alt+b"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Undo,
        name: "edit.undo",
        label: "command.undo",
        group: "command.group.edit",
        // Enhanced's undo. A text field that has the focus keeps its own.
        keys: &["mod+z"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Redo,
        name: "edit.redo",
        label: "command.redo",
        group: "command.group.edit",
        // Enhanced's CmdRedo (Ctrl+Shift+Z), plus standard Ctrl+Y alias.
        keys: &["mod+shift+z", "mod+y"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("highlight"),
        name: "markup.highlight",
        label: "command.highlight",
        group: "command.group.annotate",
        keys: &[],
        // Enhanced's CmdCreateAnnotHighlight is "a".
        page_keys: &["a"],
    },
    Command {
        id: CommandId::BlockSelect,
        name: "annotate.block_select",
        label: "command.block_select",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["mod+shift+b"],
    },
    Command {
        id: CommandId::Canvas("blockHighlight"),
        name: "markup.block_highlight",
        label: "command.block_highlight",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("underline"),
        name: "markup.underline",
        label: "command.underline",
        group: "command.group.annotate",
        keys: &[],
        // Enhanced's CmdCreateAnnotUnderline is "u".
        page_keys: &["u"],
    },
    Command {
        id: CommandId::Canvas("squiggly"),
        name: "markup.squiggly",
        label: "command.squiggly",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+u", "q"],
    },
    Command {
        id: CommandId::Canvas("strikeOut"),
        name: "markup.strikeout",
        label: "command.strikeout",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["x", "s"],
    },
    Command {
        id: CommandId::Canvas("replaceText"),
        name: "markup.replace",
        label: "command.replace_text",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["e"],
    },
    Command {
        id: CommandId::Canvas("insertText"),
        name: "markup.insert",
        label: "command.insert_text",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["insert"],
    },
    Command {
        id: CommandId::Canvas("noteSelection"),
        name: "annotate.note",
        label: "command.note",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+n"],
    },
    Command {
        id: CommandId::StickyNote,
        name: "annotate.sticky",
        label: "command.sticky_note",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+s"],
    },
    Command {
        id: CommandId::TextBox,
        name: "annotate.text_box",
        label: "command.text_box",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["t"],
    },
    Command {
        id: CommandId::Typewriter,
        name: "annotate.typewriter",
        label: "command.typewriter",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+t"],
    },
    Command {
        id: CommandId::Callout,
        name: "annotate.callout",
        label: "command.callout",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["o"],
    },
    Command {
        id: CommandId::Line,
        name: "annotate.line",
        label: "command.line",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+l"],
    },
    Command {
        id: CommandId::Arrow,
        name: "annotate.arrow",
        label: "command.arrow",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+a"],
    },
    Command {
        id: CommandId::Rectangle,
        name: "annotate.rectangle",
        label: "command.rectangle",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["r"],
    },
    Command {
        id: CommandId::Ellipse,
        name: "annotate.ellipse",
        label: "command.ellipse",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+e"],
    },
    Command {
        id: CommandId::Polygon,
        name: "annotate.polygon",
        label: "command.polygon",
        group: "command.group.annotate",
        // Plain "p" belongs to previous page (Enhanced's CmdGoToPrevPage).
        keys: &[],
        page_keys: &["alt+g"],
    },
    Command {
        id: CommandId::PolyLine,
        name: "annotate.polyline",
        label: "command.polyline",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+p"],
    },
    Command {
        id: CommandId::Cloud,
        name: "annotate.cloud",
        label: "command.cloud",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+k"],
    },
    Command {
        id: CommandId::RegularPolygon,
        name: "annotate.regular_polygon",
        label: "command.regular_polygon",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+r"],
    },
    Command {
        id: CommandId::Star,
        name: "annotate.star",
        label: "command.star",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+x"],
    },
    Command {
        id: CommandId::Stamp,
        name: "annotate.stamp",
        label: "command.stamp",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+m"],
    },
    Command {
        id: CommandId::Pen(PenKind::Fountain),
        name: "ink.pen_fountain",
        label: "command.pen_fountain",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["1"],
    },
    Command {
        id: CommandId::Pen(PenKind::Ballpoint),
        name: "ink.pen_ballpoint",
        label: "command.pen_ballpoint",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["2"],
    },
    Command {
        id: CommandId::Pen(PenKind::Pencil),
        name: "ink.pen_pencil",
        label: "command.pen_pencil",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["3"],
    },
    Command {
        id: CommandId::Pen(PenKind::Brush),
        name: "ink.pen_brush",
        label: "command.pen_brush",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["4"],
    },
    Command {
        id: CommandId::Pen(PenKind::Calligraphy),
        name: "ink.pen_calligraphy",
        label: "command.pen_calligraphy",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["5"],
    },
    Command {
        id: CommandId::Pen(PenKind::Marker),
        name: "ink.pen_marker",
        label: "command.pen_marker",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["6"],
    },
    Command {
        id: CommandId::Pen(PenKind::Highlighter),
        name: "ink.pen_highlighter",
        label: "command.pen_highlighter",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["7"],
    },
    Command {
        id: CommandId::Eraser(Eraser::Stroke),
        name: "ink.eraser_stroke",
        label: "command.eraser_stroke",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["8"],
    },
    Command {
        id: CommandId::Eraser(Eraser::Area),
        name: "ink.eraser_area",
        label: "command.eraser_area",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["9"],
    },
    Command {
        id: CommandId::Eraser(Eraser::Highlighter),
        name: "ink.eraser_highlighter",
        label: "command.eraser_highlighter",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["0"],
    },
    Command {
        id: CommandId::ZoomPad,
        name: "ink.zoom_pad",
        label: "command.zoom_pad",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["alt+z"],
    },
    Command {
        id: CommandId::Ruler,
        name: "ink.ruler",
        label: "command.ruler",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["alt+r"],
    },
    Command {
        id: CommandId::Protractor,
        name: "ink.protractor",
        label: "command.protractor",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["alt+p"],
    },
    Command {
        id: CommandId::Compass,
        name: "ink.compass",
        label: "command.compass",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["alt+c"],
    },
    Command {
        id: CommandId::Tape,
        name: "ink.tape",
        label: "command.tape",
        group: "command.group.ink",
        keys: &[],
        page_keys: &["alt+t"],
    },
    Command {
        id: CommandId::Lasso,
        name: "ink.lasso",
        label: "command.lasso",
        group: "command.group.ink",
        // V, the selection tool's key in drawing apps.
        keys: &[],
        page_keys: &["v"],
    },
    Command {
        id: CommandId::RotateLeft,
        name: "select.rotate_left",
        label: "command.rotate_left",
        group: "command.group.annotate",
        keys: &["mod+shift+["],
        page_keys: &[],
    },
    Command {
        id: CommandId::RotateRight,
        name: "select.rotate_right",
        label: "command.rotate_right",
        group: "command.group.annotate",
        keys: &["mod+shift+]"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Thinner,
        name: "select.thinner",
        label: "command.thinner",
        group: "command.group.annotate",
        keys: &["alt+["],
        page_keys: &[],
    },
    Command {
        id: CommandId::Thicker,
        name: "select.thicker",
        label: "command.thicker",
        group: "command.group.annotate",
        keys: &["alt+]"],
        page_keys: &[],
    },
    Command {
        id: CommandId::SavePen,
        name: "ink.save_pen",
        label: "command.save_pen",
        group: "command.group.ink",
        keys: &["mod+alt+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::EditPen,
        name: "ink.edit_pen",
        label: "command.edit_pen",
        group: "command.group.ink",
        keys: &["mod+alt+k"],
        page_keys: &[],
    },
    Command {
        id: CommandId::RemovePen,
        name: "ink.remove_pen",
        label: "command.remove_pen",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportPens,
        name: "ink.export_pens",
        label: "command.export_pens",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImportPens,
        name: "ink.import_pens",
        label: "command.import_pens",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Attachments,
        name: "document.attachments",
        label: "command.attachments",
        group: "command.group.read",
        keys: &["mod+alt+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Snapshot,
        name: "tools.snapshot",
        label: "command.snapshot",
        group: "command.group.read",
        keys: &["alt+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::DuplicateSelection,
        name: "select.duplicate",
        label: "command.duplicate_selection",
        group: "command.group.annotate",
        keys: &["mod+shift+d"],
        page_keys: &[],
    },
    Command {
        id: CommandId::CutAnnotations,
        name: "annotate.cut",
        label: "command.cut_annotations",
        group: "command.group.annotate",
        // Copy (mod+c) copies the selected annotations instead of text while some are selected.
        keys: &["mod+x"],
        page_keys: &[],
    },
    Command {
        id: CommandId::PasteAnnotations,
        name: "annotate.paste",
        label: "command.paste_annotations",
        group: "command.group.annotate",
        keys: &["mod+v"],
        page_keys: &[],
    },
    Command {
        id: CommandId::DeleteAnnotations,
        name: "annotate.delete",
        label: "command.delete_annotations",
        group: "command.group.annotate",
        // Enhanced's Delete (canvas) and Ctrl+Delete (CmdDeleteAnnotation).
        keys: &[],
        page_keys: &["delete", "mod+delete"],
    },
    Command {
        id: CommandId::AnnotationColors,
        name: "annotate.colors",
        label: "command.annotation_colors",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+c"],
    },
    Command {
        id: CommandId::TransferAnnotations,
        name: "annotate.transfer",
        label: "command.transfer_annotations",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+v"],
    },
    Command {
        id: CommandId::ExportAnnotations,
        name: "annotate.export",
        label: "command.export_annotations",
        group: "command.group.annotate",
        keys: &["mod+alt+e"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImportAnnotations,
        name: "annotate.import",
        label: "command.import_annotations",
        group: "command.group.annotate",
        keys: &["mod+alt+i"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportFlattened,
        name: "annotate.flatten",
        label: "command.export_flattened",
        group: "command.group.annotate",
        keys: &["mod+shift+e"],
        page_keys: &[],
    },
    Command {
        id: CommandId::AnnotationList,
        name: "annotate.list",
        label: "command.annotation_list",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["shift+o"],
    },
    Command {
        id: CommandId::Layers,
        name: "annotate.layers",
        label: "command.layers",
        group: "command.group.annotate",
        keys: &["mod+alt+l"],
        page_keys: &[],
    },
    Command {
        id: CommandId::LayersByAuthor,
        name: "annotate.layers_by_author",
        label: "command.layers_by_author",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::MarginNotes,
        name: "annotate.margin_notes",
        label: "command.margin_notes",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["m"],
    },
    Command {
        id: CommandId::Canvas("copy"),
        name: "edit.copy",
        label: "command.copy",
        group: "command.group.edit",
        // Enhanced's CmdCopySelection. A text field that has the focus keeps its own copy.
        keys: &["mod+c", "mod+insert"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("selectAll"),
        name: "edit.select_all",
        label: "command.select_all",
        group: "command.group.edit",
        // Enhanced's CmdSelectAll, for the page shown until the text of the whole document is indexed.
        keys: &["mod+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("zoomIn"),
        name: "view.zoom_in",
        label: "command.zoom_in",
        group: "command.group.zoom",
        // Enhanced's CmdZoomIn: Ctrl+= / Ctrl++ anywhere, and = / + on the pages.
        keys: &["mod+plus"],
        page_keys: &["plus", "shift+plus"],
    },
    Command {
        id: CommandId::Canvas("zoomOut"),
        name: "view.zoom_out",
        label: "command.zoom_out",
        group: "command.group.zoom",
        // Enhanced's CmdZoomOut: Ctrl+- anywhere, and - on the pages.
        keys: &["mod+minus"],
        page_keys: &["minus"],
    },
    Command {
        id: CommandId::Canvas("actualSize"),
        name: "view.actual_size",
        label: "command.actual_size",
        group: "command.group.zoom",
        // Enhanced's CmdZoomActualSize.
        keys: &["mod+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("fitWidth"),
        name: "view.fit_width",
        label: "command.fit_width",
        group: "command.group.zoom",
        // Enhanced's CmdZoomFitWidth.
        keys: &["mod+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("fitPage"),
        name: "view.fit_page",
        label: "command.fit_page",
        group: "command.group.zoom",
        // Enhanced's CmdZoomFitPage (Ctrl+0) and CmdToggleZoom (z).
        keys: &["mod+0"],
        page_keys: &["z"],
    },
    Command {
        id: CommandId::Canvas("fitHeight"),
        name: "view.fit_height",
        label: "command.fit_height",
        group: "command.group.zoom",
        // Pairs with Enhanced's Ctrl+0 / Ctrl+1 / Ctrl+2 / Ctrl+3 fit chords.
        keys: &["mod+3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutSingle"),
        name: "view.layout_single",
        label: "command.layout_single",
        group: "command.group.layout",
        // Enhanced's CmdSinglePageView.
        keys: &["mod+6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutFacing"),
        name: "view.layout_facing",
        label: "command.layout_facing",
        group: "command.group.layout",
        // Enhanced's CmdFacingView.
        keys: &["mod+7"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutBook"),
        name: "view.layout_book",
        label: "command.layout_book",
        group: "command.group.layout",
        // Enhanced's CmdBookView.
        keys: &["mod+8"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutHorizontal"),
        name: "view.layout_horizontal",
        label: "command.layout_horizontal",
        group: "command.group.layout",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("layoutGrid"),
        name: "view.layout_grid",
        label: "command.layout_grid",
        group: "command.group.layout",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("toggleContinuous"),
        name: "view.toggle_continuous",
        label: "command.toggle_continuous",
        group: "command.group.layout",
        // Enhanced's `c`.
        keys: &[],
        page_keys: &["c"],
    },
    Command {
        id: CommandId::Canvas("toggleDirection"),
        name: "view.toggle_direction",
        label: "command.toggle_direction",
        group: "command.group.layout",
        // Enhanced's manga mode has no key.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("rotateClockwise"),
        name: "view.rotate_clockwise",
        label: "command.rotate_clockwise",
        group: "command.group.page",
        // Enhanced's CmdRotateRight.
        keys: &["mod+shift+plus"],
        page_keys: &["]"],
    },
    Command {
        id: CommandId::Canvas("rotateAnticlockwise"),
        name: "view.rotate_anticlockwise",
        label: "command.rotate_anticlockwise",
        group: "command.group.page",
        // Enhanced's CmdRotateLeft.
        keys: &["mod+shift+minus"],
        page_keys: &["["],
    },
    Command {
        id: CommandId::Canvas("boxMedia"),
        name: "view.box_media",
        label: "command.box_media",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxCrop"),
        name: "view.box_crop",
        label: "command.box_crop",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxBleed"),
        name: "view.box_bleed",
        label: "command.box_bleed",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxTrim"),
        name: "view.box_trim",
        label: "command.box_trim",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("boxArt"),
        name: "view.box_art",
        label: "command.box_art",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::GoToPage,
        name: "go.to_page",
        label: "command.go_to_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToPage.
        keys: &["mod+g"],
        page_keys: &["g"],
    },
    Command {
        id: CommandId::Canvas("firstPage"),
        name: "go.first_page",
        label: "command.first_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToFirstPage.
        keys: &["mod+home"],
        page_keys: &["home"],
    },
    Command {
        id: CommandId::Canvas("lastPage"),
        name: "go.last_page",
        label: "command.last_page",
        group: "command.group.navigate",
        // Enhanced's CmdGoToLastPage.
        keys: &["mod+end"],
        page_keys: &["end"],
    },
    Command {
        id: CommandId::Canvas("historyBack"),
        name: "go.back",
        label: "command.go_back",
        group: "command.group.navigate",
        // Enhanced's CmdNavigateBack.
        keys: &["alt+left"],
        page_keys: &["backspace"],
    },
    Command {
        id: CommandId::Canvas("historyForward"),
        name: "go.forward",
        label: "command.go_forward",
        group: "command.group.navigate",
        // Enhanced's CmdNavigateForward.
        keys: &["alt+right"],
        page_keys: &["shift+backspace"],
    },
    Command {
        id: CommandId::Canvas("screenDown"),
        name: "go.screen_down",
        label: "command.screen_down",
        group: "command.group.navigate",
        // Enhanced's CmdScrollDownPage (PageDown, Enter, Ctrl+Down), CmdGoToNextPage (n), CmdScrollDownHalfPage (Shift+Down).
        keys: &["pagedown"],
        page_keys: &["enter", "n", "mod+down", "shift+down"],
    },
    Command {
        id: CommandId::Canvas("screenUp"),
        name: "go.screen_up",
        label: "command.screen_up",
        group: "command.group.navigate",
        // Enhanced's CmdScrollUpPage (PageUp, Shift+Space, Shift+Enter, Ctrl+Up), CmdGoToPrevPage (p), CmdScrollUpHalfPage (Shift+Up).
        keys: &["pageup"],
        page_keys: &["shift+space", "shift+enter", "p", "mod+up", "shift+up"],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Add),
        name: "outline.add",
        label: "command.outline_add",
        group: "command.group.outline",
        // Enhanced's CmdAddBookmark.
        keys: &["mod+b"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Rename),
        name: "outline.rename",
        label: "command.outline_rename",
        group: "command.group.outline",
        // F2 while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::SetDestination),
        name: "outline.set_destination",
        label: "command.outline_set_destination",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Bold)),
        name: "outline.bold",
        label: "command.outline_bold",
        group: "command.group.outline",
        // Ctrl+Shift+B while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Italic)),
        name: "outline.italic",
        label: "command.outline_italic",
        group: "command.group.outline",
        // Ctrl+Shift+I while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(None))),
        name: "outline.color_none",
        label: "command.outline_color_none",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Red)))),
        name: "outline.color_red",
        label: "command.outline_color_red",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Orange)))),
        name: "outline.color_orange",
        label: "command.outline_color_orange",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Green)))),
        name: "outline.color_green",
        label: "command.outline_color_green",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Blue)))),
        name: "outline.color_blue",
        label: "command.outline_color_blue",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Style(Restyle::Color(Some(Swatch::Purple)))),
        name: "outline.color_purple",
        label: "command.outline_color_purple",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Up)),
        name: "outline.move_up",
        label: "command.outline_move_up",
        group: "command.group.outline",
        // Alt+Up while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Down)),
        name: "outline.move_down",
        label: "command.outline_move_down",
        group: "command.group.outline",
        // Alt+Down while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::In)),
        name: "outline.indent",
        label: "command.outline_indent",
        group: "command.group.outline",
        // Tab while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Move(Shift::Out)),
        name: "outline.outdent",
        label: "command.outline_outdent",
        group: "command.group.outline",
        // Shift+Tab while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Sort),
        name: "outline.sort",
        label: "command.outline_sort",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::OpenElsewhere),
        name: "outline.open_in_other_pane",
        label: "command.outline_open_elsewhere",
        group: "command.group.outline",
        // Ctrl+Enter while the outline has the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::ToBookmark),
        name: "outline.to_bookmark",
        label: "command.outline_to_bookmark",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Check),
        name: "outline.check",
        label: "command.outline_check",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Fix),
        name: "outline.fix",
        label: "command.outline_fix",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Outline(OutlineAction::Delete),
        name: "outline.delete",
        label: "command.outline_delete",
        group: "command.group.outline",
        // Delete while the outline has the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Add),
        name: "bookmark.add",
        label: "command.bookmark_add",
        group: "command.group.bookmarks",
        // Plan B.6; Enhanced has no key for CmdFavoriteAdd.
        keys: &["mod+d"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Edit),
        name: "bookmark.edit",
        label: "command.bookmark_edit",
        group: "command.group.bookmarks",
        // F2 while the bookmarks have the keyboard (the panel binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::ToOutline),
        name: "bookmark.to_outline",
        label: "command.bookmark_to_outline",
        group: "command.group.bookmarks",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Bookmark(BookmarkAction::Delete),
        name: "bookmark.delete",
        label: "command.bookmark_delete",
        group: "command.group.bookmarks",
        // Delete while the bookmarks have the keyboard.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Open),
        name: "find.open",
        label: "command.find_open",
        group: "command.group.find",
        // Enhanced's CmdFindFirst (Ctrl+F) and '/' on the pages.
        keys: &["mod+f"],
        page_keys: &["/"],
    },
    Command {
        id: CommandId::Find(FindAction::Next),
        name: "find.next",
        label: "command.find_next",
        group: "command.group.find",
        // Enhanced's CmdFindNext (F3) and CmdFindNextSel (Ctrl+F3); anywhere, so it works while the find bar has the keyboard.
        keys: &["f3", "mod+f3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Previous),
        name: "find.previous",
        label: "command.find_previous",
        group: "command.group.find",
        // Enhanced's CmdFindPrev (Shift+F3) and CmdFindPrevSel (Ctrl+Shift+F3).
        keys: &["shift+f3", "mod+shift+f3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Find(FindAction::Close),
        name: "find.close",
        label: "command.find_close",
        group: "command.group.find",
        // Escape while the find bar has the keyboard (the bar binds it).
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleFullScreen,
        name: "view.full_screen",
        label: "command.full_screen",
        group: "command.group.read",
        // Enhanced's CmdToggleFullscreen; its F11 the platform takes itself.
        keys: &["mod+shift+l"],
        page_keys: &["f"],
    },
    Command {
        id: CommandId::Canvas("toggleFocus"),
        name: "view.focus_mode",
        label: "command.focus_mode",
        group: "command.group.read",
        // Enhanced has no focus mode.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("toggleRuler"),
        name: "view.reading_ruler",
        label: "command.reading_ruler",
        group: "command.group.read",
        // Enhanced's reading bar has no key.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleRulerInvert,
        name: "view.reading_ruler_invert",
        label: "command.reading_ruler_invert",
        group: "command.group.read",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ToggleDocumentColors,
        name: "view.document_colors",
        label: "command.document_colors",
        group: "command.group.read",
        // Enhanced's Shift+I inverts the colours for the session; this one keeps the choice.
        keys: &[],
        page_keys: &["shift+i"],
    },
    Command {
        id: CommandId::ChooseReadingTheme,
        name: "view.reading_theme",
        label: "command.reading_theme",
        group: "command.group.read",
        // No default shortcut (Enhanced has no reading themes); K.16 lets the user set one.
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("lineDown"),
        name: "go.line_down",
        label: "command.line_down",
        group: "command.group.read",
        // Enhanced's Down: a line down, or the reading bar's next line.
        keys: &[],
        page_keys: &["down", "j"],
    },
    Command {
        id: CommandId::Canvas("lineUp"),
        name: "go.line_up",
        label: "command.line_up",
        group: "command.group.read",
        keys: &[],
        page_keys: &["up", "k"],
    },
    Command {
        id: CommandId::Canvas("stepLeft"),
        name: "go.left",
        label: "command.step_left",
        group: "command.group.navigate",
        // Enhanced's Left, H, and Shift+Left (CmdScrollLeftPage); the row before or after follows the reading direction (R.3).
        keys: &[],
        page_keys: &["left", "h", "shift+left"],
    },
    Command {
        id: CommandId::Canvas("stepRight"),
        name: "go.right",
        label: "command.step_right",
        group: "command.group.navigate",
        // Enhanced's Right, L, and Shift+Right (CmdScrollRightPage).
        keys: &[],
        page_keys: &["right", "l", "shift+right"],
    },
    Command {
        id: CommandId::Canvas("toggleAutoScroll"),
        name: "view.auto_scroll",
        label: "command.auto_scroll",
        group: "command.group.read",
        // Enhanced's auto-scroll toggle.
        keys: &["mod+shift+h"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Canvas("autoScrollPause"),
        name: "view.auto_scroll_pause",
        label: "command.auto_scroll_pause",
        group: "command.group.read",
        // Enhanced's Space: pauses auto-scroll, else a screen down.
        keys: &[],
        page_keys: &["space"],
    },
    Command {
        id: CommandId::Canvas("autoScrollFaster"),
        name: "view.auto_scroll_faster",
        label: "command.auto_scroll_faster",
        group: "command.group.read",
        // Plain + / = and - zoom on the pages (Enhanced's FrameOnChar); Alt+= / Alt++ and Alt+- step auto-scroll speed.
        keys: &[],
        page_keys: &["alt+plus", "alt+shift+plus"],
    },
    Command {
        id: CommandId::Canvas("autoScrollSlower"),
        name: "view.auto_scroll_slower",
        label: "command.auto_scroll_slower",
        group: "command.group.read",
        keys: &[],
        page_keys: &["alt+minus"],
    },
    Command {
        id: CommandId::LeaveModes,
        name: "view.leave_modes",
        label: "command.leave_modes",
        group: "command.group.read",
        // Enhanced's Escape leaves full screen.
        keys: &[],
        page_keys: &["escape"],
    },
    Command {
        id: CommandId::Pane(PaneAction::Split(Side::Right)),
        name: "pane.split_right",
        label: "command.split_right",
        group: "command.group.panes",
        // VS Code's split key.
        keys: &["mod+\\"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Split(Side::Down)),
        name: "pane.split_down",
        label: "command.split_down",
        group: "command.group.panes",
        keys: &["mod+alt+\\"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::OpenInSplit),
        name: "pane.open_in_split",
        label: "command.open_in_split",
        group: "command.group.panes",
        keys: &["mod+shift+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::MoveTab(Step::Next)),
        name: "pane.move_tab_next",
        label: "command.move_tab_next_pane",
        group: "command.group.panes",
        keys: &["mod+alt+right"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::MoveTab(Step::Previous)),
        name: "pane.move_tab_previous",
        label: "command.move_tab_previous_pane",
        group: "command.group.panes",
        keys: &["mod+alt+left"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(0)),
        name: "pane.focus_1",
        label: "command.focus_pane_1",
        group: "command.group.panes",
        // The plan's Ctrl+1 to 4 are the tabs' (R.12); Alt+1 to 4 waits for the maintainer's word (dev_decisions).
        keys: &["alt+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(1)),
        name: "pane.focus_2",
        label: "command.focus_pane_2",
        group: "command.group.panes",
        keys: &["alt+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(2)),
        name: "pane.focus_3",
        label: "command.focus_pane_3",
        group: "command.group.panes",
        keys: &["alt+3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Focus(3)),
        name: "pane.focus_4",
        label: "command.focus_pane_4",
        group: "command.group.panes",
        keys: &["alt+4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Cycle(Step::Next)),
        name: "pane.next",
        label: "command.next_pane",
        group: "command.group.panes",
        // The common key for the next pane.
        keys: &["f6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Cycle(Step::Previous)),
        name: "pane.previous",
        label: "command.previous_pane",
        group: "command.group.panes",
        keys: &["shift+f6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Close),
        name: "pane.close",
        label: "command.close_pane",
        group: "command.group.panes",
        keys: &["mod+alt+w"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Sync),
        name: "pane.sync",
        label: "command.sync_scroll",
        group: "command.group.panes",
        keys: &["mod+alt+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::SamePage)),
        name: "pane.sync_page",
        label: "command.sync_same_page",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::SamePosition)),
        name: "pane.sync_position",
        label: "command.sync_same_position",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncFollow(Follow::Pairs)),
        name: "pane.sync_pairs",
        label: "command.sync_pairs",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::SyncZoom),
        name: "pane.sync_zoom",
        label: "command.sync_zoom",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::PairPages),
        name: "pane.pair_pages",
        label: "command.pair_pages",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::ClearPagePairs),
        name: "pane.clear_page_pairs",
        label: "command.clear_page_pairs",
        group: "command.group.panes",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::PopOut),
        name: "pane.pop_out",
        label: "command.pop_out",
        group: "command.group.panes",
        keys: &["mod+alt+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Swap),
        name: "pane.swap",
        label: "command.swap_panes",
        group: "command.group.panes",
        keys: &["mod+alt+x"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Flip),
        name: "pane.flip",
        label: "command.flip_panes",
        group: "command.group.panes",
        keys: &["mod+alt+f"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Maximize),
        name: "pane.maximize",
        label: "command.maximize_pane",
        group: "command.group.panes",
        keys: &["mod+alt+enter"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Equalize),
        name: "pane.equalize",
        label: "command.equalize_panes",
        group: "command.group.panes",
        keys: &["mod+alt+0"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::TileSelected),
        name: "pane.tile_selected",
        label: "command.tile_selected",
        group: "command.group.panes",
        keys: &["mod+alt+g"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Pane(PaneAction::Untile),
        name: "pane.untile",
        label: "command.untile",
        group: "command.group.panes",
        keys: &["mod+alt+shift+g"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Switcher),
        name: "workspace.switch",
        label: "command.switch_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::Empty)),
        name: "workspace.new",
        label: "command.new_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+n"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::AllTabs)),
        name: "workspace.new_from_tabs",
        label: "command.new_workspace_from_tabs",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::New(Start::ShownTab)),
        name: "workspace.new_from_tab",
        label: "command.new_workspace_from_tab",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Move)),
        name: "workspace.move_tab",
        label: "command.move_tab_to_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Copy)),
        name: "workspace.copy_tab",
        label: "command.copy_tab_to_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Edit),
        name: "workspace.edit",
        label: "command.edit_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::OpenWindow),
        name: "workspace.open_window",
        label: "command.open_workspace_window",
        group: "command.group.workspaces",
        keys: &["mod+alt+shift+n"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Export),
        name: "workspace.export",
        label: "command.export_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Import),
        name: "workspace.import",
        label: "command.import_workspace",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Shift(Step::Previous)),
        name: "workspace.move_earlier",
        label: "command.move_workspace_earlier",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Shift(Step::Next)),
        name: "workspace.move_later",
        label: "command.move_workspace_later",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(0)),
        name: "workspace.1",
        label: "command.workspace_1",
        group: "command.group.workspaces",
        // The plan's keys (W.5).
        keys: &["mod+alt+1"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(1)),
        name: "workspace.2",
        label: "command.workspace_2",
        group: "command.group.workspaces",
        keys: &["mod+alt+2"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(2)),
        name: "workspace.3",
        label: "command.workspace_3",
        group: "command.group.workspaces",
        keys: &["mod+alt+3"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(3)),
        name: "workspace.4",
        label: "command.workspace_4",
        group: "command.group.workspaces",
        keys: &["mod+alt+4"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(4)),
        name: "workspace.5",
        label: "command.workspace_5",
        group: "command.group.workspaces",
        keys: &["mod+alt+5"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(5)),
        name: "workspace.6",
        label: "command.workspace_6",
        group: "command.group.workspaces",
        keys: &["mod+alt+6"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(6)),
        name: "workspace.7",
        label: "command.workspace_7",
        group: "command.group.workspaces",
        keys: &["mod+alt+7"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(7)),
        name: "workspace.8",
        label: "command.workspace_8",
        group: "command.group.workspaces",
        keys: &["mod+alt+8"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Go(8)),
        name: "workspace.9",
        label: "command.workspace_9",
        group: "command.group.workspaces",
        keys: &["mod+alt+9"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Cycle(Step::Next)),
        name: "workspace.next",
        label: "command.next_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+pagedown"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Workspace(WorkspaceAction::Cycle(Step::Previous)),
        name: "workspace.previous",
        label: "command.previous_workspace",
        group: "command.group.workspaces",
        keys: &["mod+alt+pageup"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Read),
        name: "mode.read",
        label: "command.mode.read",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Annotate),
        name: "mode.annotate",
        label: "command.mode.annotate",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Write),
        name: "mode.write",
        label: "command.mode.write",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Study),
        name: "mode.study",
        label: "command.mode.study",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Mode(Mode::Present),
        name: "mode.present",
        label: "command.mode.present",
        group: "command.group.mode",
        // Enhanced's CmdTogglePresentationMode (F5, Ctrl+L, Shift+F11).
        keys: &["f5", "mod+l", "shift+f11"],
        page_keys: &[],
    },
    Command {
        id: CommandId::StepMode(false),
        name: "mode.next",
        label: "command.mode.next",
        group: "command.group.mode",
        // Enhanced has no modes; Ctrl+M is the window's minimize on macOS, so Shift joins it.
        keys: &["mod+shift+m"],
        page_keys: &[],
    },
    Command {
        id: CommandId::StepMode(true),
        name: "mode.previous",
        label: "command.mode.previous",
        group: "command.group.mode",
        keys: &["mod+alt+shift+m"],
        page_keys: &[],
    },
    Command {
        id: CommandId::CustomizeToolbar,
        name: "toolbar.customize",
        label: "command.toolbar.customize",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportToolbars,
        name: "toolbar.export",
        label: "command.toolbar.export",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImportToolbars,
        name: "toolbar.import",
        label: "command.toolbar.import",
        group: "command.group.mode",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::StraightenInk,
        name: "ink.straighten",
        label: "command.straighten_ink",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+s"],
    },
    Command {
        id: CommandId::RestyleInk,
        name: "ink.restyle",
        label: "command.restyle_ink",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+r"],
    },
    Command {
        id: CommandId::InkFill,
        name: "ink.fill",
        label: "command.ink_fill",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+f"],
    },
    Command {
        id: CommandId::Eyedropper,
        name: "ink.eyedropper",
        label: "command.eyedropper",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["i"],
    },
    Command {
        id: CommandId::ColorPalettes,
        name: "ink.palettes",
        label: "command.color_palettes",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+p"],
    },
    Command {
        id: CommandId::InkReplay,
        name: "ink.replay",
        label: "command.ink_replay",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+y"],
    },
    Command {
        id: CommandId::VectorizeInk,
        name: "ink.vectorize",
        label: "command.vectorize_ink",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &["alt+shift+v"],
    },
    Command {
        id: CommandId::NewNotebook,
        name: "notebook.new",
        label: "command.new_notebook",
        group: "command.group.notebook",
        keys: &["mod+n"],
        page_keys: &[],
    },
    Command {
        id: CommandId::InsertPage,
        name: "notebook.insert_page",
        label: "command.insert_page",
        group: "command.group.notebook",
        keys: &["mod+shift+i"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExpandMargins,
        name: "notebook.expand_margins",
        label: "command.expand_margins",
        group: "command.group.notebook",
        keys: &["mod+alt+m"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportNotebookPdf,
        name: "notebook.export_pdf",
        label: "command.export_notebook_pdf",
        group: "command.group.notebook",
        keys: &["mod+alt+shift+e"],
        page_keys: &[],
    },
    Command {
        id: CommandId::InfiniteCanvas,
        name: "notebook.infinite",
        label: "command.infinite_canvas",
        group: "command.group.notebook",
        keys: &["mod+alt+c"],
        page_keys: &[],
    },
    Command {
        id: CommandId::InsertTextBlock,
        name: "notebook.text_block",
        label: "command.insert_text_block",
        group: "command.group.notebook",
        keys: &[],
        page_keys: &["alt+b"],
    },
    Command {
        id: CommandId::InsertImage,
        name: "notebook.insert_image",
        label: "command.insert_image",
        group: "command.group.notebook",
        keys: &[],
        page_keys: &["alt+i"],
    },
    Command {
        id: CommandId::NotebookSections,
        name: "notebook.sections",
        label: "command.notebook_sections",
        group: "command.group.notebook",
        keys: &["mod+alt+t"],
        page_keys: &[],
    },
    Command {
        id: CommandId::NotebookLinks,
        name: "notebook.links",
        label: "command.notebook_links",
        group: "command.group.notebook",
        keys: &["mod+alt+u"],
        page_keys: &[],
    },
    Command {
        id: CommandId::QuickNote,
        name: "notebook.quick_note",
        label: "command.quick_note",
        group: "command.group.notebook",
        keys: &["mod+shift+q"],
        page_keys: &[],
    },
    Command {
        id: CommandId::MarkdownNotes,
        name: "notebook.markdown",
        label: "command.markdown_notes",
        group: "command.group.notebook",
        keys: &["mod+shift+k"],
        page_keys: &[],
    },
    Command {
        id: CommandId::LectureAudio,
        name: "notebook.audio",
        label: "command.lecture_audio",
        group: "command.group.notebook",
        keys: &["mod+alt+r"],
        page_keys: &[],
    },
    Command {
        id: CommandId::CondenseSlides,
        name: "notebook.condense_slides",
        label: "command.condense_slides",
        group: "command.group.notebook",
        keys: &["mod+alt+d"],
        page_keys: &[],
    },
    Command {
        id: CommandId::PresentLaser,
        name: "present.laser",
        label: "command.present.laser",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+l"],
    },
    Command {
        id: CommandId::PresentSpotlight,
        name: "present.spotlight",
        label: "command.present.spotlight",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+o"],
    },
    Command {
        id: CommandId::PresentMagnifier,
        name: "present.magnifier",
        label: "command.present.magnifier",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+m"],
    },
    Command {
        id: CommandId::PresentBlankBlack,
        name: "present.blank_black",
        label: "command.present.blank_black",
        group: "command.group.present",
        // Enhanced's B and '.' (wireless presenter) -> CmdPresentationBlackBackground.
        keys: &[],
        page_keys: &["b", "."],
    },
    Command {
        id: CommandId::PresentBlankWhite,
        name: "present.blank_white",
        label: "command.present.blank_white",
        group: "command.group.present",
        keys: &[],
        page_keys: &["w"],
    },
    Command {
        id: CommandId::PresentFadingInk,
        name: "present.fading_ink",
        label: "command.present.fading_ink",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+f"],
    },
    Command {
        id: CommandId::PresentWhiteboard,
        name: "present.whiteboard",
        label: "command.whiteboard",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+w"],
    },
    Command {
        id: CommandId::PresentCurtain,
        name: "present.curtain",
        label: "command.present.curtain",
        group: "command.group.present",
        keys: &[],
        page_keys: &["alt+k"],
    },
    Command {
        id: CommandId::PresenterView,
        name: "present.presenter_view",
        label: "command.presenter_view",
        group: "command.group.present",
        keys: &["mod+shift+r"],
        page_keys: &[],
    },
    Command {
        id: CommandId::MakeFlashcard,
        name: "study.flashcard",
        label: "command.make_flashcard",
        group: "command.group.study",
        keys: &["mod+shift+c"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImageOcclusion,
        name: "study.occlusion",
        label: "command.image_occlusion",
        group: "command.group.study",
        keys: &["mod+alt+shift+o"],
        page_keys: &[],
    },
    Command {
        id: CommandId::MakeCloze,
        name: "study.cloze",
        label: "command.make_cloze",
        group: "command.group.study",
        keys: &["mod+alt+z"],
        page_keys: &[],
    },
    Command {
        id: CommandId::DailyReview,
        name: "study.daily_review",
        label: "command.daily_review",
        group: "command.group.study",
        keys: &["mod+shift+f"],
        page_keys: &[],
    },
    Command {
        id: CommandId::Dictionary,
        name: "study.dictionary",
        label: "command.dictionary",
        group: "command.group.study",
        // Enhanced's Shift+D (CmdDictionaryLookup), plus plain "d".
        keys: &[],
        page_keys: &["d", "shift+d"],
    },
    Command {
        id: CommandId::Vocabulary,
        name: "study.vocabulary",
        label: "command.vocabulary",
        group: "command.group.study",
        keys: &["mod+shift+v"],
        page_keys: &[],
    },
    Command {
        id: CommandId::StudyExport,
        name: "study.export",
        label: "command.study_export",
        group: "command.group.study",
        keys: &["mod+alt+shift+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExamMode,
        name: "study.exam_mode",
        label: "command.exam_mode",
        group: "command.group.study",
        keys: &["mod+shift+x"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ImportAnki,
        name: "study.import_anki",
        label: "command.import_anki",
        group: "command.group.study",
        keys: &["mod+alt+y"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportAnki,
        name: "study.export_anki",
        label: "command.export_anki",
        group: "command.group.study",
        keys: &["mod+alt+shift+y"],
        page_keys: &[],
    },
    Command {
        id: CommandId::StudyStatistics,
        name: "study.statistics",
        label: "command.study_statistics",
        group: "command.group.study",
        keys: &["mod+shift+g"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ReadAloud,
        name: "study.read_aloud",
        label: "command.read_aloud",
        group: "command.group.study",
        keys: &["mod+shift+s"],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenLibrary,
        name: "library.open",
        label: "command.open_library",
        group: "command.group.find",
        keys: &["mod+alt+shift+l"],
        page_keys: &[],
    },
    Command {
        id: CommandId::FindDuplicates,
        name: "library.duplicates",
        label: "command.find_duplicates",
        group: "command.group.find",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Citations,
        name: "library.citations",
        label: "command.citations",
        group: "command.group.find",
        keys: &["mod+alt+shift+c"],
        page_keys: &[],
    },
    Command {
        id: CommandId::LibraryNotes,
        name: "library.notes",
        label: "command.library_notes",
        group: "command.group.find",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::WorkspaceRules,
        name: "workspace.rules",
        label: "command.workspace_rules",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::CliMcpDocs,
        name: "help.cli_mcp",
        label: "command.cli_mcp_docs",
        group: "command.group.general",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ReflowMode,
        name: "view.reflow_mode",
        label: "command.reflow_mode",
        group: "command.group.read",
        keys: &["mod+alt+shift+r"],
        page_keys: &[],
    },
    Command {
        id: CommandId::BionicReading,
        name: "view.bionic_reading",
        label: "command.bionic_reading",
        group: "command.group.read",
        keys: &["mod+alt+shift+b"],
        page_keys: &[],
    },
    Command {
        id: CommandId::TwoColumnScroll,
        name: "go.two_column_scroll",
        label: "command.two_column_scroll",
        group: "command.group.navigate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::SyncTex,
        name: "tools.synctex",
        label: "command.synctex",
        group: "command.group.general",
        keys: &["mod+alt+shift+j"],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExtraFormats,
        name: "file.extra_formats",
        label: "command.extra_formats",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::OpenUrl,
        name: "file.open_url",
        label: "command.open_url",
        group: "command.group.file",
        keys: &["mod+alt+shift+u"],
        page_keys: &[],
    },
    Command {
        id: CommandId::GenerateOutline,
        name: "outline.generate",
        label: "command.generate_outline",
        group: "command.group.outline",
        keys: &["mod+alt+shift+k"],
        page_keys: &[],
    },
    Command {
        id: CommandId::OutlineIo,
        name: "outline.import_export",
        label: "command.outline_io",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AnnotationReport,
        name: "annotate.report",
        label: "command.annotation_report",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AnnotationHistory,
        name: "annotate.history",
        label: "command.annotation_history",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::BeautifyInk,
        name: "ink.beautify",
        label: "command.beautify_ink",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::PatternBrush,
        name: "ink.pattern_brush",
        label: "command.pattern_brush",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Symmetry,
        name: "ink.symmetry",
        label: "command.symmetry",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::HandwritingSearch,
        name: "ink.handwriting_search",
        label: "command.handwriting_search",
        group: "command.group.ink",
        keys: &["mod+alt+shift+h"],
        page_keys: &[],
    },
    Command {
        id: CommandId::HandwritingToText,
        name: "ink.handwriting_to_text",
        label: "command.handwriting_to_text",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::OcrDocument,
        name: "tools.ocr",
        label: "command.ocr_document",
        group: "command.group.general",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Planner,
        name: "notebook.planner",
        label: "command.planner",
        group: "command.group.notebook",
        keys: &["mod+alt+shift+p"],
        page_keys: &[],
    },
    Command {
        id: CommandId::TemplatePacks,
        name: "notebook.template_packs",
        label: "command.template_packs",
        group: "command.group.notebook",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::NotebookVersions,
        name: "notebook.versions",
        label: "command.notebook_versions",
        group: "command.group.notebook",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AudioTranscript,
        name: "notebook.audio_transcript",
        label: "command.audio_transcript",
        group: "command.group.notebook",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::HighlightSummary,
        name: "study.highlight_summary",
        label: "command.highlight_summary",
        group: "command.group.study",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::Assignments,
        name: "study.assignments",
        label: "command.assignments",
        group: "command.group.study",
        keys: &["mod+alt+shift+a"],
        page_keys: &[],
    },
    Command {
        id: CommandId::LanguageLearning,
        name: "study.language_learning",
        label: "command.language_learning",
        group: "command.group.study",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::TranslateSelection,
        name: "study.translate_selection",
        label: "command.translate_selection",
        group: "command.group.study",
        keys: &["mod+alt+shift+t"],
        page_keys: &[],
    },
    Command {
        id: CommandId::AiPanel,
        name: "ai.panel",
        label: "command.ai_panel",
        group: "command.group.ai",
        keys: &["mod+alt+q"],
        page_keys: &[],
    },
    Command {
        id: CommandId::AiSummarize,
        name: "ai.summarize",
        label: "command.ai_summarize",
        group: "command.group.ai",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AiStudyMaterials,
        name: "ai.study_materials",
        label: "command.ai_study_materials",
        group: "command.group.ai",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AiResearchExtract,
        name: "ai.extract",
        label: "command.ai_extract",
        group: "command.group.ai",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::AiQuiz,
        name: "ai.quiz",
        label: "command.ai_quiz",
        group: "command.group.ai",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::MathToLatex,
        name: "ink.math_to_latex",
        label: "command.math_to_latex",
        group: "command.group.ink",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::VisualSearch,
        name: "tools.visual_search",
        label: "command.visual_search",
        group: "command.group.ai",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::FormsPanel,
        name: "forms.fill",
        label: "command.forms_fill",
        group: "command.group.forms",
        keys: &["mod+alt+j"],
        page_keys: &[],
    },
    Command {
        id: CommandId::FormsFlatten,
        name: "forms.flatten",
        label: "command.forms_flatten",
        group: "command.group.forms",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::SignaturesPanel,
        name: "document.signatures",
        label: "command.signatures",
        group: "command.group.forms",
        keys: &["mod+alt+v"],
        page_keys: &[],
    },
    Command {
        id: CommandId::PageOrganizer,
        name: "document.page_organizer",
        label: "command.page_organizer",
        group: "command.group.page",
        keys: &["mod+shift+y"],
        page_keys: &[],
    },
    Command {
        id: CommandId::EditTextAndImages,
        name: "document.edit_content",
        label: "command.edit_content",
        group: "command.group.edit",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::TrueRedaction,
        name: "document.redact",
        label: "command.redact",
        group: "command.group.edit",
        keys: &["mod+shift+u"],
        page_keys: &[],
    },
    Command {
        id: CommandId::CompressPdf,
        name: "document.compress",
        label: "command.compress",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExportAndConvert,
        name: "file.export_convert",
        label: "command.export_convert",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::BatesAndHeaders,
        name: "document.bates_headers",
        label: "command.bates_headers",
        group: "command.group.page",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::SecurityPermissions,
        name: "document.security",
        label: "command.security_permissions",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::PrintProduction,
        name: "document.print_production",
        label: "command.print_production",
        group: "command.group.view",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::CadView,
        name: "view.cad_mode",
        label: "command.cad_view",
        group: "command.group.view",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::OutlineActions,
        name: "outline.actions",
        label: "command.outline_actions",
        group: "command.group.outline",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::MeasureTool,
        name: "annotate.measure",
        label: "command.measure_tool",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ComparePdfs,
        name: "document.compare",
        label: "command.compare_pdfs",
        group: "command.group.find",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExtractTablesFigures,
        name: "document.extract_tables",
        label: "command.extract_tables",
        group: "command.group.find",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ScreencastRecord,
        name: "tools.screencast",
        label: "command.screencast",
        group: "command.group.present",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::GradingSuite,
        name: "annotate.grading",
        label: "command.grading_suite",
        group: "command.group.annotate",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::CameraScanImport,
        name: "document.camera_scan",
        label: "command.camera_scan",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::ExcerptBoard,
        name: "study.excerpt_board",
        label: "command.excerpt_board",
        group: "command.group.study",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::KnowledgeGraph,
        name: "library.knowledge_graph",
        label: "command.knowledge_graph",
        group: "command.group.find",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::SyncAndShare,
        name: "workspace.sync_share",
        label: "command.sync_share",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::LiveRelay,
        name: "workspace.live_relay",
        label: "command.live_relay",
        group: "command.group.workspaces",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::LiveClassroom,
        name: "present.classroom",
        label: "command.classroom",
        group: "command.group.present",
        keys: &[],
        page_keys: &[],
    },
    Command {
        id: CommandId::PluginsManager,
        name: "tools.plugins",
        label: "command.plugins",
        group: "command.group.file",
        keys: &[],
        page_keys: &[],
    },
];

impl Command {
    /// Its row in the command palette, in the app's language, with the first of its keys.
    fn palette_item(&self, keymap: &Keymap) -> CommandItem {
        let item = CommandItem::new(self.name, t!(self.label)).group(t!(self.group));
        match keymap.first_keys(self.id) {
            Some(keys) => item.shortcut(keys.to_owned()),
            None => item,
        }
    }
}

/// The command named `name`, as the palette reports it and settings name it.
pub(crate) fn find(name: &str) -> Option<CommandId> {
    COMMANDS.iter().find(|c| c.name == name).map(|c| c.id)
}

/// The command palette's items: every command available now, in table order.
pub(crate) fn palette_items(has_document: bool) -> Vec<CommandItem> {
    let keymap = keymap::current();
    COMMANDS
        .iter()
        .filter(|c| c.id.is_available(has_document))
        .map(|c| c.palette_item(&keymap))
        .collect()
}

/// The label of `id`, in the app's language.
pub(crate) fn label(id: CommandId) -> String {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .map(|c| t!(c.label))
        .unwrap_or_default()
}

/// What screen readers say for a button that runs `id`: its label, and its first shortcut if it has one.
pub(crate) fn label_with_shortcut(id: CommandId) -> String {
    let label = label(id);
    match shortcut_label(id) {
        keys if keys.is_empty() => label,
        keys => t!("command.with_shortcut", label = label, keys = keys),
    }
}

/// The first shortcut of `id`, as hints show it on this platform (`Ctrl+K`, `⌘K`).
pub(crate) fn shortcut_label(id: CommandId) -> String {
    keymap::current()
        .first_keys(id)
        .map(key_label)
        .unwrap_or_default()
}

/// Which page-scoped shortcuts should currently be mounted in NSH's keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageKeyState {
    /// No page-scoped shortcuts are bound (Home, modal dialog, popover, or sidebar list owns focus).
    Inactive,
    /// Only [`CommandId::LeaveModes`] (`Escape`) is bound so full screen can be exited without a document.
    FullScreenEscapeOnly,
    /// All page-scoped shortcuts are bound for the active document canvas.
    Active,
}

/// An OS-level shortcut emitted by the Windows top-level window bridge (`"osShortcut"`).
///
/// Covers keys that NSH's Windows backend either reserves for the hidden host HWND (`F11`,
/// `Ctrl+Q`) or drops in `Key::from_virtual_key` (numpad `VK_ADD`, `VK_SUBTRACT`).
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OsShortcut {
    /// `F11`: toggle full screen on the visible window.
    ToggleFullScreen,
    /// `Ctrl+Q`: save open edits and quit the application.
    Quit,
    /// `Ctrl+Numpad+`: zoom in.
    ZoomIn,
    /// `Ctrl+Numpad-`: zoom out.
    ZoomOut,
    /// `Ctrl+Shift+Numpad+`: rotate clockwise.
    RotateClockwise,
    /// `Ctrl+Shift+Numpad-`: rotate anticlockwise.
    RotateAnticlockwise,
    /// `Alt+Numpad+`: speed up auto-scroll.
    AutoScrollFaster,
    /// `Alt+Numpad-`: slow down auto-scroll.
    AutoScrollSlower,
    /// Unmodified `Numpad+` while page keys are active: zoom in.
    PageZoomIn,
    /// Unmodified `Numpad-` while page keys are active: zoom out.
    PageZoomOut,
}

#[cfg(any(target_os = "windows", test))]
impl OsShortcut {
    /// Parses an `"osShortcut"` native event payload (`{"action": "..."}`).
    pub(crate) fn from_event(data: &nsh_core::JsonValue) -> Option<Self> {
        match data.get("action").and_then(nsh_core::JsonValue::as_str)? {
            "toggleFullScreen" => Some(Self::ToggleFullScreen),
            "quit" => Some(Self::Quit),
            "zoomIn" => Some(Self::ZoomIn),
            "zoomOut" => Some(Self::ZoomOut),
            "rotateClockwise" => Some(Self::RotateClockwise),
            "rotateAnticlockwise" => Some(Self::RotateAnticlockwise),
            "autoScrollFaster" => Some(Self::AutoScrollFaster),
            "autoScrollSlower" => Some(Self::AutoScrollSlower),
            "pageZoomIn" => Some(Self::PageZoomIn),
            "pageZoomOut" => Some(Self::PageZoomOut),
            _ => None,
        }
    }

    /// Returns the [`CommandId`] this OS shortcut dispatches, or `None` for application quit.
    pub(crate) fn command(self) -> Option<CommandId> {
        match self {
            Self::ToggleFullScreen => Some(CommandId::ToggleFullScreen),
            Self::Quit => None,
            Self::ZoomIn | Self::PageZoomIn => Some(CommandId::Canvas("zoomIn")),
            Self::ZoomOut | Self::PageZoomOut => Some(CommandId::Canvas("zoomOut")),
            Self::RotateClockwise => Some(CommandId::Canvas("rotateClockwise")),
            Self::RotateAnticlockwise => Some(CommandId::Canvas("rotateAnticlockwise")),
            Self::AutoScrollFaster => Some(CommandId::Canvas("autoScrollFaster")),
            Self::AutoScrollSlower => Some(CommandId::Canvas("autoScrollSlower")),
        }
    }

    /// Returns whether this shortcut only runs while page-scoped shortcuts are active.
    pub(crate) fn is_page_scoped(self) -> bool {
        matches!(self, Self::PageZoomIn | Self::PageZoomOut)
    }
}

/// Returns the NSH [`KeyOptions`] for a window-scoped shortcut.
///
/// Repeatable navigation, zoom, rotation, find, and tab/workspace-cycling commands keep
/// `repeat: true` when held. Window-level navigation and chrome commands whose keystrokes NSH
/// classifies as text-editing (`Tab`, `PageUp`/`PageDown`, `Alt+Left`/`Right`, `Enter`) or that
/// must stay responsive over focused chrome controls use `When::Always` and `over_focused: true`.
fn window_key_options(id: CommandId) -> KeyOptions {
    let repeat = matches!(
        id,
        CommandId::Canvas(
            "screenDown"
                | "screenUp"
                | "zoomIn"
                | "zoomOut"
                | "historyBack"
                | "historyForward"
                | "rotateClockwise"
                | "rotateAnticlockwise"
        ) | CommandId::Find(FindAction::Next | FindAction::Previous)
            | CommandId::Tab(TabAction::Cycle(_))
            | CommandId::Workspace(WorkspaceAction::Cycle(_))
    );
    let always = matches!(
        id,
        CommandId::Tab(TabAction::Cycle(_) | TabAction::Move(_))
            | CommandId::Workspace(WorkspaceAction::Cycle(_))
            | CommandId::Pane(PaneAction::MoveTab(_) | PaneAction::Maximize)
            | CommandId::Canvas("historyBack" | "historyForward")
            | CommandId::Find(FindAction::Next | FindAction::Previous)
            | CommandId::OpenPalette
            | CommandId::ToggleFullScreen
    );
    let when = if always {
        nsh_core::keymap::When::Always
    } else {
        nsh_core::keymap::When::Auto
    };
    let over_focused = matches!(
        id,
        CommandId::Tab(TabAction::Cycle(_) | TabAction::Move(_))
            | CommandId::Workspace(WorkspaceAction::Cycle(_))
            | CommandId::Canvas("historyBack" | "historyForward")
            | CommandId::Find(FindAction::Next | FindAction::Previous)
            | CommandId::OpenPalette
            | CommandId::ToggleFullScreen
    );
    KeyOptions {
        when,
        repeat,
        over_focused,
        ..KeyOptions::default()
    }
}

/// Returns the NSH [`KeyOptions`] for a page-scoped shortcut mounted while the document canvas
/// owns plain-key shortcuts.
///
/// Page bindings are mounted reactively only when no modal dialog, popover, or sidebar list owns
/// focus. Setting `pass_through: false`, `for_focus: true`, and `over_focused: true` ensures
/// `Space`, `Enter`, arrows, `Home`, `End`, and single letters are cleanly consumed by the PDF
/// even if a toolbar or tab button retained residual focus, while `When::Auto` still yields
/// automatically whenever a text input (such as the Find bar) has focus. `LeaveModes` (`Escape`)
/// uses `When::Always` so pressing Escape while typing in the Find bar closes it reliably.
fn page_key_options(id: CommandId) -> KeyOptions {
    let repeat = matches!(
        id,
        CommandId::Canvas(
            "lineDown"
                | "lineUp"
                | "stepLeft"
                | "stepRight"
                | "screenDown"
                | "screenUp"
                | "autoScrollPause"
                | "zoomIn"
                | "zoomOut"
                | "rotateClockwise"
                | "rotateAnticlockwise"
                | "autoScrollFaster"
                | "autoScrollSlower"
                | "historyBack"
                | "historyForward"
        )
    );
    let when = if id == CommandId::LeaveModes {
        nsh_core::keymap::When::Always
    } else {
        nsh_core::keymap::When::Auto
    };
    KeyOptions {
        when,
        repeat,
        pass_through: false,
        for_focus: true,
        over_focused: true,
        ..KeyOptions::default()
    }
}

/// Binds every window-scoped shortcut in the current keymap to `run` while the current owner lives.
pub(crate) fn bind_window_keys(run: impl Fn(CommandId) + Copy + 'static) {
    for binding in keymap::current().bindings() {
        if binding.scope == Scope::Window {
            let id = binding.id;
            on_key_with(&binding.keys, window_key_options(id), move |_| run(id));
        }
    }
}

/// Binds page-scoped shortcuts matching `state` to `run` while the current owner lives.
///
/// Calling this inside a reactive effect keyed on [`PageKeyState`] unmounts all page shortcuts
/// whenever Home, a dialog, a popover, or a sidebar list owns focus, so arrows, `Space`, `Enter`,
/// `Delete`, and single letters never interfere with chrome or dialogs.
pub(crate) fn bind_page_keys(state: PageKeyState, run: impl Fn(CommandId) + Copy + 'static) {
    if state == PageKeyState::Inactive {
        return;
    }
    for binding in keymap::current().bindings() {
        if binding.scope != Scope::Page {
            continue;
        }
        let id = binding.id;
        if state == PageKeyState::FullScreenEscapeOnly && id != CommandId::LeaveModes {
            continue;
        }
        on_key_with(&binding.keys, page_key_options(id), move |_| run(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_found() {
        for command in COMMANDS {
            assert_eq!(find(command.name), Some(command.id), "{}", command.name);
        }
        assert_eq!(find("no.such.command"), None);
    }

    #[test]
    fn only_window_commands_are_offered_without_a_document() {
        assert!(CommandId::OpenPalette.is_available(false));
        assert!(CommandId::ToggleFullScreen.is_available(false));
        assert!(!CommandId::Canvas("zoomIn").is_available(false));
        assert!(!CommandId::GoToPage.is_available(false));
        assert!(CommandId::Canvas("zoomIn").is_available(true));
        assert!(!CommandId::Tab(TabAction::Close).is_available(false));
        assert!(CommandId::Tab(TabAction::ReopenClosed).is_available(false));
        assert!(CommandId::OpenFile.is_available(false));
        assert!(!CommandId::Pane(PaneAction::Split(Side::Right)).is_available(false));
        assert!(CommandId::Pane(PaneAction::OpenInSplit).is_available(false));
        assert!(CommandId::Workspace(WorkspaceAction::New(Start::Empty)).is_available(false));
        assert!(
            !CommandId::Workspace(WorkspaceAction::SendTab(Transfer::Move)).is_available(false)
        );
        assert!(!CommandId::Undo.is_available(false));
        assert!(!CommandId::Outline(OutlineAction::Add).is_available(false));
    }

    #[test]
    fn every_command_has_english_text() {
        for command in COMMANDS {
            for key in [command.label, command.group] {
                assert!(
                    crate::l10n::EN.contains(&format!("\"{key}\":")),
                    "{key} is missing from app/locales/en.json"
                );
            }
        }
    }

    #[test]
    fn every_canvas_command_name_is_implemented_by_page_canvas() {
        // Mirrors `ecclesia_canvas::page::component::PageCanvas::command` + `ViewCommand::parse`.
        const KNOWN_CANVAS_COMMANDS: &[&str] = &[
            "zoomIn",
            "zoomOut",
            "actualSize",
            "screenDown",
            "screenUp",
            "stepLeft",
            "stepRight",
            "historyBack",
            "historyForward",
            "firstPage",
            "lastPage",
            "copy",
            "highlight",
            "blockHighlight",
            "underline",
            "squiggly",
            "strikeOut",
            "replaceText",
            "insertText",
            "noteSelection",
            "selectAll",
            "toggleRuler",
            "toggleFocus",
            "lineDown",
            "lineUp",
            "toggleAutoScroll",
            "autoScrollPause",
            "autoScrollFaster",
            "autoScrollSlower",
            "layoutSingle",
            "layoutFacing",
            "layoutBook",
            "layoutHorizontal",
            "layoutGrid",
            "toggleContinuous",
            "toggleDirection",
            "rotateClockwise",
            "rotateAnticlockwise",
            "boxMedia",
            "boxCrop",
            "boxBleed",
            "boxTrim",
            "boxArt",
            "fitWidth",
            "fitPage",
            "fitHeight",
        ];

        for command in COMMANDS {
            if let CommandId::Canvas(name) = command.id {
                assert!(
                    KNOWN_CANVAS_COMMANDS.contains(&name),
                    "command {:?} targets unknown PageCanvas command {name:?}",
                    command.name
                );
            }
        }
    }

    #[test]
    fn window_key_options_cover_repeat_always_and_over_focused() {
        use nsh_core::keymap::When;

        let zoom = window_key_options(CommandId::Canvas("zoomIn"));
        assert!(zoom.repeat);
        assert_eq!(zoom.when, When::Auto);
        assert!(!zoom.over_focused);

        let page_down = window_key_options(CommandId::Canvas("screenDown"));
        assert!(page_down.repeat);

        let rotate = window_key_options(CommandId::Canvas("rotateClockwise"));
        assert!(rotate.repeat);

        let history_back = window_key_options(CommandId::Canvas("historyBack"));
        assert!(history_back.repeat);
        assert_eq!(history_back.when, When::Always);
        assert!(history_back.over_focused);

        let tab_cycle = window_key_options(CommandId::Tab(TabAction::Cycle(Step::Next)));
        assert!(tab_cycle.repeat);
        assert_eq!(tab_cycle.when, When::Always);
        assert!(tab_cycle.over_focused);

        let tab_move = window_key_options(CommandId::Tab(TabAction::Move(Step::Next)));
        assert!(!tab_move.repeat);
        assert_eq!(tab_move.when, When::Always);
        assert!(tab_move.over_focused);

        let palette = window_key_options(CommandId::OpenPalette);
        assert_eq!(palette.when, When::Always);
        assert!(palette.over_focused);

        let copy = window_key_options(CommandId::Canvas("copy"));
        assert!(!copy.repeat);
        assert_eq!(copy.when, When::Auto);
        assert!(!copy.over_focused);
    }

    #[test]
    fn page_key_options_consume_keys_for_focus_and_repeat_only_navigation() {
        use nsh_core::keymap::When;

        for id in [
            CommandId::Canvas("lineDown"),
            CommandId::Canvas("stepLeft"),
            CommandId::Canvas("screenDown"),
            CommandId::Canvas("autoScrollPause"),
            CommandId::Canvas("zoomIn"),
            CommandId::Canvas("rotateClockwise"),
            CommandId::Canvas("autoScrollFaster"),
        ] {
            let opts = page_key_options(id);
            assert!(!opts.pass_through, "{id:?} must consume its key");
            assert!(
                opts.for_focus,
                "{id:?} must win over residual control focus"
            );
            assert!(
                opts.over_focused,
                "{id:?} must win over roving toolbar/tab keys"
            );
            assert!(opts.repeat, "{id:?} should repeat while held");
            assert_eq!(opts.when, When::Auto, "{id:?} must yield to text inputs");
        }

        for id in [
            CommandId::ToggleFullScreen,
            CommandId::ToggleDocumentColors,
            CommandId::GoToPage,
            CommandId::Canvas("highlight"),
            CommandId::Canvas("toggleContinuous"),
            CommandId::BlockSelect,
        ] {
            let opts = page_key_options(id);
            assert!(!opts.pass_through, "{id:?}");
            assert!(opts.for_focus, "{id:?}");
            assert!(opts.over_focused, "{id:?}");
            assert!(!opts.repeat, "{id:?} must not flicker when held");
            assert_eq!(opts.when, When::Auto, "{id:?}");
        }

        let leave = page_key_options(CommandId::LeaveModes);
        assert!(!leave.pass_through);
        assert!(leave.for_focus);
        assert!(leave.over_focused);
        assert!(!leave.repeat);
        assert_eq!(leave.when, When::Always);
    }

    #[test]
    fn os_shortcut_parses_all_actions_and_maps_commands() {
        let cases = [
            (
                "toggleFullScreen",
                OsShortcut::ToggleFullScreen,
                Some(CommandId::ToggleFullScreen),
                false,
            ),
            ("quit", OsShortcut::Quit, None, false),
            (
                "zoomIn",
                OsShortcut::ZoomIn,
                Some(CommandId::Canvas("zoomIn")),
                false,
            ),
            (
                "zoomOut",
                OsShortcut::ZoomOut,
                Some(CommandId::Canvas("zoomOut")),
                false,
            ),
            (
                "rotateClockwise",
                OsShortcut::RotateClockwise,
                Some(CommandId::Canvas("rotateClockwise")),
                false,
            ),
            (
                "rotateAnticlockwise",
                OsShortcut::RotateAnticlockwise,
                Some(CommandId::Canvas("rotateAnticlockwise")),
                false,
            ),
            (
                "autoScrollFaster",
                OsShortcut::AutoScrollFaster,
                Some(CommandId::Canvas("autoScrollFaster")),
                false,
            ),
            (
                "autoScrollSlower",
                OsShortcut::AutoScrollSlower,
                Some(CommandId::Canvas("autoScrollSlower")),
                false,
            ),
            (
                "pageZoomIn",
                OsShortcut::PageZoomIn,
                Some(CommandId::Canvas("zoomIn")),
                true,
            ),
            (
                "pageZoomOut",
                OsShortcut::PageZoomOut,
                Some(CommandId::Canvas("zoomOut")),
                true,
            ),
        ];
        for (action, expected, cmd, page_scoped) in cases {
            let parsed = OsShortcut::from_event(&serde_json::json!({ "action": action }))
                .unwrap_or_else(|| panic!("action {action:?} should parse"));
            assert_eq!(parsed, expected);
            assert_eq!(parsed.command(), cmd);
            assert_eq!(parsed.is_page_scoped(), page_scoped);
        }
        assert_eq!(
            OsShortcut::from_event(&serde_json::json!({ "action": "unknown" })),
            None
        );
        assert_eq!(OsShortcut::from_event(&serde_json::json!({})), None);
    }
}
