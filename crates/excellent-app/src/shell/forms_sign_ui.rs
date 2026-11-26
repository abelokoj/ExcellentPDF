//! Phase 7 Forms filling, validation, export/import, autofill profiles, flattening, and
//! digital signature verification/signing/LTV UI (`O.11`–`O.16`, `O.20`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use excellent_core::forms::{AcroForm, FieldKind, FieldValue, FlattenTarget, flatten_pdf_bytes};
use excellent_core::signing::{
    CertRole, CertValidity, DistinguishedName, DocMdpPermission, DocumentEditKind, PadesLevel,
    SignatureAppearance, TrustStore, VisibleAppearance, VisibleSignatureContent, X509Certificate,
};
use excellent_features::forms::{
    ExportScope, FormProfile, FormSession, ProfileMatchMode, ProfileStore, apply_profile,
    export_csv, export_fdf, export_json, export_xfdf, import_csv, import_fdf, import_json,
    import_xfdf,
};
use excellent_features::signing::{
    EditGuardDecision, LtvRevocationEvidence, OnlineTimestampConsent, SignaturePanelModel,
    SigningWorkflow, TimestampConfig, evaluate_edit_guard, extract_signed_revision,
};
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::ai_ui::{labeled_field, pill_button, status_caption};
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::panes::Panes;
use crate::settings::{self, Live, write_atomically};
use crate::settings_window::{segment_owned, segmented};
use crate::workspaces::Workspaces;

/// `settings.toml` key authorizing remote RFC 3161 TSA timestamping (`O.14` / `O.20`).
pub(crate) const TSA_REMOTE_CONSENT: &str = "tsa_remote_consent";
/// `settings.toml` key storing an optional remote TSA URL (`O.14` / `O.20`).
pub(crate) const TSA_REMOTE_URL: &str = "tsa_remote_url";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn profiles_db_path() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-profiles"))
        .join("form_profiles.sqlite")
}

fn signing_dir() -> PathBuf {
    settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-signing"))
        .join("signing")
}

/// Active sub-view in the Forms dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FormsTab {
    /// Fill, validate & tab-navigate AcroForm fields (`O.11`).
    #[default]
    Fill,
    /// Import/export JSON, CSV, FDF, XFDF and manage reusable autofill profiles (`O.12`).
    ExchangeAndProfiles,
}

/// Persistent UI state for Forms & Digital Signatures dialogs.
#[derive(Clone, Copy)]
pub(crate) struct FormsSignUiState {
    forms_open: Signal<bool>,
    signatures_open: Signal<bool>,
    forms_tab: Signal<FormsTab>,
    form_session: Stored<Option<FormSession>>,
    form_rev: Signal<u64>,
    field_value_draft: Signal<String>,
    profile_name_draft: Signal<String>,
    saved_profiles: Stored<Vec<FormProfile>>,
    forms_status: Signal<String>,
    sig_panel: Stored<Option<SignaturePanelModel>>,
    sig_rev: Signal<u64>,
    signer_name: Signal<String>,
    sign_reason: Signal<String>,
    sign_location: Signal<String>,
    pades_level: Signal<PadesLevel>,
    sign_visible: Signal<bool>,
    sig_status: Signal<String>,
}

impl FormsSignUiState {
    /// Creates reactive state for Phase 7 Forms & Signing UI.
    pub(crate) fn new() -> Self {
        Self {
            forms_open: signal(false),
            signatures_open: signal(false),
            forms_tab: signal(FormsTab::Fill),
            form_session: stored(None),
            form_rev: signal(0),
            field_value_draft: signal(String::new()),
            profile_name_draft: signal("Default Profile".to_owned()),
            saved_profiles: stored(Vec::new()),
            forms_status: signal(String::new()),
            sig_panel: stored(None),
            sig_rev: signal(0),
            signer_name: signal("Document Signer".to_owned()),
            sign_reason: signal("Approved".to_owned()),
            sign_location: signal(String::new()),
            pades_level: signal(PadesLevel::B),
            sign_visible: signal(true),
            sig_status: signal(String::new()),
        }
    }

