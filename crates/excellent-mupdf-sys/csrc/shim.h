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
 * (otherwise on white). */
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
} ex_annot_props;

/* The texts an annotation carries, for ex_annot_text: /Contents, /NM, /T (author) and the app's private key
 * (/ExcellentPDF, a text string holding the anchor and tags). */
#define EX_ANNOT_CONTENTS 0
#define EX_ANNOT_ID 1
#define EX_ANNOT_AUTHOR 2
#define EX_ANNOT_APP_DATA 3

/* Reads `annot`: its properties, then its arrays into caller memory sized from them (any may be NULL to skip). */
int ex_annot_props_of(fz_context *ctx, pdf_annot *annot, ex_annot_props *out, ex_error *err);
int ex_annot_arrays(fz_context *ctx, pdf_annot *annot, fz_quad *quads, int *stroke_lengths, fz_point *points,
	ex_error *err);
/* One of the EX_ANNOT_* texts, copied into a new buffer; empty when the annotation has none. */
int ex_annot_text(fz_context *ctx, pdf_annot *annot, int which, fz_buffer **out, ex_error *err);

/* An annotation as it is written: `props` (its type is used only to create; the arrays come from the pointers
 * here, sized by its counts) and its texts, UTF-8. `id` names it. A NULL text writes none; an empty app data
 * removes the key. Writing every property keeps the dictionary's other keys (an appearance's own, another app's). */
typedef struct ex_annot_write {
	const char *id;
	const char *author;
	const char *contents;
	const char *app_data;
	ex_annot_props props;
	const fz_quad *quads;
	const int *stroke_lengths;
	const fz_point *points;
} ex_annot_write;

/* Annotation edits (K.14), inside an operation, on page `page` (zero-based) of a PDF. ex_annot_create adds one;
 * ex_annot_update writes every property of the one whose /NM is `id`; ex_annot_delete removes it. The appearance is
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
 * is FZ_OUTLINE_FLAG_* and `r g b` its colour (0 to 1); an update keeps the entry's own flags and colour. */
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

/* Buffers: `*data` points into the buffer and is valid until it is dropped. */
void ex_buffer_data(fz_context *ctx, fz_buffer *buf, const unsigned char **data, size_t *len);
void ex_drop_buffer(fz_context *ctx, fz_buffer *buf);

#ifdef __cplusplus
}
#endif

#endif
