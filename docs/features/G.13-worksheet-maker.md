# Ph7: 10/32 | G.13: Worksheet maker

**Tag:** G.13; **Priority:** P1; **Phase:** 7; **Needs:** O.11, A.22; **Code:** `ecclesia-features::grading` (`WorksheetSpec`, `build_worksheet_pdf`, `export_student_copy`)
**Plan:** §27 entry `#### Ph7: 10/32 | G.13`.

## Problem

Teachers creating quizzes or problem sets need to assemble questions with fillable form fields (`/Tx`, `/Btn`, `/Ch`), keep an answer key on a toggleable OCG layer while authoring, and export a clean student copy with answers stripped.

## User story

As a teacher building an interactive worksheet, I want to author fillable questions and an AnswerKey layer in one PDF, then export a student PDF with zero answer content leaked.

## UI sketch

- Worksheet builder panel for text boxes, checkboxes, and multiple-choice dropdowns plus an "Answer Key" visibility toggle and "Export Student Copy" button.

## Mode

Create / Grading modes.

## Speed and size impact

- Pure Rust PDF `/AcroForm` + `/OCProperties` generation on top of MuPDF HTML layout (`build_worksheet_pdf`).

## Design

- **`WorksheetSpec` / `WorksheetQuestion` (`QuestionKind::ShortAnswer | Checkbox | MultipleChoice`)**: Defines prompt text, answer key string, points, and interactive AcroForm widget geometry.
- **`build_worksheet_pdf` & `export_student_copy`**: Emits both the AcroForm fields and a dedicated `"AnswerKey"` OCG layer (`/OCProperties` `/OCGs`), then strips `"AnswerKey"` content streams and field `/V` answer values cleanly when generating student copies.

## Acceptance tests

- [x] `worksheet_maker_builds_fillable_fields_and_strips_answers_for_students`: builds a worksheet with `/Tx`/`/Btn`/`/Ch` AcroForm fields and an `"AnswerKey"` OCG, verifies answer text appears in the teacher copy, and proves student export retains fillable widgets while containing zero answer strings.