    /// True when either the Forms or Digital Signatures modal is displayed.
    pub(crate) fn any_dialog_open(self) -> bool {
        self.forms_open.get() || self.signatures_open.get()
    }

    /// Opens the Forms dialog and loads the active PDF's AcroForm (`O.11`–`O.12`).
    pub(crate) fn open_forms(self, panes: Panes) {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        match AcroForm::from_pdf_bytes(&bytes) {
            Ok(form) => {
                let session = FormSession::new(form);
                let active_val = session
                    .active_field()
                    .and_then(|n| session.form().field(n))
                    .map(field_value_display)
                    .unwrap_or_default();
                self.field_value_draft.set(active_val);
                self.form_session.with(|slot| *slot = Some(session));
                self.forms_status.set(String::new());
            }
            Err(err) => {
                self.form_session.with(|slot| *slot = None);
                self.forms_status.set(err.to_string());
            }
        }
        if let Ok(conn) = excellent_store::rusqlite::Connection::open(profiles_db_path())
            && let Ok(store) = ProfileStore::load_from_sqlite(&conn)
        {
            let list: Vec<FormProfile> = store.list().into_iter().cloned().collect();
            self.saved_profiles.with(|slot| *slot = list);
        }
        self.form_rev.update(|r| *r = r.wrapping_add(1));
        self.forms_open.set(true);
    }

    /// Flattens AcroForm fields of the active PDF into a chosen output PDF (`O.13`).
    pub(crate) fn flatten_forms(self, panes: Panes) {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "form".to_owned());
        let dialog = SaveDialog::new()
            .title(t!("forms.flatten.title"))
            .file_name(format!("{stem}-flattened.pdf"))
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let res = blocking(move || {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let flat = flatten_pdf_bytes(&bytes, FlattenTarget::FormsOnly)
                    .map_err(|e| e.to_string())?;
                write_atomically(&dest, &flat).map_err(|e| e.to_string())?;
                Ok::<_, String>(dest)
            })
            .await;
            match res {
                Ok(Ok(dest)) => show_message(
                    t!("forms.flatten.done_title"),
                    t!("forms.flatten.done_detail", path = dest.to_string_lossy()),
                ),
                Ok(Err(err)) => show_message(t!("forms.flatten.title"), err),
                Err(err) => show_message(t!("forms.flatten.title"), err.to_string()),
            }
        });
    }

    /// Opens the Digital Signatures inspector and signing wizard (`O.14`–`O.16`, `O.20`).
    pub(crate) fn open_signatures(self, panes: Panes) {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        let store = TrustStore::new();
        match SignaturePanelModel::inspect(&bytes, &store) {
            Ok(panel) => {
                self.sig_panel.with(|slot| *slot = Some(panel));
                self.sig_status.set(String::new());
            }
            Err(err) => {
                self.sig_panel.with(|slot| *slot = None);
                self.sig_status.set(err.to_string());
            }
        }
        self.sig_rev.update(|r| *r = r.wrapping_add(1));
        self.signatures_open.set(true);
    }
}

