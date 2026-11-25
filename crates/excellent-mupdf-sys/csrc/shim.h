/* ExcellentPDF's C shim around MuPDF (plan §6.3, ADR 0004).
 *
 * Rust never calls MuPDF directly: MuPDF reports errors with setjmp / longjmp (fz_try / fz_catch), and a longjmp
 * must not cross a Rust frame. Every function here runs its MuPDF calls inside fz_try, catches every error, and
 * reports it as a return code plus a message.
 *
 * Conventions:
 * - Functions that can fail return 0 on success or an FZ_ERROR_* code, and fill `err` (if not NULL) on failure.
 *   Their results go through out-parameters, which are left untouched on failure.
 * - ex_drop_* functions never fail and accept NULL.
 * - Every object belongs to the fz_context it was made with, or to a clone of it.
 */
#ifndef EXCELLENT_SHIM_H
#define EXCELLENT_SHIM_H

#include "mupdf/fitz.h"
#include "mupdf/pdf.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Error report filled by a failing call. The message is truncated to fit. */
typedef struct ex_error {
	int code;
	char message[256];
} ex_error;

/* Width, height and layout of a pixmap's samples. */
typedef struct ex_pixmap_info {
	int width;
	int height;
	/* Bytes per row of `samples`. */
	int stride;
	/* Components per pixel, including alpha. */
	int components;
	int alpha;
	/* Points into the pixmap; valid until it is dropped. */
	unsigned char *samples;
} ex_pixmap_info;

/* Returns the bytes of a built-in font by file name (for example "NimbusSans-Regular.cff"), or NULL. The bytes must
 * stay valid for the life of the process. */
typedef const unsigned char *(*ex_font_loader)(const char *file_name, int *size);

/* Sets the process-wide loader for MuPDF's built-in fonts. Without one, documents whose fonts are not embedded
 * render with whatever fallback MuPDF finds (none, on Windows, until system fonts are wired in). */
void ex_set_font_loader(ex_font_loader loader);

/* Contexts. ex_new_context makes a base context: the default store size, every document handler registered, and real
 * locks, so clones of it may be used on other threads at the same time (one clone per thread). Every context, base
 * or clone, is dropped with ex_drop_context; the locks go with the last one. */
int ex_new_context(fz_context **out, ex_error *err);
int ex_clone_context(fz_context *ctx, fz_context **out, ex_error *err);
void ex_drop_context(fz_context *ctx);

/* Documents. `path` is UTF-8. */
int ex_open_document(fz_context *ctx, const char *path, fz_document **out, ex_error *err);
/* Opens a document from `len` bytes at `data`, which are copied. `magic` is a file name or MIME type that names the
 * format ("application/pdf", "file.epub"). */
int ex_open_document_from_bytes(fz_context *ctx, const char *magic, const unsigned char *data, size_t len,
	fz_document **out, ex_error *err);
void ex_drop_document(fz_context *ctx, fz_document *doc);
int ex_count_pages(fz_context *ctx, fz_document *doc, int *out, ex_error *err);

/* Page `number`'s `box` (an fz_box_type) in page points, with the page's rotation applied: what ex_page_bounds
 * reports for the crop box. A PDF page is read from the page tree without loading the page (K.6: never load every
 * page to lay a document out); other formats load it. */
int ex_page_box(fz_context *ctx, fz_document *doc, int number, int box, fz_rect *out, ex_error *err);

/* 1 when the document lays its text out to a page size of the reader's choosing (EPUB, FB2, MOBI, HTML,
 * Markdown), else 0. */
int ex_is_reflowable(fz_context *ctx, fz_document *doc, int *out, ex_error *err);

/* Lays a reflowable document out on pages of `w` x `h` points with text of `em` points (R.28). When `keep` is a page
 * number, the text at the start of that page is found again after the layout: `out` gets the page it is on now, else
 * -1. Loaded pages of the document must be dropped first. */
int ex_reflow(fz_context *ctx, fz_document *doc, float w, float h, float em, int keep, int *out, ex_error *err);

/* 1 when the document asks to be read right to left (a PDF's /ViewerPreferences /Direction /R2L), else 0. */
int ex_reads_right_to_left(fz_context *ctx, fz_document *doc, int *out, ex_error *err);

