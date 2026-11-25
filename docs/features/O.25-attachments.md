# Ph3: 48/49 | O.25: Attachments

**Tag:** O.25; **Priority:** P1; **Phase:** 3; **Needs:** K.3, K.23; **Code:** `excellent-features::attachments`
(safe names, what may open), `excellent-mupdf` (`Document::attachments`, `attachment_contents`,
`Edit::add_attachment`, the shim), `excellent-core::edit` (`Change::AddAttachment`), `excellent-app::attachments`
(the dialog); MuPDF patch `0095`
**Plan:** §14.2 and §27 entry `#### Ph3: 48/49 | O.25`.

## Problem

PDFs carry files: a spreadsheet behind a report's tables, a form's data, a page's attached source. They couldn't be
seen, saved or added.

## User story

As an accountant reading a report, I want to save the spreadsheet attached to it, and attach my own reply file
before sending the PDF back, without any risk that an attachment runs something on my computer.

## UI sketch

Attachments on the Read toolbar (Ctrl+Alt+A, the palette): a dialog lists each attachment, its name, where it is
(in the document, or on page N) and its size, with Save… and, for a kind that may open, Open. At the bottom: Add
file… (on an editable PDF), Save all… and Close. Open asks first ("Open name?"); a program or script is never opened,
only saved, and says why.

## Mode

Every mode.

## Speed and size impact

Nothing until the dialog opens. Listing reads every page's annotations on a worker; reading and writing files run on
workers too.

## Design

- **Listed:** the document's own embedded files (`/EmbeddedFiles`, loaded with `pdf_load_name_tree`, in name order),
  then each page's file attachment annotations (`pdf_annot_filespec`), with their names and sizes from
  `pdf_get_filespec_params`. A file spec that points outside the PDF is listed but can't be read.
- **Names are untrusted:** before anything is written, a name keeps only its last path part, loses characters any
  desktop forbids or reads as a path, leading and trailing dots and spaces, and Windows device names (CON, NUL, COM1
  and on), and is cut to 150 characters with its extension kept. Save all writes each file inside the chosen folder,
  numbering a name already there; nothing is overwritten.
- **Open, never run:** a file opens in the system's app for its kind after the reader agrees, from a copy in the
  temporary folder; executables, installers, scripts, shortcuts and HTML (33 kinds) and files with no extension are
  never opened.
- **Add:** `pdf_add_embedded_file` (with a checksum) put in `/EmbeddedFiles` in name order, one undoable operation
  whose bytes the operation log keeps. MuPDF 1.28.5 left the journal inside an operation after an embed, so undo
  refused; patch `0095` ends it.

## Acceptance tests

- [x] Names with `../` can't write outside the chosen folder:
  `names_with_dot_dot_cannot_write_outside_the_chosen_folder` (`../`, `..\`, absolute and drive paths, UNC paths,
  `..` and `.` alone, a NUL, `....//`).
- [x] `an_embedded_file_is_listed_saved_and_undone` (a real PDF: two files listed in name order, read back byte for byte
  from the saved file, one undo takes the last out), `names_are_kept_where_they_are_safe`, `a_used_name_gets_a_number`,
  `programs_are_never_opened`.

## Open

- Adding a file attachment annotation on a page (`pdf_set_annot_filespec`) is not built; files are added to the
  document's own list.
- Removing an attachment is not built.
- A name tree split into `/Kids` gets the new file as a leaf of its own, at the end.