fn field_value_display(field: &excellent_core::forms::FormField) -> String {
    match &field.value {
        FieldValue::None => String::new(),
        FieldValue::Text(s) => s.clone(),
        FieldValue::Boolean(b) => {
            if *b {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        }
        FieldValue::Choice(v) => v.join(", "),
    }
}

fn parse_value_for_field(kind: &FieldKind, raw: &str) -> FieldValue {
    match kind {
        FieldKind::Checkbox { .. } | FieldKind::RadioGroup { .. } => {
            let trimmed = raw.trim();
            FieldValue::Boolean(
                trimmed.eq_ignore_ascii_case("true")
                    || trimmed == "1"
                    || trimmed.eq_ignore_ascii_case("yes")
                    || trimmed.eq_ignore_ascii_case("on"),
            )
        }
        FieldKind::Choice { .. } => {
            let items: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            FieldValue::Choice(items)
        }
        _ => FieldValue::Text(raw.to_owned()),
    }
}

/// Renders open Forms and Signatures modals.
pub(crate) fn forms_sign_dialogs(
    state: FormsSignUiState,
    panes: Panes,
    workspaces: Workspaces,
    live: Live,
) -> impl IntoElement {
    view()
        .child(when(
            move || state.forms_open.get(),
            move || forms_dialog(state, panes, workspaces),
        ))
        .child(when(
            move || state.signatures_open.get(),
            move || signatures_dialog(state, panes, workspaces, live),
        ))
}

fn forms_dialog(state: FormsSignUiState, panes: Panes, workspaces: Workspaces) -> impl IntoElement {
    let close = move || state.forms_open.set(false);

    let tabs = segmented()
        .child(segment_owned(
            t!("forms.tab.fill"),
            move || state.forms_tab.get() == FormsTab::Fill,
            move || state.forms_tab.set(FormsTab::Fill),
        ))
        .child(segment_owned(
            t!("forms.tab.exchange"),
            move || state.forms_tab.get() == FormsTab::ExchangeAndProfiles,
            move || state.forms_tab.set(FormsTab::ExchangeAndProfiles),
        ));

    let save_pdf = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let Some(session) = state.form_session.with(|s| s.clone()) else {
            return;
        };
        let orig = std::fs::read(&path).unwrap_or_default();
        if let Ok(EditGuardDecision::WarnUser { reason, .. }) =
            evaluate_edit_guard(&orig, &DocumentEditKind::FormFill { field_name: None })
        {
            state.forms_status.set(reason);
            return;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "filled".to_owned());
        let dialog = SaveDialog::new()
            .title(t!("forms.save_pdf"))
            .file_name(format!("{stem}-filled.pdf"))
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            match session.form().to_pdf_bytes(&orig) {
                Ok(out_bytes) => {
                    if let Err(err) = write_atomically(&dest, &out_bytes) {
                        state.forms_status.set(err.to_string());
                    } else {
                        state.forms_status.set(t!("forms.saved"));
                        workspaces.open_paths(vec![dest]);
                    }
                }
                Err(err) => state.forms_status.set(err.to_string()),
            }
        });
    };

    let body = card(
        view()
            .w(px(640.0))
            .max_h(px(540.0))
            .gap_3()
            .child(tabs)
            .child(when(
                move || !state.forms_status.get().is_empty(),
                move || status_caption(move || state.forms_status.get()),
            ))
            .child(when(
                move || state.forms_tab.get() == FormsTab::Fill,
                move || forms_fill_body(state),
            ))
            .child(when(
                move || state.forms_tab.get() == FormsTab::ExchangeAndProfiles,
                move || forms_exchange_body(state),
            ))
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(t!("forms.flatten.button"), false, move || {
                        state.flatten_forms(panes);
                    }))
                    .child(pill_button(t!("forms.save_pdf"), true, save_pdf))
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );

    list_dialog(t!("forms.title"), close, body)
}