/* 1 when the document numbers its pages its own way (a PDF's /PageLabels), else 0. */
int ex_has_page_labels(fz_context *ctx, fz_document *doc, int *out, ex_error *err);

/* Page `number`'s label ("iv", "A-3") into `buf` of `size` bytes, without loading the page; a document without
 * labels gives the page's number from 1. */
int ex_page_label(fz_context *ctx, fz_document *doc, int number, char *buf, int size, ex_error *err);

/* Pages. `number` is zero-based. Bounds are in page points, with the page's rotation applied. */
int ex_load_page(fz_context *ctx, fz_document *doc, int number, fz_page **out, ex_error *err);
void ex_drop_page(fz_context *ctx, fz_page *page);
int ex_page_bounds(fz_context *ctx, fz_page *page, fz_rect *out, ex_error *err);

/* Rendering: draws the page through `ctm` into a new RGB pixmap, with an alpha channel if `alpha` is non-zero
 * (otherwise on white). Replies in comment threads (A.21) are left out, as in ex_new_display_list. */
int ex_render_page(fz_context *ctx, fz_page *page, fz_matrix ctm, int alpha, fz_pixmap **out, ex_error *err);
void ex_pixmap_info_of(fz_context *ctx, fz_pixmap *pix, ex_pixmap_info *out);
void ex_drop_pixmap(fz_context *ctx, fz_pixmap *pix);

/* Display lists: a page recorded once, then rendered any number of times, from any thread (each with its own cloned
 * context). ex_render_display_list draws the part of the page that `ctm` maps into `area` (device pixels) into a new
 * RGB pixmap covering `area`, transparent if `alpha` is non-zero, otherwise on white. Wherever a `cookie` is taken it
 * may be NULL; setting its `abort` from another thread stops the work early, and the result is then incomplete, not an
 * error: the caller checks `abort` afterwards. */
int ex_new_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out, ex_error *err);
/* The same without the page's annotations, form fields kept: what printing with "print annotations" off draws. */
int ex_new_contents_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out,
	ex_error *err);
/* The same as ex_new_display_list without the annotations whose /NM is one of the `count` strings of `hidden`: the
 * annotation list's "show only matching" (A.19). A page that is not a PDF's keeps all its annotations. Neither draws
 * the replies in a comment thread (A.21): readers show them in their comment pane, not on the page. */
int ex_new_display_list_without(fz_context *ctx, fz_page *page, const char *const *hidden, int count,
	fz_cookie *cookie, fz_display_list **out, ex_error *err);
void ex_drop_display_list(fz_context *ctx, fz_display_list *list);
int ex_display_list_bounds(fz_context *ctx, fz_display_list *list, fz_rect *out, ex_error *err);
int ex_render_display_list(fz_context *ctx, fz_display_list *list, fz_matrix ctm, fz_irect area, int alpha,
	fz_cookie *cookie, fz_pixmap **out, ex_error *err);

/* The images a display list draws (smart dark mode, R.14), in the order it draws them: where each lands in page
 * space and its own size in pixels. Each `image` is a kept reference: drop it with ex_drop_image, then the array with
 * ex_drop_page_images. At most EX_MAX_PAGE_IMAGES are listed; the rest of the page's images are left out. */
#define EX_MAX_PAGE_IMAGES 4096
typedef struct ex_page_image {
	fz_rect bbox;
	int width;
	int height;
	fz_image *image;
} ex_page_image;
int ex_display_list_images(fz_context *ctx, fz_display_list *list, fz_cookie *cookie, ex_page_image **out,
	int *count, ex_error *err);
void ex_drop_page_images(fz_context *ctx, ex_page_image *images);
void ex_drop_image(fz_context *ctx, fz_image *image);

/* An RGB copy of `image` (no alpha) for its colour statistics: decoded as if drawn 64 pixels across, which MuPDF
 * rounds up to its nearest power-of-two subsampling (none for an image already decoded), as SumatraPDF Enhanced
 * samples it. */
int ex_image_sample(fz_context *ctx, fz_image *image, fz_pixmap **out, ex_error *err);

/* A pixmap encoded as PNG. */
int ex_pixmap_png(fz_context *ctx, fz_pixmap *pix, fz_buffer **out, ex_error *err);

