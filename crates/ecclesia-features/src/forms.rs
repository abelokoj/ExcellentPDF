//! Interactive forms feature module: filling/validation/calculation sessions (O.11),
//! JSON/CSV/FDF/XFDF exchange & reusable autofill profiles (O.12), and form flattening (O.13).

mod exchange;
mod profiles;
mod session;

pub use exchange::{
    ExportScope, FormExchangeError, FormExchangeFormat, ImportSummary, export_csv, export_csv_rows,
    export_fdf, export_json, export_xfdf, import_csv, import_fdf, import_json, import_xfdf,
};
pub use profiles::{AutofillReport, FormProfile, ProfileMatchMode, ProfileStore, apply_profile};
pub use session::{FieldHighlight, FormSession, HighlightTone};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ecclesia_core::forms::{
        AcroForm, ChoiceKind, ChoiceOption, FieldFlags, FieldKind, FieldValue, FormField,
        ValidationRule, WidgetAppearance,
    };
    use ecclesia_mupdf::PageRect;
    use rusqlite::Connection;

    use super::*;

    fn sample_form() -> AcroForm {
        let mut form = AcroForm::new();

        let mut name = FormField::text("applicant.full_name", "Ada Lovelace");
        name.flags = name.flags.with(FieldFlags::REQUIRED);
        name.widgets.push(WidgetAppearance::at(
            0,
            PageRect {
                x0: 40.0,
                y0: 20.0,
                x1: 240.0,
                y1: 44.0,
            },
        ));
        form.add_field(name);

        let mut email = FormField::text("applicant.email", "ada@analytical.org");
        email.validation.push(ValidationRule::Email);
        email.widgets.push(WidgetAppearance::at(
            0,
            PageRect {
                x0: 40.0,
                y0: 60.0,
                x1: 240.0,
                y1: 84.0,
            },
        ));
        form.add_field(email);

        let mut agree = FormField::checkbox("terms", "Yes", FieldValue::Boolean(true));
        form.add_field(agree);

        let mut plan = FormField::text("plan", "");
        plan.kind = FieldKind::Choice {
            kind: ChoiceKind::Combo,
            options: vec![ChoiceOption::simple("Basic"), ChoiceOption::simple("Pro")],
            editable: false,
            multi_select: false,
        };
        plan.value = FieldValue::Choice(vec!["Pro".to_owned()]);
        form.add_field(plan);

        let mut score = FormField::text("subtotal", "10");
        form.add_field(score);

        let mut doubled = FormField::text("doubled", "");
        doubled.calculate_script =
            Some(r#"event.value = getField("subtotal").value * 2"#.to_owned());
        form.add_field(doubled);

        agree = form.field("terms").expect("terms").clone();
        let _ = agree;
        score = form.field("subtotal").expect("subtotal").clone();
        let _ = score;

        form.recalculate().expect("recalculate");
        form
    }

    #[test]
    fn round_trips_json_csv_xfdf_and_fdf() {
        let source = sample_form();

        // JSON round-trip
        let json = export_json(&source, ExportScope::ExportableOnly).expect("export json");
        let mut target_json = sample_form();
        target_json
            .set_value("applicant.full_name", FieldValue::None)
            .expect("clear");
        target_json
            .set_value("subtotal", FieldValue::Text("1".into()))
            .expect("set 1");
        let summary = import_json(&mut target_json, &json).expect("import json");
        assert!(summary.updated >= 5);
        assert_eq!(
            target_json
                .field("applicant.full_name")
                .expect("name")
                .value
                .as_display_string(),
            "Ada Lovelace"
        );
        assert_eq!(
            target_json
                .field("doubled")
                .expect("doubled")
                .value
                .as_display_string(),
            "20"
        );

        // CSV round-trip
        let csv = export_csv(&source, ExportScope::ExportableOnly).expect("export csv");
        let mut target_csv = sample_form();
        target_csv
            .set_value(
                "applicant.email",
                FieldValue::Text("other@example.com".into()),
            )
            .expect("clear email");
        import_csv(&mut target_csv, &csv).expect("import csv");
        assert_eq!(
            target_csv
                .field("applicant.email")
                .expect("email")
                .value
                .as_display_string(),
            "ada@analytical.org"
        );

        // XFDF round-trip (nested field elements)
        let xfdf = export_xfdf(
            &source,
            Some("application.pdf"),
            ExportScope::ExportableOnly,
        );
        assert!(xfdf.contains("<field name=\"applicant\"><field name=\"full_name\">"));
        let mut target_xfdf = sample_form();
        target_xfdf
            .set_value("applicant.full_name", FieldValue::Text("Temp".into()))
            .expect("set temp");
        import_xfdf(&mut target_xfdf, &xfdf).expect("import xfdf");
        assert_eq!(
            target_xfdf
                .field("applicant.full_name")
                .expect("name")
                .value
                .as_display_string(),
            "Ada Lovelace"
        );

        // FDF round-trip
        let fdf = export_fdf(
            &source,
            Some("application.pdf"),
            ExportScope::ExportableOnly,
        );
        let mut target_fdf = sample_form();
        target_fdf
            .set_value("applicant.full_name", FieldValue::Text("Temp".into()))
            .expect("set temp");
        import_fdf(&mut target_fdf, &fdf).expect("import fdf");
        assert_eq!(
            target_fdf
                .field("applicant.full_name")
                .expect("name")
                .value
                .as_display_string(),
            "Ada Lovelace"
        );
    }

    #[test]
    fn autofill_profile_fills_second_form_with_matching_or_normalized_names_and_persists_sqlite() {
        let first_form = sample_form();
        let mut profile =
            FormProfile::from_form("ada-profile", "Ada Default", &first_form, 1_760_000_000);
        profile.aliases.insert(
            "applicant.full_name".to_owned(),
            vec!["Full Name".to_owned()],
        );

        let mut second_form = AcroForm::new();
        second_form.add_field(FormField::text("Full Name", ""));
        second_form.add_field(FormField::text("applicant_email", ""));
        let mut terms = FormField::checkbox("TERMS", "Yes", FieldValue::Boolean(false));
        terms.flags = terms.flags.with(FieldFlags::REQUIRED);
        second_form.add_field(terms);

        let report = apply_profile(&profile, &mut second_form, ProfileMatchMode::NormalizedName)
            .expect("apply profile");
        assert_eq!(report.matched_fields.len(), 3);
        assert_eq!(
            second_form
                .field("Full Name")
                .expect("full name")
                .value
                .as_display_string(),
            "Ada Lovelace"
        );
        assert_eq!(
            second_form
                .field("applicant_email")
                .expect("email")
                .value
                .as_display_string(),
            "ada@analytical.org"
        );
        assert_eq!(
            second_form.field("TERMS").expect("terms").value,
            FieldValue::Boolean(true)
        );

        // SQLite persistence round-trip
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        let mut store = ProfileStore::new();
        store.save(profile.clone());
        store.save_to_sqlite(&conn).expect("save sqlite");
        let loaded = ProfileStore::load_from_sqlite(&conn).expect("load sqlite");
        assert_eq!(loaded.profile("ada-profile"), Some(&profile));

        let _unused: BTreeMap<String, String> = BTreeMap::new();
    }

    #[test]
    fn form_session_navigates_tabs_highlights_and_resets() {
        let form = sample_form();
        let mut session = FormSession::new(form);
        assert_eq!(session.active_field(), Some("applicant.full_name"));
        assert!(session.next_tab_field().is_some());
        assert!(session.previous_tab_field().is_some());

        session
            .set_field("applicant.full_name", FieldValue::None)
            .expect("clear required field");
        let validation = session.validate();
        assert!(!validation.is_valid());
        assert!(session.dirty_fields().contains("applicant.full_name"));
    }
}