fn forms_fill_body(state: FormsSignUiState) -> impl IntoElement {
    let apply_active = move || {
        let val_str = state.field_value_draft.get_untracked();
        state.form_session.with(|opt| {
            let Some(session) = opt else {
                return;
            };
            let Some(name) = session.active_field().map(str::to_owned) else {
                return;
            };
            if let Some(f) = session.form().field(&name).cloned() {
                let parsed = parse_value_for_field(&f.kind, &val_str);
                match session.set_field(&name, parsed) {
                    Ok(report) => {
                        state.forms_status.set(t!(
                            "forms.recalculated",
                            count = report.updated_fields.len()
                        ));
                    }
                    Err(err) => state.forms_status.set(err.to_string()),
                }
            }
        });
        state.form_rev.update(|r| *r = r.wrapping_add(1));
    };

    let step_tab = move |forward: bool| {
        state.form_session.with(|opt| {
            let Some(session) = opt else {
                return;
            };
            let next = if forward {
                session.next_tab_field().map(str::to_owned)
            } else {
                session.previous_tab_field().map(str::to_owned)
            };
            if let Some(name) = next
                && let Some(f) = session.form().field(&name)
            {
                state.field_value_draft.set(field_value_display(f));
            }
        });
        state.form_rev.update(|r| *r = r.wrapping_add(1));
    };

    let validate_all = move || {
        state.form_session.with(|opt| {
            let Some(session) = opt else {
                return;
            };
            let report = session.validate();
            let highlights = session.highlights();
            if report.is_valid() {
                state
                    .forms_status
                    .set(t!("forms.valid", highlights = highlights.len()));
            } else {
                state.forms_status.set(t!(
                    "forms.invalid",
                    errors = report.issues.len(),
                    highlights = highlights.len()
                ));
            }
        });
    };

    let reset_all = move || {
        state.form_session.with(|opt| {
            if let Some(session) = opt {
                let _ = session.reset_to_defaults();
            }
        });
        state.form_rev.update(|r| *r = r.wrapping_add(1));
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(t!("forms.prev_field"), false, move || {
                    step_tab(false)
                }))
                .child(pill_button(t!("forms.next_field"), false, move || {
                    step_tab(true)
                }))
                .child(pill_button(t!("forms.validate"), false, validate_all))
                .child(pill_button(t!("forms.reset"), false, reset_all)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    input()
                        .value(move || state.field_value_draft.get())
                        .on_change(move |v: &str| state.field_value_draft.set(v.to_owned()))
                        .on_submit(move |_: &str| apply_active())
                        .placeholder(t!("forms.value_placeholder"))
                        .label(t!("forms.value_label"))
                        .flex_1()
                        .h(px(34.0))
                        .px_3()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || theme().input),
                )
                .child(pill_button(t!("forms.apply_field"), true, apply_active)),
        )
        .child(scroll().max_h(px(230.0)).child(dynamic(move || {
            let _rev = state.form_rev.get();
            let Some(session) = state.form_session.with(|s| s.clone()) else {
                return status_caption(move || t!("forms.none")).into_element();
            };
            let active = session.active_field().map(str::to_owned);
            let mut list = view().gap_1();
            for f in session.form().tab_ordered_fields() {
                let fname = f.name.clone();
                let is_active = active.as_deref() == Some(fname.as_str());
                let disp = field_value_display(f);
                let val_for_pick = disp.clone();
                let label = format!("{}: {}", fname, if disp.is_empty() { "—" } else { &disp });
                list = list.child(pill_button(label, is_active, move || {
                    state.form_session.with(|opt| {
                        if let Some(s) = opt {
                            s.focus_field(&fname);
                        }
                    });
                    state.field_value_draft.set(val_for_pick.clone());
                    state.form_rev.update(|r| *r = r.wrapping_add(1));
                }));
            }
            list.into_element()
        })))
}

#[derive(Clone, Copy)]
enum ExchangeFmt {
    Json,
    Csv,
    Fdf,
    Xfdf,
}