/* Structured text of a page, and its plain text as UTF-8. `flags` are fz_stext_options' (FZ_STEXT_*). `cookie` may
 * be NULL, as for display lists. */
int ex_new_stext_page(fz_context *ctx, fz_page *page, int flags, fz_cookie *cookie, fz_stext_page **out,
	ex_error *err);
void ex_drop_stext_page(fz_context *ctx, fz_stext_page *stext);
int ex_stext_page_text(fz_context *ctx, fz_stext_page *stext, fz_buffer **out, ex_error *err);

/* The text of a structured-text page as flat arrays (K.13): its text blocks in MuPDF's order (structure blocks walked
 * depth first; image, vector and grid blocks left out), their lines and their characters. Count them with
 * ex_stext_counts, then copy them into arrays of those sizes with ex_stext_copy. Neither allocates or throws. */
typedef struct ex_text_block {
	fz_rect bbox;
	int first_line;
	int line_count;
} ex_text_block;

typedef struct ex_text_line {
	fz_rect bbox;
	/* The baseline's direction, normalized. */
	fz_point dir;
	/* 0 horizontal, 1 vertical. */
	int wmode;
	/* 1 when FZ_STEXT_DEHYPHENATE joined it to the next line: its trailing hyphen and line break aren't text. */
	int joined;
	/* Index of its block. */
	int block;
	int first_char;
	int char_count;
} ex_text_line;

typedef struct ex_text_char {
	/* Unicode scalar value. */
	int c;
	/* fz_stext_char's flags (FZ_STEXT_BOLD, ...). */
	int flags;
	/* Its area, in the page's coordinates (those of ex_page_bounds). */
	fz_quad quad;
	float size;
} ex_text_char;

void ex_stext_counts(fz_stext_page *stext, int *blocks, int *lines, int *chars);
void ex_stext_copy(fz_stext_page *stext, ex_text_block *blocks, ex_text_line *lines, ex_text_char *chars);

/* Outline (table of contents). `*out` is NULL when the document has none. Walk it with ex_outline_item_of; drop the
 * first entry only, which drops the whole tree. */
typedef struct ex_outline_item {
	/* UTF-8, or NULL. Valid until the outline is dropped. */
	const char *title;
	const char *uri;
	/* Zero-based page number, or -1 when the entry points nowhere in this document. */
	int page;
	int is_open;
	fz_outline *next;
	fz_outline *down;
	/* The PDF's /F bits: 1 italic, 2 bold (the PDF spec's order; MuPDF's FZ_OUTLINE_FLAG_* names have them the other
	 * way round). */
	int flags;
	/* Its /C as 0xRRGGBB, or -1 when it has none (black counts as none, as in SumatraPDF Enhanced). */
	int color;
} ex_outline_item;

int ex_load_outline(fz_context *ctx, fz_document *doc, fz_outline **out, ex_error *err);
int ex_outline_item_of(fz_context *ctx, fz_document *doc, fz_outline *outline, ex_outline_item *out, ex_error *err);
void ex_drop_outline(fz_context *ctx, fz_outline *outline);

/* Links of a page (R.7). `*out` is NULL when the page has none. Walk them with ex_link_item_of; drop the first link
 * only, which drops the whole chain. */
typedef struct ex_link_item {
	/* The active area, in the page's coordinates (those of ex_page_bounds). */
	fz_rect rect;
	/* UTF-8, never NULL. Valid until the links are dropped. */
	const char *uri;
	/* For a link inside the document: the zero-based page, and the target point on it (NAN when the link names only
	 * the page). -1 for an external link, or one whose target can't be found. */
	int page;
	float x;
	float y;
	fz_link *next;
} ex_link_item;

int ex_load_links(fz_context *ctx, fz_page *page, fz_link **out, ex_error *err);
int ex_link_item_of(fz_context *ctx, fz_page *page, fz_link *link, ex_link_item *out, ex_error *err);
void ex_drop_links(fz_context *ctx, fz_link *link);

/* PDF annotations of a page, in order. `*out` is NULL after the last one, or at once when the page is not a PDF page.
 * Each returned annotation is a new reference: drop it with ex_drop_annot. `type` is an `enum pdf_annot_type`; the
 * contents are copied into a new buffer. */
