# Ph1: 55/60 | O.22: Print

**Tag:** O.22; **Priority:** P0; **Phase:** 1 (Windows), 2 (macOS, Linux); **Needs:** K.19; **Code:**
`excellent-core::print`, `excellent-platform::print`, `excellent-app::print`
**Plan:** §27 entry `Ph1: 55/60 | O.22`.

## Problem

A reader can't put the document on paper. Every reader prints, and SumatraPDF users expect Ctrl+P.

## User story

As a reader, I want to print the pages I choose, sized to my paper, and stop a long job if I change my mind.

## UI sketch

- **Ctrl+P** (palette: *Print…*) opens the system print dialog over the window: printer, pages (all, the current
  page, or ranges such as "1-3, 7"), copies and collation; the printer's **Preferences** has duplex and colour.
- Printing runs on its own thread. A strip at the window's foot says "Printing report.pdf: page 3 of 12" with a
  progress bar and **Cancel printing**; then "Sent 12 pages of report.pdf to the printer" for a few seconds, or why it
  couldn't print, with **Dismiss**. Screen readers hear how it ended.
- **Settings, Printing:** **Page size on paper** (*Shrink to fit*, the default; *Fit to paper*; *Actual size*) and
  **Print annotations** (on by default; form fields always print).
- A landscape page on portrait paper (or the other way) is turned to match, and every page is centred.

## Mode

All modes, while a document is shown.

## Speed and size impact

- Startup: nothing; the dialog and the job exist only after Ctrl+P.
- Pages render at the printer's own resolution in bands of at most 16 MB, one page at a time, so memory stays bounded
  at 600 or 1200 dpi. The UI thread only reads the job's progress five times a second. Time per page is **not
  measured** (K.24).
- Nothing to the installer (GDI and the common dialogs are part of Windows).

## Design

- Model (`excellent-core::print`, no OS types): `Scaling`, `Paper`, `placement` (the page's map onto the printable
  area: fit, shrink or actual, turned to the paper's shape, centred, clipped), `bands`, `pages_in` (the dialog's
  ranges to pages from 0), `Job`, `Progress` (pages sent; cancel through a MuPDF cookie, so a page being drawn stops
  at once), the `Sheets` trait the printer is drawn through, and `print`, which runs a job on the calling thread.
- MuPDF: a page without its annotations is recorded by `ex_new_contents_display_list` (`fz_run_page_contents` and
  `fz_run_page_widgets`), new in the shim. With annotations on, the views' display list is reused when there is one.
- Platform (`excellent-platform::print`, Windows): `ask` shows `PrintDlgExW` (with `PD_RETURNDC` and the driver's
  own copies and collation) on the calling thread; `Printer` wraps the printer's device context: `StartDocW`, a
  `StretchDIBits` per band (RGB to a top-down 24-bit DIB), `EndPage`, `EndDoc`. Dropped before it finishes, it calls
  `AbortDoc`. This is how SumatraPDF prints, minus its own property page.
- App (`excellent-app::print`): `Printing` per window (status, file name, progress), the `file.print` command, and
  the strip. The dialog and the job each run on a blocking thread (`blocking`), as NSH runs its file dialogs, so the
  window's message loop never runs inside an app callback. The registry's document prints, so edits not saved yet
  print too.
- Platform, macOS and Linux (Phase 2): the system dialog runs on the main thread while the job's thread waits for it
  (`NSPrintPanel` over the shared `NSPrintInfo`; GTK 4.14's `GtkPrintDialog`, the print portal where there is one).
  The bands go to a spool (`print::spool`): a PDF of image pages at the paper's size, written as the bands come, one
  Flate-packed image per band. Finishing sends it to the chosen printer with the chosen settings
  (`PMPrinterPrintWithFile`; `gtk_print_dialog_print_file`), the page range reset to all, since the spool holds just
  the chosen pages. Resolution: the printer's (at most 600 dpi), else 300.
- Not built: scaling and annotations in the
  dialog itself (they are settings, DD6); "print as image" as a choice (every page prints as an image already, the
  way SumatraPDF prints); the view's own rotation (R.4) is not applied, so a page prints as the file has it; no
  toolbar button yet (there is no toolbar).

## Acceptance tests

- [x] `a_page_is_fit_shrunk_or_kept_at_its_size_and_centred`
- [x] `a_landscape_page_turns_onto_portrait_paper`
- [x] `bands_cover_the_area_once_each_within_the_size`
- [x] `page_ranges_from_the_dialog_become_pages_from_zero`
- [x] `a_job_draws_each_page_in_bands_and_stops_when_cancelled`
- [x] `only_the_dialog_and_the_job_are_busy`
- [x] `every_object_starts_where_the_table_says`: the spool's cross-reference table is right.
- [x] `a_band_keeps_its_pixels_and_lands_below_the_margin`: a band keeps its pixels and its place on the paper.
- [x] `the_area_is_the_printable_part_in_pixels`
- [ ] printing works on all three desktops before the first public build (Appendix C.4): **not tried on a printer
  yet** on Windows, macOS or Linux