fn forms_exchange_body(state: FormsSignUiState) -> impl IntoElement {
    let do_export = move |fmt: ExchangeFmt| {
        let Some(session) = state.form_session.with(|s| s.clone()) else {
            return;
        };
        let (ext, title, res): (&str, String, Result<Vec<u8>, String>) = match fmt {
            ExchangeFmt::Json => (
                "json",
                t!("forms.export.json"),
                export_json(session.form(), ExportScope::ExportableOnly)
                    .map(String::into_bytes)
                    .map_err(|e| e.to_string()),
            ),
            ExchangeFmt::Csv => (
                "csv",
                t!("forms.export.csv"),
                export_csv(session.form(), ExportScope::ExportableOnly)
                    .map(String::into_bytes)
                    .map_err(|e| e.to_string()),
            ),
            ExchangeFmt::Fdf => (
                "fdf",
                t!("forms.export.fdf"),
                Ok(export_fdf(
                    session.form(),
                    None,
                    ExportScope::ExportableOnly,
                )),
            ),
            ExchangeFmt::Xfdf => (
                "xfdf",
                t!("forms.export.xfdf"),
                Ok(export_xfdf(session.form(), None, ExportScope::ExportableOnly).into_bytes()),
            ),
        };
        let data = match res {
            Ok(s) => s,
            Err(err) => {
                state.forms_status.set(err);
                return;
            }
        };
        let dialog = SaveDialog::new()
            .title(title)
            .file_name(format!("form-data.{ext}"))
            .filter(FileFilter::new(ext.to_uppercase(), [ext]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(path)) = answer.await {
                let _ = write_atomically(&path, &data);
                state.forms_status.set(t!("forms.exported"));
            }
        });
    };

    let do_import = move || {
        let dialog = OpenDialog::files()
            .title(t!("forms.import"))
            .filter(FileFilter::new(
                t!("forms.import_filter"),
                ["json", "csv", "fdf", "xfdf"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                return;
            };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            state.form_session.with(|opt| {
                let Some(session) = opt else {
                    return;
                };
                let res = match ext.as_str() {
                    "csv" => import_csv(session.form_mut(), &text),
                    "fdf" => import_fdf(session.form_mut(), text.as_bytes()),
                    "xfdf" => import_xfdf(session.form_mut(), &text),
                    _ => import_json(session.form_mut(), &text),
                };
                match res {
                    Ok(summary) => state
                        .forms_status
                        .set(t!("forms.imported", updated = summary.updated)),
                    Err(err) => state.forms_status.set(err.to_string()),
                }
            });
            state.form_rev.update(|r| *r = r.wrapping_add(1));
        });
    };

    let save_profile = move || {
        let name = state.profile_name_draft.get_untracked();
        let trimmed = name.trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        let Some(session) = state.form_session.with(|s| s.clone()) else {
            return;
        };
        let profile = FormProfile::from_form(
            trimmed.to_ascii_lowercase().replace(' ', "-"),
            trimmed,
            session.form(),
            now_secs(),
        );
        let db = profiles_db_path();
        if let Some(parent) = db.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(conn) = excellent_store::rusqlite::Connection::open(&db) {
            let mut store = ProfileStore::load_from_sqlite(&conn).unwrap_or_default();
            store.save(profile);
            if let Err(err) = store.save_to_sqlite(&conn) {
                state.forms_status.set(err.to_string());
            } else {
                let list: Vec<FormProfile> = store.list().into_iter().cloned().collect();
                state.saved_profiles.with(|slot| *slot = list);
                state.forms_status.set(t!("forms.profile.saved"));
                state.form_rev.update(|r| *r = r.wrapping_add(1));
            }
        }
    };

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill_button(t!("forms.export.json"), false, move || {
                    do_export(ExchangeFmt::Json)
                }))
                .child(pill_button(t!("forms.export.csv"), false, move || {
                    do_export(ExchangeFmt::Csv)
                }))
                .child(pill_button(t!("forms.export.fdf"), false, move || {
                    do_export(ExchangeFmt::Fdf)
                }))
                .child(pill_button(t!("forms.export.xfdf"), false, move || {
                    do_export(ExchangeFmt::Xfdf)
                }))
                .child(pill_button(t!("forms.import"), true, do_import)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(labeled_field(
                    move || state.profile_name_draft.get(),
                    move |v| state.profile_name_draft.set(v),
                    t!("forms.profile.placeholder"),
                    t!("forms.profile.label"),
                ))
                .child(pill_button(t!("forms.profile.save"), false, save_profile)),
        )
        .child(scroll().max_h(px(170.0)).child(dynamic(move || {
            let _rev = state.form_rev.get();
            let profiles = state.saved_profiles.with(|v| v.clone());
            if profiles.is_empty() {
                return status_caption(move || t!("forms.profile.empty")).into_element();
            }
            let mut list = view().gap_1();
            for prof in profiles {
                let p_clone = prof.clone();
                list = list.child(pill_button(
                    t!(
                        "forms.profile.apply",
                        name = prof.name,
                        count = prof.values.len()
                    ),
                    false,
                    move || {
                        state.form_session.with(|opt| {
                            if let Some(session) = opt {
                                match apply_profile(
                                    &p_clone,
                                    session.form_mut(),
                                    ProfileMatchMode::NormalizedName,
                                ) {
                                    Ok(rep) => state.forms_status.set(t!(
                                        "forms.imported",
                                        updated = rep.matched_fields.len()
                                    )),
                                    Err(err) => state.forms_status.set(err.to_string()),
                                }
                            }
                        });
                        state.form_rev.update(|r| *r = r.wrapping_add(1));
                    },
                ));
            }
            list.into_element()
        })))
}