int ex_first_annot(fz_context *ctx, fz_page *page, pdf_annot **out, ex_error *err);
int ex_next_annot(fz_context *ctx, pdf_annot *annot, pdf_annot **out, ex_error *err);
int ex_annot_info(fz_context *ctx, pdf_annot *annot, int *type, fz_rect *rect, ex_error *err);
int ex_annot_contents(fz_context *ctx, pdf_annot *annot, fz_buffer **out, ex_error *err);
void ex_drop_annot(fz_context *ctx, pdf_annot *annot);

/* An annotation's properties as the annotation model (K.14) reads and writes them, in page space (that of
 * ex_page_bounds). The arrays are counted: `quads` for text markup, `stroke_lengths` (points per stroke) and `points`
 * for ink, `points` for a line (two) and a polygon's or polyline's vertices. Dates are seconds since 1970 (UTC). */
typedef struct ex_annot_props {
	/* enum pdf_annot_type */
	int type;
	fz_rect rect;
	/* 0 for none (transparent), 1 gray, 3 RGB, 4 CMYK. */
	int color_n;
	float color[4];
	float opacity;
	float border;
	int64_t created;
	int64_t modified;
	int quad_count;
	int stroke_count;
	int point_count;
	/* Whether its note shows open in other readers (/Open: a sticky note's own, else its popup's), A.5. */
	int is_open;
	/* A text box's (A.6): its enum pdf_intent (a typewriter's, a callout's), its alignment (/Q: 0 left, 1 centred,
	 * 2 right), and its text's size and colour (/DA; colour components as `color`). Zero for the other kinds. */
	int intent;
	int quadding;
	float font_size;
	int text_color_n;
	float text_color[4];
	/* Line endings (enum pdf_line_ending): a line's, polyline's or polygon's start and end (/LE); a callout's arrow
	 * at its tip in `line_start` (A.7, A.8). */
	int line_start;
	int line_end;
	/* A callout's leader line (/CL), its tip first: 0, 2 or 3 points (A.7). Written from its tip alone: MuPDF draws
	 * the rest from the box. */
	int callout_count;
	fz_point callout[3];
	/* A shape's fill (/IC; components as `color`, 0 for none) and its border's dash pattern (/BS /D, used when /S
	 * is /D; 0 items for a solid border), A.9. */
	int fill_n;
	float fill[4];
	int dash_count;
	float dash[8];
	/* Whether a closed shape's border is cloudy (/BE /S /C), and how much (/BE /I, 0 to 2), A.10. */
	int cloudy;
	float cloud_intensity;
	/* Whether it is locked (/F bit 8, Locked: not to be moved, resized, deleted or restyled; its note can still be
	 * written), A.12. In the padding after `cloud_intensity`, so the struct's size is as before. */
	int locked;
	/* Whether it blends Multiply (PDF 2.0's /BM /Multiply): a highlighter pen's ink, H.3. */
	int multiply;
} ex_annot_props;

/* The texts an annotation carries, for ex_annot_text: /Contents, /NM, /T (author), the app's private key
 * (/ExcellentPDF, a text string holding the anchor and tags), and the /NM of the annotation it is grouped with
 * (its /IRT, when its /RT is /Group: a "replace text" pair, A.2). */
#define EX_ANNOT_CONTENTS 0
#define EX_ANNOT_ID 1
#define EX_ANNOT_AUTHOR 2
#define EX_ANNOT_APP_DATA 3
#define EX_ANNOT_GROUP 4
/* Its icon's name (/Name: a sticky note's, a stamp's), as the file has it: empty when it names none (A.5). */
#define EX_ANNOT_ICON 5
/* A text box's font (/DA), as MuPDF names the standard ones: Helv, TiRo, Cour (A.6). */
#define EX_ANNOT_FONT 6
/* The /NM of the annotation it answers: its /IRT when its /RT is not /Group (a reply in a comment thread, A.21). */
#define EX_ANNOT_REPLY_TO 7
/* Its review state's name (/State: Accepted, Rejected, Cancelled, Completed, None, Marked, Unmarked), as the file has
 * it (A.21). */
#define EX_ANNOT_STATE 8
/* The name of the layer it is on (/OC, when that is an optional content group: A.22); empty for none. */
#define EX_ANNOT_LAYER 9

/* Reads `annot`: its properties, then its arrays into caller memory sized from them (any may be NULL to skip). */
int ex_annot_props_of(fz_context *ctx, pdf_annot *annot, ex_annot_props *out, ex_error *err);
int ex_annot_arrays(fz_context *ctx, pdf_annot *annot, fz_quad *quads, int *stroke_lengths, fz_point *points,
	ex_error *err);
/* An ink annotation's point widths (H.9, the private /ExcellentWidths key): `*count` of them, copied into `widths`
 * when it is not NULL, at most `capacity`; 0 when it has none. */
int ex_annot_ink_widths(fz_context *ctx, pdf_annot *annot, float *widths, int capacity, int *count, ex_error *err);
/* One of the EX_ANNOT_* texts, copied into a new buffer; empty when the annotation has none. */
int ex_annot_text(fz_context *ctx, pdf_annot *annot, int which, fz_buffer **out, ex_error *err);

/* An annotation as it is written: `props` (its type is used only to create; the arrays come from the pointers
 * here, sized by its counts) and its texts, UTF-8. `id` names it. A NULL text writes none; an empty app data
 * removes the key. `group` is the /NM of an annotation on the same page to group it with (/IRT and /RT /Group);
 * NULL or empty ungroups it, leaving a reply (an /IRT without /RT /Group) as it is. Writing every property keeps the
 * dictionary's other keys (an appearance's own, another app's). */
typedef struct ex_annot_write {
	const char *id;
	const char *author;
	const char *contents;
	const char *app_data;
	ex_annot_props props;
	const fz_quad *quads;
	const int *stroke_lengths;
	const fz_point *points;
	const char *group;
	/* The icon's name for the kinds that have one; NULL or empty removes it, leaving the kind's default. */
	const char *icon;
	/* A text box's font, Helv, TiRo or Cour; NULL or empty for Helv. */
	const char *font;
	/* The /NM of the annotation on its page it answers (A.21); NULL or empty for none. Not written with a group. */
	const char *reply_to;
	/* A reply's review state (/State, see EX_ANNOT_STATE); NULL or empty for none. */
	const char *state;
	/* The name of the layer to put it on (A.22), made when the document has none of that name; NULL or empty for
	 * none. */
	const char *layer;
	/* Ink (H.9): each point's width, `width_count` of them in the order of `points` (0 for none, which removes the
	 * private /ExcellentWidths key); and each stroke's outline, filled in the annotation's colour, opacity and blend
	 * as its appearance in place of MuPDF's constant-width line: `outline_count` closed rings of `outline_lengths`
	 * points each, page space (0 keeps MuPDF's appearance). */
	const float *widths;
	int width_count;
	const int *outline_lengths;
	const fz_point *outline;
	int outline_count;
} ex_annot_write;

/* Annotation edits (K.14), inside an operation, on page `page` (zero-based) of a PDF. ex_annot_create adds one;
 * ex_annot_update writes every property of the one whose /NM is `id`; ex_annot_delete removes it and the replies in
 * its comment thread (A.21), so undoing the operation brings them all back. The appearance is
 * rebuilt after each change. Fails with FZ_ERROR_ARGUMENT when no annotation on the page has the id. */
int ex_annot_create(fz_context *ctx, fz_document *doc, int page, const ex_annot_write *annot, ex_error *err);
int ex_annot_update(fz_context *ctx, fz_document *doc, int page, const ex_annot_write *annot, ex_error *err);
int ex_annot_delete(fz_context *ctx, fz_document *doc, int page, const char *id, ex_error *err);

/* Document writers: `format` is a MuPDF output format ("pdf", "png", "cbz", "svg", "text", ...), or NULL to pick one
 * from the extension of `path`; `options` are MuPDF's writer options, or NULL. Each page is written at its own size.
 * Close the writer to finish the file; dropping it without closing abandons the output. */
int ex_new_document_writer(fz_context *ctx, const char *path, const char *format, const char *options,
	fz_document_writer **out, ex_error *err);
int ex_write_page(fz_context *ctx, fz_document_writer *wri, fz_page *page, ex_error *err);
int ex_close_document_writer(fz_context *ctx, fz_document_writer *wri, ex_error *err);
void ex_drop_document_writer(fz_context *ctx, fz_document_writer *wri);