fn signatures_dialog(
    state: FormsSignUiState,
    panes: Panes,
    workspaces: Workspaces,
    live: Live,
) -> impl IntoElement {
    let close = move || state.signatures_open.set(false);

    let sign_now = move || {
        let Some(path) = panes.active().tabs.shown_path() else {
            return;
        };
        let signer = state.signer_name.get_untracked();
        let reason = state.sign_reason.get_untracked();
        let location = state.sign_location.get_untracked();
        let visible = state.sign_visible.get_untracked();
        let tsa_consent = if live.read(|s| s.flag(TSA_REMOTE_CONSENT).unwrap_or(false)) {
            OnlineTimestampConsent::Allowed
        } else {
            OnlineTimestampConsent::Denied
        };
        let tsa_url = live.read(|s| s.text(TSA_REMOTE_URL).unwrap_or_default().to_owned());

        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "signed".to_owned());
        let dialog = SaveDialog::new()
            .title(t!("sign.save_title"))
            .file_name(format!("{stem}-signed.pdf"))
            .filter(FileFilter::new(t!("filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(dest)) = answer.await else {
                return;
            };
            let res = blocking(move || {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let dir = signing_dir();
                let _ = std::fs::create_dir_all(&dir);
                let now = now_secs();
                let seed: [u8; 32] = {
                    let key_path = dir.join("default-ed25519.key");
                    if let Ok(raw) = std::fs::read(&key_path)
                        && raw.len() == 32
                    {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&raw);
                        arr
                    } else {
                        let arr = [0x42u8; 32];
                        let _ = write_atomically(&key_path, arr);
                        arr
                    }
                };
                let key = SigningKey::from_bytes(&seed);
                let cert = X509Certificate::issue_ed25519(
                    &DistinguishedName {
                        common_name: signer.clone(),
                        organization: Some("ExcellentPDF".to_owned()),
                        country: Some("US".to_owned()),
                    },
                    &key.verifying_key(),
                    None,
                    &key,
                    CertValidity {
                        not_before: now.saturating_sub(60),
                        not_after: now.saturating_add(365 * 24 * 3600),
                    },
                    CertRole::EndEntity,
                    1,
                )
                .map_err(|e| e.to_string())?;
                let appearance = if visible {
                    SignatureAppearance::Visible(VisibleAppearance {
                        page: 0,
                        rect: excellent_mupdf::PageRect {
                            x0: 40.0,
                            y0: 40.0,
                            x1: 240.0,
                            y1: 96.0,
                        },
                        content: VisibleSignatureContent::Typed {
                            text: signer.clone(),
                            font_name: "Helv".to_owned(),
                        },
                        caption: Some(reason.clone()),
                    })
                } else {
                    SignatureAppearance::Invisible
                };
                let tsa_cfg = if !tsa_url.trim().is_empty()
                    && tsa_consent == OnlineTimestampConsent::Allowed
                {
                    TimestampConfig::Remote {
                        url: tsa_url,
                        consent: tsa_consent,
                    }
                } else {
                    TimestampConfig::None
                };
                let wf = SigningWorkflow {
                    signer_cert: &cert,
                    chain_certs: &[],
                    signing_key: &key,
                    field_name: format!("Sig_{now}"),
                    reason: (!reason.trim().is_empty()).then_some(reason),
                    location: (!location.trim().is_empty()).then_some(location),
                    signing_time: now,
                    appearance,
                    certification: Some(DocMdpPermission::FormFillAndSign),
                    timestamp: tsa_cfg,
                    ltv_revocation: Some(LtvRevocationEvidence::default()),
                };
                let signed = wf.execute(&bytes).map_err(|e| e.to_string())?;
                write_atomically(&dest, &signed).map_err(|e| e.to_string())?;
                Ok::<_, String>(dest)
            })
            .await;
            match res {
                Ok(Ok(dest)) => {
                    state
                        .sig_status
                        .set(t!("sign.signed_ok", path = dest.to_string_lossy()));
                    workspaces.open_paths(vec![dest]);
                }
                Ok(Err(err)) => state.sig_status.set(err),
                Err(err) => state.sig_status.set(err.to_string()),
            }
        });
    };

    let body = card(
        view()
            .w(px(620.0))
            .max_h(px(540.0))
            .gap_3()
            .child(when(
                move || !state.sig_status.get().is_empty(),
                move || status_caption(move || state.sig_status.get()),
            ))
            .child(scroll().max_h(px(190.0)).child(dynamic(move || {
                let _rev = state.sig_rev.get();
                let Some(panel) = state.sig_panel.with(|p| p.clone()) else {
                    return status_caption(move || t!("sign.empty")).into_element();
                };
                if panel.signatures.is_empty() {
                    return status_caption(move || t!("sign.empty")).into_element();
                }
                let mut list = view().gap_2();
                for (idx, sig) in panel.signatures.into_iter().enumerate() {
                    let field_name = sig.field_name.clone();
                    let status_txt = format!(
                        "{} — {:?} ({:?})",
                        sig.signer_name, sig.trust_status, panel.pades.achieved_level
                    );
                    let issuer = sig.issuer_name.clone();
                    list = list.child(
                        view()
                            .p_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || theme().border)
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                view()
                                    .gap_1()
                                    .child(
                                        text(format!("{field_name}: {status_txt}"))
                                            .text_size(px(12.0)),
                                    )
                                    .child(status_caption(move || issuer.clone())),
                            )
                            .child(pill_button(t!("sign.view_revision"), false, move || {
                                if let Some(path) = panes.active().tabs.shown_path()
                                    && let Ok(bytes) = std::fs::read(&path)
                                    && let Ok(rev_bytes) = extract_signed_revision(&bytes, idx)
                                {
                                    let tmp = std::env::temp_dir()
                                        .join(format!("signed-rev-{}.pdf", idx + 1));
                                    if write_atomically(&tmp, &rev_bytes).is_ok() {
                                        workspaces.open_paths(vec![tmp]);
                                    }
                                }
                            })),
                    );
                }
                list.into_element()
            })))
            .child(
                view()
                    .gap_2()
                    .child(labeled_field(
                        move || state.signer_name.get(),
                        move |v| state.signer_name.set(v),
                        t!("sign.signer_placeholder"),
                        t!("sign.signer_label"),
                    ))
                    .child(labeled_field(
                        move || state.sign_reason.get(),
                        move |v| state.sign_reason.set(v),
                        t!("sign.reason_placeholder"),
                        t!("sign.reason_label"),
                    ))
                    .child(labeled_field(
                        move || state.sign_location.get(),
                        move |v| state.sign_location.set(v),
                        t!("sign.location_placeholder"),
                        t!("sign.location_label"),
                    ))
                    .child(
                        segmented()
                            .child(segment_owned(
                                t!("sign.pades.bb"),
                                move || state.pades_level.get() == PadesLevel::B,
                                move || state.pades_level.set(PadesLevel::B),
                            ))
                            .child(segment_owned(
                                t!("sign.pades.bt"),
                                move || state.pades_level.get() == PadesLevel::T,
                                move || state.pades_level.set(PadesLevel::T),
                            ))
                            .child(segment_owned(
                                t!("sign.pades.blt"),
                                move || state.pades_level.get() == PadesLevel::LT,
                                move || state.pades_level.set(PadesLevel::LT),
                            ))
                            .child(segment_owned(
                                t!("sign.pades.blta"),
                                move || state.pades_level.get() == PadesLevel::LTA,
                                move || state.pades_level.set(PadesLevel::LTA),
                            )),
                    ),
            )
            .child(
                view()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(pill_button(
                        if state.sign_visible.get() {
                            t!("sign.visible_on")
                        } else {
                            t!("sign.visible_off")
                        },
                        false,
                        move || state.sign_visible.update(|v| *v = !*v),
                    ))
                    .child(pill_button(t!("sign.apply_button"), true, sign_now))
                    .child(pill_button(t!("action.close"), false, close)),
            ),
    );

    list_dialog(t!("sign.title"), close, body)
}