/* Opens the PDF at `path` (UTF-8) for reading and editing. On Windows the file is opened so that it may be renamed or
 * replaced while it is open (FILE_SHARE_DELETE): a save writes a new file beside it and moves it over this one
 * (K.15), and the open document goes on reading the bytes it opened. */
int ex_open_pdf(fz_context *ctx, const char *path, fz_document **out, ex_error *err);
/* 1 when the document is a PDF, which alone can be edited and saved. */
int ex_is_pdf(fz_context *ctx, fz_document *doc);

/* Edits (K.14). Every change to a PDF runs inside an operation, which is one step of undo; ex_begin_operation turns
 * the journal on the first time. End an operation that worked; abandon one that failed, which takes back what it
 * changed. Fails with FZ_ERROR_ARGUMENT when the document is not a PDF. */
int ex_begin_operation(fz_context *ctx, fz_document *doc, const char *name, ex_error *err);
int ex_end_operation(fz_context *ctx, fz_document *doc, ex_error *err);
void ex_abandon_operation(fz_context *ctx, fz_document *doc);
/* Where the document is in its undo history: `*position` of `*steps` operations are applied (0 when nothing was ever
 * edited). */
int ex_undo_state(fz_context *ctx, fz_document *doc, int *position, int *steps, ex_error *err);
int ex_undo(fz_context *ctx, fz_document *doc, ex_error *err);
int ex_redo(fz_context *ctx, fz_document *doc, ex_error *err);

/* An outline entry as it is written. `title` and `uri` are UTF-8; NULL in an update keeps the entry's own. `flags`
 * is the PDF's /F bits (as in ex_outline_item) and `r g b` its colour (0 to 1); an update keeps the entry's own flags
 * and colour (ex_outline_style changes them). */
typedef struct ex_outline_edit {
	const char *title;
	const char *uri;
	int is_open;
	int flags;
	float r;
	float g;
	float b;
} ex_outline_edit;

/* Outline edits (B.1-B.5), inside an operation. An entry is named by its path: its index on each level from the top,
 * `depth` long. ex_outline_insert puts a new entry before the one at `path`, which may also be one past the last entry
 * of its level; the first insert into a document without an outline makes one. Deleting an entry deletes the entries
 * under it. ex_outline_move takes the entry at `from` with everything under it, and puts it at `to` as the path is
 * after the entry left (flags and colours go with it). */
int ex_outline_insert(fz_context *ctx, fz_document *doc, const int *path, int depth, const ex_outline_edit *item,
	ex_error *err);
int ex_outline_update(fz_context *ctx, fz_document *doc, const int *path, int depth, const ex_outline_edit *item,
	ex_error *err);
int ex_outline_delete(fz_context *ctx, fz_document *doc, const int *path, int depth, ex_error *err);
/* B.7: the entry at `path` drawn with `flags` (/F bits, as in ex_outline_item) in `color` (0xRRGGBB, or -1 for none);
 * everything else about it stays. */
int ex_outline_style(fz_context *ctx, fz_document *doc, const int *path, int depth, int flags, int color,
	ex_error *err);
int ex_outline_move(fz_context *ctx, fz_document *doc, const int *from, int from_depth, const int *to, int to_depth,
	ex_error *err);

/* The link to point (x, y) of page `page` (zero-based), in the page's coordinates, at `zoom` (1 = 100 %; 0 keeps the
 * reader's), as MuPDF names it: "#page=4&zoom=150,72,92" (a PDF outline stores it as an XYZ destination). Copied into
 * `buf`, NUL-terminated, cut at `size`. */
int ex_link_uri(fz_context *ctx, fz_document *doc, int page, float x, float y, float zoom, char *buf, int size,
	ex_error *err);

/* Saves (K.15). Writes the PDF with its edits to `path`, which must not be the file it was opened from: the opened
 * file's bytes, then the edits as one incremental update when the file allows that, else the whole document
 * rewritten. The document keeps its edits and its undo history, so later saves write every edit since it was opened
 * again. `*incremental` is 1 when the opened bytes start the new file unchanged, else 0. */
int ex_save_pdf(fz_context *ctx, fz_document *doc, const char *path, int *incremental, ex_error *err);

/* A.22: the document's layers (optional content groups, /OCProperties /OCGs) in the file's order, into a new
 * buffer: one entry each, a flags byte ('0' plus 1 when the default configuration /D hides it, plus 2 when it locks
 * it) then its name, UTF-8, ending in a NUL. Empty for a document without layers or not a PDF. */
int ex_layers(fz_context *ctx, fz_document *doc, fz_buffer **out, ex_error *err);
/* A.22, inside an operation: the layer named `name` shown or hidden, locked or not, in the default configuration
 * (/D /ON, /OFF, /Locked), which is what other readers open with. Fails with FZ_ERROR_ARGUMENT when the document has
 * no layer of that name. */
int ex_layer_set(fz_context *ctx, fz_document *doc, const char *name, int visible, int locked, ex_error *err);
/* Page `number`'s map from PDF space (the file's coordinates: points, y up) to page space, as MuPDF reads
 * annotations (A.24's XFDF writes PDF space). PDF only. */
int ex_page_transform(fz_context *ctx, fz_document *doc, int number, fz_matrix *out, ex_error *err);

/* The PDF's annotations as an FDF file (A.24), each with its /Page: all but links, popups and form fields, replies
 * pointing at their parents' copies. */
int ex_fdf_write(fz_context *ctx, fz_document *doc, fz_buffer **out, ex_error *err);

/* A new PDF with `doc`'s page count and page boxes and no content, holding the annotations of the FDF file opened as
 * `fdf`, each on its /Page: so they read as the document's would (A.24). Annotations for pages it lacks are left
 * out. */
int ex_fdf_read(fz_context *ctx, fz_document *doc, fz_document *fdf, fz_document **out, ex_error *err);

/* Writes a copy of the PDF, its edits included, to `path` with its annotations drawn into the page content and taken
 * out (A.25); its form fields too when `forms` is nonzero. The open document is left as it is. */
int ex_save_flattened(fz_context *ctx, fz_document *doc, const char *path, int forms, ex_error *err);

/* Buffers: `*data` points into the buffer and is valid until it is dropped. */
/* A.20: the stamp of page `page` whose /NM is `id` drawn as the `len` bytes of `art`: with `kind` 0 the first page
 * of a PDF (its appearance, kept by MuPDF because the stamp's name is not a standard one), with 1 an image
 * (pdf_set_annot_stamp_image). */
int ex_annot_set_stamp_art(fz_context *ctx, fz_document *doc, int page, const char *id, int kind,
	const unsigned char *art, size_t len, ex_error *err);

/* Attachments (O.25): the document's own embedded files (/EmbeddedFiles, in name order), then each page's file
 * attachment annotations, in page order; `index` counts them all. ex_attachment_info gives one's file name (UTF-8,
 * as the file names it: untrusted), its size in bytes (-1 when unknown) and its page (-1 for the document's own).
 * ex_attachment_add embeds `len` bytes as file `name` in /EmbeddedFiles (inside an operation). */
int ex_attachment_count(fz_context *ctx, fz_document *doc, int *count, ex_error *err);
int ex_attachment_info(fz_context *ctx, fz_document *doc, int index, fz_buffer **name, int *size, int *page,
	ex_error *err);
int ex_attachment_contents(fz_context *ctx, fz_document *doc, int index, fz_buffer **out, ex_error *err);
int ex_attachment_add(fz_context *ctx, fz_document *doc, const char *name, const unsigned char *data, size_t len,
	ex_error *err);
/* A.20: `page` written as a one-page PDF, into a new buffer. */
int ex_page_to_pdf(fz_context *ctx, fz_page *page, fz_buffer **out, ex_error *err);
/* A.20: the HTML `html`, styled by `css`, laid out `width` points wide and written as a one-page PDF as tall as it,
 * into a new buffer. */
int ex_html_to_pdf(fz_context *ctx, const char *html, const char *css, float width, fz_buffer **out,
	ex_error *err);
void ex_buffer_data(fz_context *ctx, fz_buffer *buf, const unsigned char **data, size_t *len);
void ex_drop_buffer(fz_context *ctx, fz_buffer *buf);

#ifdef __cplusplus
}
#endif

#endif
