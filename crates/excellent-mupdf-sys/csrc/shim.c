/* ExcellentPDF's C shim around MuPDF. See shim.h for the conventions. */

#include "shim.h"
#include "noto_sumatra.h"
#include "ofd-seal.h" /* ofd.h needs it first */
#include "ofd.h"
#include "mupdf/helpers/mu-threads.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Called inside fz_catch: takes the error from the context (clearing it) and reports it. */
static int ex_fail(fz_context *ctx, ex_error *err)
{
	int code = FZ_ERROR_GENERIC;
	const char *message = fz_convert_error(ctx, &code);
	if (code == FZ_ERROR_NONE)
		code = FZ_ERROR_GENERIC;
	if (err)
	{
		err->code = code;
		snprintf(err->message, sizeof err->message, "%s", message ? message : "");
	}
	return code;
}

static int ex_fail_message(int code, const char *message, ex_error *err)
{
	if (err)
	{
		err->code = code;
		snprintf(err->message, sizeof err->message, "%s", message);
	}
	return code;
}

void ex_set_font_loader(ex_font_loader loader)
{
	fz_set_builtin_font_loader(loader);
}

/* The locks a base context and all its clones share; freed with the last of them. `refs` is guarded by the
 * FZ_LOCK_ALLOC mutex. */
typedef struct ex_locks {
	mu_mutex mutex[FZ_LOCK_MAX];
	int refs;
} ex_locks;

static void ex_lock(void *user, int lock)
{
	mu_lock_mutex(&((ex_locks *)user)->mutex[lock]);
}

static void ex_unlock(void *user, int lock)
{
	mu_unlock_mutex(&((ex_locks *)user)->mutex[lock]);
}

static void ex_free_locks(ex_locks *locks, int count)
{
	int i;
	for (i = 0; i < count; i++)
		mu_destroy_mutex(&locks->mutex[i]);
	free(locks);
}

int ex_new_context(fz_context **out, ex_error *err)
{
	fz_locks_context lock_fns;
	fz_context *ctx;
	ex_locks *locks = calloc(1, sizeof *locks);
	int i;

	if (!locks)
		return ex_fail_message(FZ_ERROR_SYSTEM, "cannot allocate MuPDF locks", err);
	for (i = 0; i < FZ_LOCK_MAX; i++)
	{
		if (mu_create_mutex(&locks->mutex[i]))
		{
			ex_free_locks(locks, i);
			return ex_fail_message(FZ_ERROR_SYSTEM, "cannot create MuPDF locks", err);
		}
	}
	locks->refs = 1;
	lock_fns.user = locks;
	lock_fns.lock = ex_lock;
	lock_fns.unlock = ex_unlock;

	ctx = fz_new_context(NULL, &lock_fns, FZ_STORE_DEFAULT);
	if (!ctx)
	{
		ex_free_locks(locks, FZ_LOCK_MAX);
		return ex_fail_message(FZ_ERROR_SYSTEM, "cannot create MuPDF context", err);
	}
	fz_try(ctx)
	{
		fz_register_document_handlers(ctx);
		/* SumatraPDF's OFD reader (third_party/sumatra/shared/mupdf/ofd.c); MuPDF has none of its own. */
		fz_register_ofd_handler(ctx);
	}
	fz_catch(ctx)
	{
		int code = ex_fail(ctx, err);
		ex_drop_context(ctx);
		return code;
	}
	*out = ctx;
	return 0;
}

int ex_clone_context(fz_context *ctx, fz_context **out, ex_error *err)
{
	ex_locks *locks = ctx->locks.user;
	fz_context *clone;

	ex_lock(locks, FZ_LOCK_ALLOC);
	locks->refs++;
	ex_unlock(locks, FZ_LOCK_ALLOC);

	clone = fz_clone_context(ctx);
	if (!clone)
	{
		ex_lock(locks, FZ_LOCK_ALLOC);
		locks->refs--;
		ex_unlock(locks, FZ_LOCK_ALLOC);
		return ex_fail_message(FZ_ERROR_SYSTEM, "cannot clone MuPDF context", err);
	}
	*out = clone;
	return 0;
}

void ex_drop_context(fz_context *ctx)
{
	ex_locks *locks;
	int last;

	if (!ctx)
		return;
	locks = ctx->locks.user;
	fz_drop_context(ctx);
	ex_lock(locks, FZ_LOCK_ALLOC);
	last = --locks->refs == 0;
	ex_unlock(locks, FZ_LOCK_ALLOC);
	if (last)
		ex_free_locks(locks, FZ_LOCK_MAX);
}

int ex_open_document(fz_context *ctx, const char *path, fz_document **out, ex_error *err)
{
	fz_document *doc = NULL;
	fz_try(ctx)
		doc = fz_open_document(ctx, path);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = doc;
	return 0;
}

int ex_open_document_from_bytes(fz_context *ctx, const char *magic, const unsigned char *data, size_t len,
	fz_document **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_document *doc = NULL;

	fz_var(buf);
	fz_try(ctx)
	{
		buf = fz_new_buffer_from_copied_data(ctx, data, len);
		/* The document takes its own reference to the buffer. */
		doc = fz_open_document_with_buffer(ctx, magic, buf);
	}
	fz_always(ctx)
		fz_drop_buffer(ctx, buf);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = doc;
	return 0;
}

void ex_drop_document(fz_context *ctx, fz_document *doc)
{
	fz_drop_document(ctx, doc);
}

int ex_count_pages(fz_context *ctx, fz_document *doc, int *out, ex_error *err)
{
	int count = 0;
	fz_try(ctx)
		count = fz_count_pages(ctx, doc);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = count;
	return 0;
}

int ex_page_box(fz_context *ctx, fz_document *doc, int number, int box, fz_rect *out, ex_error *err)
{
	fz_rect rect = fz_empty_rect;
	fz_page *page = NULL;
	fz_var(page);
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		if (pdf)
		{
			/* As pdf_bound_page, from the page object alone. */
			fz_matrix ctm;
			pdf_obj *obj = pdf_lookup_page_obj(ctx, pdf, number);
			pdf_page_obj_transform_box(ctx, obj, &rect, &ctm, (fz_box_type)box);
			rect = fz_transform_rect(rect, ctm);
		}
		else
		{
			page = fz_load_page(ctx, doc, number);
			rect = fz_bound_page_box(ctx, page, (fz_box_type)box);
		}
	}
	fz_always(ctx)
		fz_drop_page(ctx, page);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = rect;
	return 0;
}

int ex_is_reflowable(fz_context *ctx, fz_document *doc, int *out, ex_error *err)
{
	int reflowable = 0;
	fz_try(ctx)
		reflowable = fz_is_document_reflowable(ctx, doc);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = reflowable;
	return 0;
}

int ex_reflow(fz_context *ctx, fz_document *doc, float w, float h, float em, int keep, int *out, ex_error *err)
{
	int page = -1;
	fz_try(ctx)
	{
		/* A bookmark names a place in the text, not a page, so it survives the layout. */
		fz_bookmark mark = 0;
		int marked = keep >= 0;
		if (marked)
			mark = fz_make_bookmark(ctx, doc, fz_location_from_page_number(ctx, doc, keep));
		fz_layout_document(ctx, doc, w, h, em);
		/* fz_layout_document only marks the layout stale, and the lookup doesn't redo it: an HTML-family
		 * bookmark would be read against the old layout. Counting the pages lays the document out. */
		fz_count_pages(ctx, doc);
		if (marked)
			page =fz_page_number_from_location(ctx, doc, fz_lookup_bookmark(ctx, doc, mark));
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = page;
	return 0;
}

int ex_reads_right_to_left(fz_context *ctx, fz_document *doc, int *out, ex_error *err)
{
	int rtl = 0;
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		if (pdf)
		{
			pdf_obj *direction = pdf_dict_getp(ctx, pdf_trailer(ctx, pdf), "Root/ViewerPreferences/Direction");
			rtl = strcmp(pdf_to_name(ctx, direction), "R2L") == 0;
		}
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = rtl;
	return 0;
}

int ex_has_page_labels(fz_context *ctx, fz_document *doc, int *out, ex_error *err)
{
	int has = 0;
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		if (pdf)
			has = pdf_dict_getp(ctx, pdf_trailer(ctx, pdf), "Root/PageLabels") != NULL;
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = has;
	return 0;
}

int ex_page_label(fz_context *ctx, fz_document *doc, int number, char *buf, int size, ex_error *err)
{
	if (size < 1)
		return 0;
	buf[0] = 0;
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		if (pdf)
			pdf_page_label(ctx, pdf, number, buf, (size_t)size);
		else
			fz_snprintf(buf, (size_t)size, "%d", number + 1);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_load_page(fz_context *ctx, fz_document *doc, int number, fz_page **out, ex_error *err)
{
	fz_page *page = NULL;
	fz_try(ctx)
		page = fz_load_page(ctx, doc, number);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = page;
	return 0;
}

void ex_drop_page(fz_context *ctx, fz_page *page)
{
	fz_drop_page(ctx, page);
}

int ex_page_bounds(fz_context *ctx, fz_page *page, fz_rect *out, ex_error *err)
{
	fz_rect bounds = fz_empty_rect;
	fz_try(ctx)
		bounds = fz_bound_page(ctx, page);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = bounds;
	return 0;
}

int ex_render_page(fz_context *ctx, fz_page *page, fz_matrix ctm, int alpha, fz_pixmap **out, ex_error *err)
{
	fz_pixmap *pix = NULL;
	fz_display_list *list = NULL;
	int failed = ex_new_display_list(ctx, page, NULL, &list, err);
	if (failed)
		return failed;
	fz_try(ctx)
		pix = fz_new_pixmap_from_display_list(ctx, list, ctm, fz_device_rgb(ctx), alpha);
	fz_always(ctx)
		fz_drop_display_list(ctx, list);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = pix;
	return 0;
}

void ex_pixmap_info_of(fz_context *ctx, fz_pixmap *pix, ex_pixmap_info *out)
{
	/* These accessors read fields and never throw. */
	out->width = fz_pixmap_width(ctx, pix);
	out->height = fz_pixmap_height(ctx, pix);
	out->stride = fz_pixmap_stride(ctx, pix);
	out->components = fz_pixmap_components(ctx, pix);
	out->alpha = fz_pixmap_alpha(ctx, pix);
	out->samples = fz_pixmap_samples(ctx, pix);
}

void ex_drop_pixmap(fz_context *ctx, fz_pixmap *pix)
{
	fz_drop_pixmap(ctx, pix);
}

/* Whether `obj`, an annotation's dictionary, is a reply in a comment thread (A.21): an /IRT that is not a group's
 * (/RT /Group, A.2). Readers show replies in their comment pane, not on the page. */
static int ex_is_reply(fz_context *ctx, pdf_obj *obj)
{
	return pdf_dict_get(ctx, obj, PDF_NAME(IRT)) && !pdf_name_eq(ctx, pdf_dict_get(ctx, obj, PDF_NAME(RT)), PDF_NAME(Group));
}

static int new_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out, ex_error *err)
{
	/* fz_new_display_list_from_page_contents plus the form fields, with a cookie. */
	fz_display_list *list = NULL;
	fz_device *dev = NULL;

	fz_var(list);
	fz_var(dev);
	fz_try(ctx)
	{
		list = fz_new_display_list(ctx, fz_bound_page(ctx, page));
		dev = fz_new_list_device(ctx, list);
		fz_run_page_contents(ctx, page, dev, fz_identity, cookie);
		fz_run_page_widgets(ctx, page, dev, fz_identity, cookie);
		fz_close_device(ctx, dev);
	}
	fz_always(ctx)
		fz_drop_device(ctx, dev);
	fz_catch(ctx)
	{
		fz_drop_display_list(ctx, list);
		return ex_fail(ctx, err);
	}
	*out = list;
	return 0;
}

int ex_new_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out, ex_error *err)
{
	return ex_new_display_list_without(ctx, page, NULL, 0, cookie, out, err);
}

int ex_new_contents_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out,
	ex_error *err)
{
	return new_display_list(ctx, page, cookie, out, err);
}

int ex_new_display_list_without(fz_context *ctx, fz_page *page, const char *const *hidden, int count,
	fz_cookie *cookie, fz_display_list **out, ex_error *err)
{
	/* fz_run_page's three steps, with the annotations run one by one as pdf_run_page_annots does. */
	fz_display_list *list = NULL;
	fz_device *dev = NULL;

	fz_var(list);
	fz_var(dev);
	fz_try(ctx)
	{
		pdf_page *pdf = pdf_page_from_fz_page(ctx, page);

		list = fz_new_display_list(ctx, fz_bound_page(ctx, page));
		dev = fz_new_list_device(ctx, list);
		fz_run_page_contents(ctx, page, dev, fz_identity, cookie);
		if (pdf)
		{
			pdf_annot *annot;

			for (annot = pdf_first_annot(ctx, pdf); annot; annot = pdf_next_annot(ctx, annot))
			{
				const char *id;
				int i, skip = ex_is_reply(ctx, pdf_annot_obj(ctx, annot));

				if (cookie && cookie->abort)
					break;
				id = pdf_dict_get_text_string(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(NM));
				for (i = 0; i < count && !skip; i++)
					skip = !strcmp(id, hidden[i]);
				if (!skip)
					pdf_run_annot(ctx, annot, dev, fz_identity, cookie);
			}
		}
		else
			fz_run_page_annots(ctx, page, dev, fz_identity, cookie);
		fz_run_page_widgets(ctx, page, dev, fz_identity, cookie);
		fz_close_device(ctx, dev);
	}
	fz_always(ctx)
		fz_drop_device(ctx, dev);
	fz_catch(ctx)
	{
		fz_drop_display_list(ctx, list);
		return ex_fail(ctx, err);
	}
	*out = list;
	return 0;
}

void ex_drop_display_list(fz_context *ctx, fz_display_list *list)
{
	fz_drop_display_list(ctx, list);
}

int ex_display_list_bounds(fz_context *ctx, fz_display_list *list, fz_rect *out, ex_error *err)
{
	fz_rect bounds = fz_empty_rect;
	fz_try(ctx)
		bounds = fz_bound_display_list(ctx, list);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = bounds;
	return 0;
}

int ex_render_display_list(fz_context *ctx, fz_display_list *list, fz_matrix ctm, fz_irect area, int alpha,
	fz_cookie *cookie, fz_pixmap **out, ex_error *err)
{
	fz_pixmap *pix = NULL;
	fz_device *dev = NULL;

	fz_var(pix);
	fz_var(dev);
	fz_try(ctx)
	{
		pix = fz_new_pixmap_with_bbox(ctx, fz_device_rgb(ctx), area, NULL, alpha);
		if (alpha)
			fz_clear_pixmap(ctx, pix);
		else
			fz_clear_pixmap_with_value(ctx, pix, 0xFF);
		dev = fz_new_draw_device(ctx, fz_identity, pix);
		fz_run_display_list(ctx, list, dev, ctm, fz_rect_from_irect(area), cookie);
		fz_close_device(ctx, dev);
	}
	fz_always(ctx)
		fz_drop_device(ctx, dev);
	fz_catch(ctx)
	{
		fz_drop_pixmap(ctx, pix);
		return ex_fail(ctx, err);
	}
	*out = pix;
	return 0;
}

/* A device that only lists the images drawn through it. */
typedef struct ex_image_device {
	fz_device super;
	ex_page_image *items;
	int count;
	int cap;
} ex_image_device;

static void ex_list_image(fz_context *ctx, fz_device *dev_, fz_image *image, fz_matrix ctm, float alpha,
	fz_color_params color_params)
{
	ex_image_device *dev = (ex_image_device *)dev_;
	ex_page_image *item;
	(void)alpha;
	(void)color_params;
	if (dev->count >= EX_MAX_PAGE_IMAGES)
		return;
	if (dev->count == dev->cap)
	{
		int cap = dev->cap ? dev->cap * 2 : 8;
		dev->items = fz_realloc_array(ctx, dev->items, cap, ex_page_image);
		dev->cap = cap;
	}
	item = &dev->items[dev->count];
	item->bbox = fz_transform_rect(fz_unit_rect, ctm);
	item->width = image->w;
	item->height = image->h;
	item->image = fz_keep_image(ctx, image);
	dev->count++;
}

int ex_display_list_images(fz_context *ctx, fz_display_list *list, fz_cookie *cookie, ex_page_image **out,
	int *count, ex_error *err)
{
	ex_image_device *dev = NULL;
	ex_page_image *items = NULL;
	int n = 0;
	int i;

	fz_var(dev);
	fz_var(items);
	fz_var(n);
	fz_try(ctx)
	{
		dev = fz_new_derived_device(ctx, ex_image_device);
		dev->super.fill_image = ex_list_image;
		/* Culled to the page: what lands on the page is what's listed. */
		fz_run_display_list(ctx, list, &dev->super, fz_identity, fz_bound_display_list(ctx, list), cookie);
		fz_close_device(ctx, &dev->super);
	}
	fz_always(ctx)
	{
		/* The items outlive the device, so take them before it is freed (macOS zeroes freed blocks: reading them
		 * after the drop listed nothing there). */
		if (dev)
		{
			items = dev->items;
			n = dev->count;
		}
		fz_drop_device(ctx, &dev->super);
		dev = NULL;
	}
	fz_catch(ctx)
	{
		for (i = 0; i < n; i++)
			fz_drop_image(ctx, items[i].image);
		fz_free(ctx, items);
		return ex_fail(ctx, err);
	}
	*out = items;
	*count = n;
	return 0;
}

void ex_drop_page_images(fz_context *ctx, ex_page_image *images)
{
	fz_free(ctx, images);
}

void ex_drop_image(fz_context *ctx, fz_image *image)
{
	fz_drop_image(ctx, image);
}

int ex_image_sample(fz_context *ctx, fz_image *image, fz_pixmap **out, ex_error *err)
{
	fz_pixmap *pix = NULL;
	fz_pixmap *rgb = NULL;
	const int max_dim = 64;

	fz_var(pix);
	fz_var(rgb);
	fz_try(ctx)
	{
		int w = image->w > 0 ? image->w : 1;
		int h = image->h > 0 ? image->h : 1;
		if (w > max_dim || h > max_dim)
		{
			float scale = (float)max_dim / (float)(w > h ? w : h);
			fz_matrix ctm = fz_scale(scale, scale);
			pix = fz_get_pixmap_from_image(ctx, image, NULL, &ctm, NULL, NULL);
		}
		else
			pix = fz_get_pixmap_from_image(ctx, image, NULL, NULL, NULL, NULL);
		if (!pix->colorspace || pix->w <= 0 || pix->h <= 0)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "the image has no colours to sample");
		rgb = fz_convert_pixmap(ctx, pix, fz_device_rgb(ctx), NULL, NULL, fz_default_color_params, 0);
	}
	fz_always(ctx)
		fz_drop_pixmap(ctx, pix);
	fz_catch(ctx)
	{
		fz_drop_pixmap(ctx, rgb);
		return ex_fail(ctx, err);
	}
	*out = rgb;
	return 0;
}

int ex_pixmap_png(fz_context *ctx, fz_pixmap *pix, fz_buffer **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_try(ctx)
		buf = fz_new_buffer_from_pixmap_as_png(ctx, pix, fz_default_color_params);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = buf;
	return 0;
}

int ex_new_stext_page(fz_context *ctx, fz_page *page, int flags, fz_cookie *cookie, fz_stext_page **out,
	ex_error *err)
{
	fz_stext_page *stext = NULL;
	fz_stext_options opts = { 0 };
	opts.flags = flags;
	fz_try(ctx)
		stext = fz_new_stext_page_from_page_with_cookie(ctx, page, &opts, cookie);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = stext;
	return 0;
}

void ex_drop_stext_page(fz_context *ctx, fz_stext_page *stext)
{
	fz_drop_stext_page(ctx, stext);
}

/* Where ex_stext_copy writes, or NULL arrays to only count. */
typedef struct ex_text_walk {
	ex_text_block *blocks;
	ex_text_line *lines;
	ex_text_char *chars;
	int block_count;
	int line_count;
	int char_count;
} ex_text_walk;

/* Structure nests no deeper than this is followed, so a hostile file can't exhaust the stack. */
#define EX_TEXT_MAX_DEPTH 64

static void ex_walk_text(fz_stext_block *block, ex_text_walk *w, int depth)
{
	for (; block; block = block->next)
	{
		fz_stext_line *line;
		int index, first_line;
		if (block->type == FZ_STEXT_BLOCK_STRUCT)
		{
			if (block->u.s.down && depth < EX_TEXT_MAX_DEPTH)
				ex_walk_text(block->u.s.down->first_block, w, depth + 1);
			continue;
		}
		if (block->type != FZ_STEXT_BLOCK_TEXT)
			continue;
		index = w->block_count++;
		first_line = w->line_count;
		for (line = block->u.t.first_line; line; line = line->next)
		{
			fz_stext_char *ch;
			int at = w->line_count++;
			int first_char = w->char_count;
			for (ch = line->first_char; ch; ch = ch->next)
			{
				if (w->chars)
				{
					ex_text_char *out = &w->chars[w->char_count];
					out->c = ch->c;
					out->flags = ch->flags;
					out->quad = ch->quad;
					out->size = ch->size;
				}
				w->char_count++;
			}
			if (w->lines)
			{
				ex_text_line *out = &w->lines[at];
				out->bbox = line->bbox;
				out->dir = line->dir;
				out->wmode = line->wmode;
				out->joined = (line->flags & FZ_STEXT_LINE_FLAGS_JOINED) != 0;
				out->block = index;
				out->first_char = first_char;
				out->char_count = w->char_count - first_char;
			}
		}
		if (w->blocks)
		{
			w->blocks[index].bbox = block->bbox;
			w->blocks[index].first_line = first_line;
			w->blocks[index].line_count = w->line_count - first_line;
		}
	}
}

void ex_stext_counts(fz_stext_page *stext, int *blocks, int *lines, int *chars)
{
	ex_text_walk w = { 0 };
	ex_walk_text(stext->first_block, &w, 0);
	*blocks = w.block_count;
	*lines = w.line_count;
	*chars = w.char_count;
}

void ex_stext_copy(fz_stext_page *stext, ex_text_block *blocks, ex_text_line *lines, ex_text_char *chars)
{
	ex_text_walk w = { 0 };
	w.blocks = blocks;
	w.lines = lines;
	w.chars = chars;
	ex_walk_text(stext->first_block, &w, 0);
}

int ex_stext_page_text(fz_context *ctx, fz_stext_page *stext, fz_buffer **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_try(ctx)
		buf = fz_new_buffer_from_stext_page(ctx, stext);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = buf;
	return 0;
}

void ex_buffer_data(fz_context *ctx, fz_buffer *buf, const unsigned char **data, size_t *len)
{
	unsigned char *storage = NULL;
	*len = fz_buffer_storage(ctx, buf, &storage);
	*data = storage;
}

void ex_drop_buffer(fz_context *ctx, fz_buffer *buf)
{
	fz_drop_buffer(ctx, buf);
}

/* fz_load_outline leaves out each entry's /F and /C; a PDF's outline iterator has them. It walks the same tree in the
 * same order, so the two are read side by side; levels deeper than this keep no style. */
#define EX_MAX_OUTLINE_STYLE_DEPTH 64

static void ex_outline_styles(fz_context *ctx, fz_outline_iterator *iter, fz_outline *node, int depth)
{
	for (; node; node = node->next)
	{
		fz_outline_item *item = fz_outline_iterator_item(ctx, iter);
		if (!item)
			return;
		node->flags = item->flags & 127;
		/* The PDF iterator gives /C as 0 to 255. */
		node->r = (unsigned int)fz_clampi((int)item->r, 0, 255);
		node->g = (unsigned int)fz_clampi((int)item->g, 0, 255);
		node->b = (unsigned int)fz_clampi((int)item->b, 0, 255);
		if (node->down && depth < EX_MAX_OUTLINE_STYLE_DEPTH &&
			fz_outline_iterator_down(ctx, iter) == FZ_OUTLINE_ITERATOR_AT_ITEM)
		{
			ex_outline_styles(ctx, iter, node->down, depth + 1);
			fz_outline_iterator_up(ctx, iter);
		}
		if (fz_outline_iterator_next(ctx, iter) != FZ_OUTLINE_ITERATOR_AT_ITEM)
			return;
	}
}

int ex_load_outline(fz_context *ctx, fz_document *doc, fz_outline **out, ex_error *err)
{
	fz_outline *outline = NULL;
	fz_outline_iterator *iter = NULL;
	fz_var(outline);
	fz_var(iter);
	fz_try(ctx)
	{
		outline = fz_load_outline(ctx, doc);
		if (outline && pdf_specifics(ctx, doc))
		{
			iter = fz_new_outline_iterator(ctx, doc);
			if (iter)
				ex_outline_styles(ctx, iter, outline, 0);
		}
	}
	fz_always(ctx)
		fz_drop_outline_iterator(ctx, iter);
	fz_catch(ctx)
	{
		fz_drop_outline(ctx, outline);
		return ex_fail(ctx, err);
	}
	*out = outline;
	return 0;
}

int ex_outline_item_of(fz_context *ctx, fz_document *doc, fz_outline *outline, ex_outline_item *out, ex_error *err)
{
	int page = -1;
	fz_try(ctx)
	{
		if (outline->page.page >= 0)
			page = fz_page_number_from_location(ctx, doc, outline->page);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	out->title = outline->title;
	out->uri = outline->uri;
	out->page = page;
	out->is_open = outline->is_open;
	out->next = outline->next;
	out->down = outline->down;
	out->flags = outline->flags;
	out->color = (outline->r || outline->g || outline->b)
		? (int)((outline->r << 16) | (outline->g << 8) | outline->b)
		: -1;
	return 0;
}

void ex_drop_outline(fz_context *ctx, fz_outline *outline)
{
	fz_drop_outline(ctx, outline);
}

int ex_load_links(fz_context *ctx, fz_page *page, fz_link **out, ex_error *err)
{
	fz_link *links = NULL;
	fz_try(ctx)
		links = fz_load_links(ctx, page);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = links;
	return 0;
}

int ex_link_item_of(fz_context *ctx, fz_page *page, fz_link *link, ex_link_item *out, ex_error *err)
{
	const char *uri = link->uri ? link->uri : "";
	int number = -1;
	float x = NAN, y = NAN;
	fz_try(ctx)
	{
		if (!fz_is_external_link(ctx, uri))
		{
			fz_location location = fz_resolve_link(ctx, page->doc, uri, &x, &y);
			if (location.page >= 0)
				number = fz_page_number_from_location(ctx, page->doc, location);
		}
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	out->rect = link->rect;
	out->uri = uri;
	out->page = number;
	out->x = x;
	out->y = y;
	out->next = link->next;
	return 0;
}

void ex_drop_links(fz_context *ctx, fz_link *link)
{
	fz_drop_link(ctx, link);
}

int ex_first_annot(fz_context *ctx, fz_page *page, pdf_annot **out, ex_error *err)
{
	pdf_annot *annot = NULL;
	fz_try(ctx)
	{
		pdf_page *pdf = pdf_page_from_fz_page(ctx, page);
		if (pdf)
			annot = pdf_keep_annot(ctx, pdf_first_annot(ctx, pdf));
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = annot;
	return 0;
}

int ex_next_annot(fz_context *ctx, pdf_annot *annot, pdf_annot **out, ex_error *err)
{
	pdf_annot *next = NULL;
	fz_try(ctx)
		next = pdf_keep_annot(ctx, pdf_next_annot(ctx, annot));
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = next;
	return 0;
}

int ex_annot_info(fz_context *ctx, pdf_annot *annot, int *type, fz_rect *rect, ex_error *err)
{
	enum pdf_annot_type kind = PDF_ANNOT_UNKNOWN;
	fz_rect bounds = fz_empty_rect;
	fz_try(ctx)
	{
		kind = pdf_annot_type(ctx, annot);
		bounds = pdf_bound_annot(ctx, annot);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*type = (int)kind;
	*rect = bounds;
	return 0;
}

int ex_annot_contents(fz_context *ctx, pdf_annot *annot, fz_buffer **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_try(ctx)
	{
		const char *text = pdf_annot_contents(ctx, annot);
		if (!text)
			text = "";
		buf = fz_new_buffer_from_copied_data(ctx, (const unsigned char *)text, strlen(text));
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = buf;
	return 0;
}

void ex_drop_annot(fz_context *ctx, pdf_annot *annot)
{
	pdf_drop_annot(ctx, annot);
}

/* The annotation model's private key: a text string the app owns (anchor and tags). */
#define EX_APP_KEY "ExcellentPDF"

/* Ink strokes, line ends or vertices: whichever of them the annotation's type has. */
static int ex_annot_point_count(fz_context *ctx, pdf_annot *annot, int *strokes)
{
	int i, n = 0;
	*strokes = 0;
	if (pdf_annot_has_ink_list(ctx, annot))
	{
		*strokes = pdf_annot_ink_list_count(ctx, annot);
		for (i = 0; i < *strokes; i++)
			n += pdf_annot_ink_list_stroke_count(ctx, annot, i);
		return n;
	}
	if (pdf_annot_has_line(ctx, annot))
		return 2;
	if (pdf_annot_has_vertices(ctx, annot))
		return pdf_annot_vertex_count(ctx, annot);
	return 0;
}

/* Reads `annot`'s fill and dash pattern into `props` (A.9). */
static void ex_annot_shape_props(fz_context *ctx, pdf_annot *annot, ex_annot_props *props)
{
	int i;
	if (pdf_annot_has_interior_color(ctx, annot))
		pdf_annot_interior_color(ctx, annot, &props->fill_n, props->fill);
	if (pdf_annot_has_border(ctx, annot) && pdf_annot_border_style(ctx, annot) == PDF_BORDER_STYLE_DASHED)
	{
		props->dash_count = fz_mini(pdf_annot_border_dash_count(ctx, annot), 8);
		for (i = 0; i < props->dash_count; i++)
			props->dash[i] = pdf_annot_border_dash_item(ctx, annot, i);
	}
	if (pdf_annot_has_border_effect(ctx, annot) && pdf_annot_border_effect(ctx, annot) == PDF_BORDER_EFFECT_CLOUDY)
	{
		props->cloudy = 1;
		props->cloud_intensity = pdf_annot_border_effect_intensity(ctx, annot);
	}
	props->locked = (pdf_annot_flags(ctx, annot) & PDF_ANNOT_IS_LOCKED) != 0;
}

/* Writes `p`'s fill and dash pattern (A.9). The border is touched only when its dashes change, so another app's
 * beveled or inset border stays as it is. */
static void ex_annot_write_shape(fz_context *ctx, pdf_annot *annot, const ex_annot_props *p)
{
	int i, same;
	/* No fill writes no /IC, rather than an empty one, unless there was one to clear. */
	if (pdf_annot_has_interior_color(ctx, annot)
		&& (p->fill_n > 0 || pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(IC))))
		pdf_set_annot_interior_color(ctx, annot, p->fill_n, p->fill);
	if (!pdf_annot_has_border(ctx, annot))
		return;
	if (pdf_annot_border_style(ctx, annot) == PDF_BORDER_STYLE_DASHED)
	{
		same = pdf_annot_border_dash_count(ctx, annot) == p->dash_count;
		for (i = 0; same && i < p->dash_count; i++)
			same = pdf_annot_border_dash_item(ctx, annot, i) == p->dash[i];
	}
	else
		same = p->dash_count == 0;
	/* The border effect, only when it changes: a plain border stays without /BE (A.10). */
	if (pdf_annot_has_border_effect(ctx, annot))
	{
		int cloudy = pdf_annot_border_effect(ctx, annot) == PDF_BORDER_EFFECT_CLOUDY;
		if (cloudy != p->cloudy || (cloudy && pdf_annot_border_effect_intensity(ctx, annot) != p->cloud_intensity))
		{
			pdf_set_annot_border_effect(ctx, annot, p->cloudy ? PDF_BORDER_EFFECT_CLOUDY : PDF_BORDER_EFFECT_NONE);
			if (p->cloudy)
				pdf_set_annot_border_effect_intensity(ctx, annot, p->cloud_intensity);
		}
	}
	if (same)
		return;
	pdf_clear_annot_border_dash(ctx, annot);
	for (i = 0; i < p->dash_count; i++)
		pdf_add_annot_border_dash_item(ctx, annot, p->dash[i]);
	pdf_set_annot_border_style(ctx, annot, p->dash_count > 0 ? PDF_BORDER_STYLE_DASHED : PDF_BORDER_STYLE_SOLID);
}

int ex_annot_props_of(fz_context *ctx, pdf_annot *annot, ex_annot_props *out, ex_error *err)
{
	ex_annot_props props;
	memset(&props, 0, sizeof props);
	fz_try(ctx)
	{
		props.type = (int)pdf_annot_type(ctx, annot);
		props.rect = pdf_annot_has_rect(ctx, annot) ? pdf_annot_rect(ctx, annot) : pdf_bound_annot(ctx, annot);
		pdf_annot_color(ctx, annot, &props.color_n, props.color);
		props.opacity = pdf_annot_opacity(ctx, annot);
		props.border = pdf_annot_has_border(ctx, annot) ? pdf_annot_border_width(ctx, annot) : 0;
		if (pdf_annot_has_author(ctx, annot))
		{
			props.created = pdf_annot_creation_date(ctx, annot);
			props.modified = pdf_annot_modification_date(ctx, annot);
		}
		props.quad_count = pdf_annot_has_quad_points(ctx, annot) ? pdf_annot_quad_point_count(ctx, annot) : 0;
		props.point_count = ex_annot_point_count(ctx, annot, &props.stroke_count);
		props.is_open = pdf_annot_has_open(ctx, annot) ? pdf_annot_is_open(ctx, annot) : 0;
		if (pdf_annot_has_intent(ctx, annot))
			props.intent = (int)pdf_annot_intent(ctx, annot);
		if (pdf_annot_has_quadding(ctx, annot))
			props.quadding = pdf_annot_quadding(ctx, annot);
		if (pdf_annot_has_default_appearance(ctx, annot))
		{
			const char *font;
			pdf_annot_default_appearance(ctx, annot, &font, &props.font_size, &props.text_color_n, props.text_color);
		}
		if (props.type == PDF_ANNOT_FREE_TEXT)
		{
			if (props.intent == PDF_ANNOT_IT_FREETEXT_CALLOUT)
			{
				pdf_annot_callout_line(ctx, annot, props.callout, &props.callout_count);
				props.line_start = (int)pdf_annot_callout_style(ctx, annot);
			}
		}
		else if (pdf_annot_has_line_ending_styles(ctx, annot))
		{
			enum pdf_line_ending start, end;
			pdf_annot_line_ending_styles(ctx, annot, &start, &end);
			props.line_start = (int)start;
			props.line_end = (int)end;
		}
		ex_annot_shape_props(ctx, annot, &props);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = props;
	return 0;
}

int ex_annot_arrays(fz_context *ctx, pdf_annot *annot, fz_quad *quads, int *stroke_lengths, fz_point *points,
	ex_error *err)
{
	fz_try(ctx)
	{
		int i, k, n = 0;
		if (quads && pdf_annot_has_quad_points(ctx, annot))
			for (i = 0; i < pdf_annot_quad_point_count(ctx, annot); i++)
				quads[i] = pdf_annot_quad_point(ctx, annot, i);
		if (pdf_annot_has_ink_list(ctx, annot))
		{
			for (i = 0; i < pdf_annot_ink_list_count(ctx, annot); i++)
			{
				int length = pdf_annot_ink_list_stroke_count(ctx, annot, i);
				if (stroke_lengths)
					stroke_lengths[i] = length;
				for (k = 0; k < length; k++, n++)
					if (points)
						points[n] = pdf_annot_ink_list_stroke_vertex(ctx, annot, i, k);
			}
		}
		else if (points && pdf_annot_has_line(ctx, annot))
			pdf_annot_line(ctx, annot, &points[0], &points[1]);
		else if (points && pdf_annot_has_vertices(ctx, annot))
			for (i = 0; i < pdf_annot_vertex_count(ctx, annot); i++)
				points[i] = pdf_annot_vertex(ctx, annot, i);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_annot_text(fz_context *ctx, pdf_annot *annot, int which, fz_buffer **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_try(ctx)
	{
		const char *text = NULL;
		switch (which)
		{
		case EX_ANNOT_CONTENTS:
			text = pdf_annot_contents(ctx, annot);
			break;
		case EX_ANNOT_ID:
			text = pdf_annot_name(ctx, annot);
			break;
		case EX_ANNOT_AUTHOR:
			if (pdf_annot_has_author(ctx, annot))
				text = pdf_annot_author(ctx, annot);
			break;
		case EX_ANNOT_APP_DATA:
			text = pdf_to_text_string(ctx, pdf_dict_gets(ctx, pdf_annot_obj(ctx, annot), EX_APP_KEY));
			break;
		case EX_ANNOT_GROUP:
		{
			pdf_obj *obj = pdf_annot_obj(ctx, annot);
			if (pdf_name_eq(ctx, pdf_dict_get(ctx, obj, PDF_NAME(RT)), PDF_NAME(Group)))
				text = pdf_dict_get_text_string(ctx, pdf_dict_get(ctx, obj, PDF_NAME(IRT)), PDF_NAME(NM));
			break;
		}
		case EX_ANNOT_ICON:
			/* The file's own name, not MuPDF's default for the kind, so what is read is what was written. */
			if (pdf_annot_has_icon_name(ctx, annot))
				text = pdf_to_name(ctx, pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(Name)));
			break;
		case EX_ANNOT_REPLY_TO:
		{
			pdf_obj *obj = pdf_annot_obj(ctx, annot);
			if (ex_is_reply(ctx, obj))
				text = pdf_dict_get_text_string(ctx, pdf_dict_get(ctx, obj, PDF_NAME(IRT)), PDF_NAME(NM));
			break;
		}
		case EX_ANNOT_STATE:
			text = pdf_to_name(ctx, pdf_dict_gets(ctx, pdf_annot_obj(ctx, annot), "State"));
			break;
		case EX_ANNOT_LAYER:
		{
			pdf_obj *oc = pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(OC));
			if (pdf_name_eq(ctx, pdf_dict_get(ctx, oc, PDF_NAME(Type)), PDF_NAME(OCG)))
				text = pdf_dict_get_text_string(ctx, oc, PDF_NAME(Name));
			break;
		}
		case EX_ANNOT_FONT:
			if (pdf_annot_has_default_appearance(ctx, annot))
			{
				float size, color[4];
				int n;
				pdf_annot_default_appearance(ctx, annot, &text, &size, &n, color);
			}
			break;
		default:
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no annotation text %d", which);
		}
		if (!text)
			text = "";
		buf = fz_new_buffer_from_copied_data(ctx, (const unsigned char *)text, strlen(text));
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = buf;
	return 0;
}

/* ---- Layers (A.22): optional content groups (/OCProperties) that annotations are put on with /OC ---- */

/* MuPDF reads a document's layers once and keeps them (doc->ocg): it reads them again after an edit changed them,
 * or undo or redo did. */
static void ex_reset_layers(fz_context *ctx, pdf_document *pdf)
{
	pdf_drop_ocg(ctx, pdf);
	pdf->ocg = NULL;
}

/* The document's /OCProperties with its /OCGs and /D, made when `make` (else NULL when it has none). */
static pdf_obj *ex_ocprops(fz_context *ctx, pdf_document *pdf, int make)
{
	pdf_obj *root = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
	pdf_obj *props = pdf_dict_get(ctx, root, PDF_NAME(OCProperties));
	if (!make)
		return props;
	if (!props)
		props = pdf_dict_put_dict(ctx, root, PDF_NAME(OCProperties), 2);
	if (!pdf_is_array(ctx, pdf_dict_get(ctx, props, PDF_NAME(OCGs))))
		pdf_dict_put_array(ctx, props, PDF_NAME(OCGs), 4);
	if (!pdf_is_dict(ctx, pdf_dict_get(ctx, props, PDF_NAME(D))))
		pdf_dict_put_dict(ctx, props, PDF_NAME(D), 4);
	return props;
}

/* The first layer of `props` named `name`, or NULL. */
static pdf_obj *ex_find_ocg(fz_context *ctx, pdf_obj *props, const char *name)
{
	pdf_obj *ocgs = pdf_dict_get(ctx, props, PDF_NAME(OCGs));
	int i, n = pdf_array_len(ctx, ocgs);
	for (i = 0; i < n; i++)
	{
		pdf_obj *ocg = pdf_array_get(ctx, ocgs, i);
		if (!strcmp(pdf_dict_get_text_string(ctx, ocg, PDF_NAME(Name)), name))
			return ocg;
	}
	return NULL;
}

/* Puts `ocg` in the array `key` of `d` once (made when it has none), or takes it out of it. */
static void ex_ocg_in(fz_context *ctx, pdf_obj *d, pdf_obj *key, pdf_obj *ocg, int in)
{
	pdf_obj *array = pdf_dict_get(ctx, d, key);
	int at;
	if (in && !pdf_is_array(ctx, array))
		array = pdf_dict_put_array(ctx, d, key, 4);
	while ((at = pdf_array_find(ctx, array, ocg)) >= 0)
		pdf_array_delete(ctx, array, at);
	if (in)
		pdf_array_push(ctx, array, ocg);
}

/* The layer named `name`, made when the document has none: listed in /OCGs and in the default configuration's
 * /Order (so readers show it in their layers panel), shown. Borrowed from /OCGs. */
static pdf_obj *ex_make_ocg(fz_context *ctx, pdf_document *pdf, const char *name)
{
	pdf_obj *props = ex_ocprops(ctx, pdf, 1);
	pdf_obj *ocgs = pdf_dict_get(ctx, props, PDF_NAME(OCGs));
	pdf_obj *d = pdf_dict_get(ctx, props, PDF_NAME(D));
	pdf_obj *ocg = ex_find_ocg(ctx, props, name);
	pdf_obj *made;
	if (ocg)
		return ocg;
	made = pdf_add_new_dict(ctx, pdf, 2);
	fz_try(ctx)
	{
		pdf_obj *order = pdf_dict_get(ctx, d, PDF_NAME(Order));
		pdf_dict_put(ctx, made, PDF_NAME(Type), PDF_NAME(OCG));
		pdf_dict_put_text_string(ctx, made, PDF_NAME(Name), name);
		pdf_array_push(ctx, ocgs, made);
		/* No /Order lets a reader list the layers as it likes: a new one would list ours alone, so it lists them
		 * all. */
		if (pdf_is_array(ctx, order))
			pdf_array_push(ctx, order, made);
		else
			pdf_dict_put_drop(ctx, d, PDF_NAME(Order), pdf_deep_copy_obj(ctx, ocgs));
		if (pdf_name_eq(ctx, pdf_dict_get(ctx, d, PDF_NAME(BaseState)), PDF_NAME(OFF)))
			ex_ocg_in(ctx, d, PDF_NAME(ON), made, 1);
	}
	fz_always(ctx)
		pdf_drop_obj(ctx, made);
	fz_catch(ctx)
		fz_rethrow(ctx);
	ex_reset_layers(ctx, pdf);
	return ex_find_ocg(ctx, props, name);
}

/* Puts `annot` on the layer named `layer` (/OC), made when the document has none of that name, or on none when
 * `layer` is NULL or empty. Another app's membership dictionary (/OCMD, which reads as no layer) is kept then. */
static void ex_annot_layer(fz_context *ctx, pdf_annot *annot, const char *layer)
{
	pdf_obj *obj = pdf_annot_obj(ctx, annot);
	pdf_obj *oc = pdf_dict_get(ctx, obj, PDF_NAME(OC));
	int is_ocg = pdf_name_eq(ctx, pdf_dict_get(ctx, oc, PDF_NAME(Type)), PDF_NAME(OCG));
	if (layer && *layer)
	{
		if (is_ocg && !strcmp(pdf_dict_get_text_string(ctx, oc, PDF_NAME(Name)), layer))
			return;
		pdf_dict_put(ctx, obj, PDF_NAME(OC), ex_make_ocg(ctx, pdf_annot_page(ctx, annot)->doc, layer));
	}
	else if (is_ocg)
		pdf_dict_del(ctx, obj, PDF_NAME(OC));
}

int ex_layers(fz_context *ctx, fz_document *doc, fz_buffer **out, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_var(buf);
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		pdf_obj *props = pdf ? ex_ocprops(ctx, pdf, 0) : NULL;
		pdf_obj *ocgs = pdf_dict_get(ctx, props, PDF_NAME(OCGs));
		pdf_obj *d = pdf_dict_get(ctx, props, PDF_NAME(D));
		int base_off = pdf_name_eq(ctx, pdf_dict_get(ctx, d, PDF_NAME(BaseState)), PDF_NAME(OFF));
		int i, n = pdf_array_len(ctx, ocgs);
		buf = fz_new_buffer(ctx, 64);
		for (i = 0; i < n; i++)
		{
			pdf_obj *ocg = pdf_array_get(ctx, ocgs, i);
			const char *name = pdf_dict_get_text_string(ctx, ocg, PDF_NAME(Name));
			int hidden = pdf_array_contains(ctx, pdf_dict_get(ctx, d, PDF_NAME(OFF)), ocg)
				|| (base_off && !pdf_array_contains(ctx, pdf_dict_get(ctx, d, PDF_NAME(ON)), ocg));
			int locked = pdf_array_contains(ctx, pdf_dict_get(ctx, d, PDF_NAME(Locked)), ocg);
			fz_append_byte(ctx, buf, '0' + hidden + 2 * locked);
			fz_append_string(ctx, buf, name);
			fz_append_byte(ctx, buf, 0);
		}
	}
	fz_catch(ctx)
	{
		fz_drop_buffer(ctx, buf);
		return ex_fail(ctx, err);
	}
	*out = buf;
	return 0;
}

int ex_layer_set(fz_context *ctx, fz_document *doc, const char *name, int visible, int locked, ex_error *err)
{
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		pdf_obj *props, *ocg, *d;
		if (!pdf)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "only a PDF has layers to edit");
		props = ex_ocprops(ctx, pdf, 0);
		ocg = props ? ex_find_ocg(ctx, props, name) : NULL;
		if (!ocg)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no layer %s", name);
		props = ex_ocprops(ctx, pdf, 1);
		d = pdf_dict_get(ctx, props, PDF_NAME(D));
		ex_ocg_in(ctx, d, PDF_NAME(OFF), ocg, !visible);
		ex_ocg_in(ctx, d, PDF_NAME(ON), ocg,
			visible && pdf_name_eq(ctx, pdf_dict_get(ctx, d, PDF_NAME(BaseState)), PDF_NAME(OFF)));
		ex_ocg_in(ctx, d, PDF_NAME(Locked), ocg, locked);
		ex_reset_layers(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

static pdf_annot *ex_find_annot(fz_context *ctx, pdf_page *page, const char *id);

/* Groups `annot` with the annotation of its page whose /NM is `group` (/IRT, /RT /Group), or ungroups it when
 * `group` is NULL or empty. A reply (an /IRT without /RT /Group) is left as it is. */
static void ex_annot_group(fz_context *ctx, pdf_annot *annot, const char *group)
{
	pdf_obj *obj = pdf_annot_obj(ctx, annot);
	if (group && *group)
	{
		pdf_annot *target = ex_find_annot(ctx, pdf_annot_page(ctx, annot), group);
		if (target == annot)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "an annotation can't be grouped with itself");
		pdf_dict_put(ctx, obj, PDF_NAME(IRT), pdf_annot_obj(ctx, target));
		pdf_dict_put(ctx, obj, PDF_NAME(RT), PDF_NAME(Group));
	}
	else if (pdf_name_eq(ctx, pdf_dict_get(ctx, obj, PDF_NAME(RT)), PDF_NAME(Group)))
	{
		pdf_dict_del(ctx, obj, PDF_NAME(IRT));
		pdf_dict_del(ctx, obj, PDF_NAME(RT));
	}
}

/* Makes `annot` a reply to the annotation of its page whose /NM is `parent` (/IRT, no /RT: A.21), or no reply when
 * `parent` is NULL or empty. With a `state` the reply is a review status (/State, /StateModel): its own model for
 * Marked and Unmarked, else Review; readers keep it hidden on the page (/F 30, as Acrobat writes it). */
static void ex_annot_reply(fz_context *ctx, pdf_annot *annot, const char *parent, const char *state)
{
	pdf_obj *obj = pdf_annot_obj(ctx, annot);
	if (parent && *parent)
	{
		pdf_annot *target = ex_find_annot(ctx, pdf_annot_page(ctx, annot), parent);
		if (target == annot)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "an annotation can't answer itself");
		pdf_dict_put(ctx, obj, PDF_NAME(IRT), pdf_annot_obj(ctx, target));
		pdf_dict_del(ctx, obj, PDF_NAME(RT));
	}
	else if (ex_is_reply(ctx, obj))
	{
		pdf_dict_del(ctx, obj, PDF_NAME(IRT));
		pdf_dict_del(ctx, obj, PDF_NAME(RT));
	}
	if (state && *state)
	{
		int marked = !strcmp(state, "Marked") || !strcmp(state, "Unmarked");
		pdf_dict_puts_drop(ctx, obj, "State", pdf_new_name(ctx, state));
		pdf_dict_puts_drop(ctx, obj, "StateModel", pdf_new_name(ctx, marked ? "Marked" : "Review"));
		pdf_set_annot_flags(ctx, annot, pdf_annot_flags(ctx, annot) | PDF_ANNOT_IS_HIDDEN | PDF_ANNOT_IS_PRINT
			| PDF_ANNOT_IS_NO_ZOOM | PDF_ANNOT_IS_NO_ROTATE);
	}
	else
	{
		pdf_dict_dels(ctx, obj, "State");
		pdf_dict_dels(ctx, obj, "StateModel");
	}
}

/* A note's popup (A.4): 200 x 120 pt beside the annotation's right edge, its top at the annotation's, moved inside the
 * page when it would leave it. Other readers open the note there. */
static fz_rect ex_popup_rect(fz_context *ctx, pdf_annot *annot, const ex_annot_write *w)
{
	fz_rect page = pdf_bound_page(ctx, pdf_annot_page(ctx, annot), FZ_CROP_BOX);
	fz_rect at = w->props.rect;
	float width = fz_min(200, page.x1 - page.x0);
	float height = fz_min(120, page.y1 - page.y0);
	fz_rect popup;
	int i;
	if (w->props.quad_count > 0)
	{
		at = fz_rect_from_quad(w->quads[0]);
		for (i = 1; i < w->props.quad_count; i++)
			at = fz_union_rect(at, fz_rect_from_quad(w->quads[i]));
	}
	popup.x0 = fz_clamp(at.x1 + 8, page.x0, page.x1 - width);
	popup.y0 = fz_clamp(at.y0, page.y0, page.y1 - height);
	popup.x1 = popup.x0 + width;
	popup.y1 = popup.y0 + height;
	return popup;
}

/* Writes a text box's font, size and colour (/DA) when they changed: writing it drops the rich text (/RC, /DS)
 * another app may have given the box, so an unchanged style keeps it. */
static void ex_annot_text_style(fz_context *ctx, pdf_annot *annot, const ex_annot_write *w)
{
	const ex_annot_props *p = &w->props;
	const char *font = w->font && *w->font ? w->font : "Helv";
	float size = p->font_size > 0 ? p->font_size : 12;
	const char *was_font;
	float was_size, was_color[4];
	int was_n, i, same;
	pdf_annot_default_appearance(ctx, annot, &was_font, &was_size, &was_n, was_color);
	same = !strcmp(font, was_font) && size == was_size && p->text_color_n == was_n;
	for (i = 0; same && i < was_n && i < 4; i++)
		same = p->text_color[i] == was_color[i];
	if (!same || !pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(DA)))
		pdf_set_annot_default_appearance(ctx, annot, font, size, p->text_color_n, p->text_color);
}

/* Writes every property of `w` to `annot`, then rebuilds its appearance. */
static void ex_annot_write_to(fz_context *ctx, pdf_annot *annot, const ex_annot_write *w)
{
	const ex_annot_props *p = &w->props;
	/* MuPDF's rect setter gives every free text a leader line (to the page's corner when it has none): only a
	 * callout keeps one. */
	int had_callout = pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(CL)) != NULL;
	pdf_set_annot_name(ctx, annot, w->id);
	/* Before the rect: MuPDF's width setter grows /Rect by half the change in width. */
	if (pdf_annot_has_border(ctx, annot))
		pdf_set_annot_border_width(ctx, annot, p->border);
	if (pdf_annot_has_rect(ctx, annot))
		pdf_set_annot_rect(ctx, annot, p->rect);
	pdf_set_annot_contents(ctx, annot, w->contents ? w->contents : "");
	if (pdf_annot_has_author(ctx, annot))
	{
		if (w->author)
			pdf_set_annot_author(ctx, annot, w->author);
		pdf_set_annot_creation_date(ctx, annot, p->created);
	}
	if (w->app_data && *w->app_data)
		pdf_dict_puts_drop(ctx, pdf_annot_obj(ctx, annot), EX_APP_KEY, pdf_new_text_string(ctx, w->app_data));
	else
		pdf_dict_dels(ctx, pdf_annot_obj(ctx, annot), EX_APP_KEY);
	pdf_set_annot_color(ctx, annot, p->color_n, p->color);
	pdf_set_annot_opacity(ctx, annot, p->opacity);
	if (pdf_annot_has_quad_points(ctx, annot))
		pdf_set_annot_quad_points(ctx, annot, p->quad_count, w->quads);
	if (pdf_annot_has_ink_list(ctx, annot))
		pdf_set_annot_ink_list(ctx, annot, p->stroke_count, w->stroke_lengths, w->points);
	else if (pdf_annot_has_line(ctx, annot))
	{
		if (p->point_count != 2)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "a line has two points, not %d", p->point_count);
		pdf_set_annot_line(ctx, annot, w->points[0], w->points[1]);
	}
	else if (pdf_annot_has_vertices(ctx, annot))
		pdf_set_annot_vertices(ctx, annot, p->point_count, w->points);
	/* A note gets a popup to open in other readers (A.4); one already there keeps its place. A text box shows its
	 * text on the page, so it has none (A.6), nor has a reply, read in its thread (A.21). */
	if (w->contents && *w->contents && pdf_annot_has_popup(ctx, annot) && pdf_annot_type(ctx, annot) != PDF_ANNOT_FREE_TEXT
		&& !(w->reply_to && *w->reply_to) && !pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(Popup)))
		pdf_set_annot_popup(ctx, annot, ex_popup_rect(ctx, annot, w));
	if (pdf_annot_has_icon_name(ctx, annot))
		pdf_set_annot_icon_name(ctx, annot, w->icon && *w->icon ? w->icon : NULL);
	if (pdf_annot_has_open(ctx, annot))
		pdf_set_annot_is_open(ctx, annot, p->is_open);
	if (pdf_annot_type(ctx, annot) == PDF_ANNOT_FREE_TEXT)
	{
		ex_annot_text_style(ctx, annot, w);
		pdf_set_annot_quadding(ctx, annot, p->quadding);
		/* Only a text box's: other kinds' intents (a dimension line's) are not the model's and stay as they are. */
		pdf_set_annot_intent(ctx, annot, (enum pdf_intent)p->intent);
		if (p->intent == PDF_ANNOT_IT_FREETEXT_CALLOUT && p->callout_count > 0)
		{
			/* From the tip, MuPDF draws the line to the box's edge, and draws it again when the box moves (A.7). */
			pdf_set_annot_callout_point(ctx, annot, p->callout[0]);
			pdf_set_annot_callout_style(ctx, annot, (enum pdf_line_ending)p->line_start);
		}
		else if (!had_callout)
			pdf_dict_del(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(CL));
	}
	else if (pdf_annot_has_line_ending_styles(ctx, annot))
		pdf_set_annot_line_ending_styles(ctx, annot, (enum pdf_line_ending)p->line_start,
			(enum pdf_line_ending)p->line_end);
	ex_annot_write_shape(ctx, annot, p);
	{
		/* Only the Locked bit: the others (hidden, print, no zoom) are another app's and stay as they are (A.12). */
		int flags = pdf_annot_flags(ctx, annot);
		int locked = p->locked ? flags | PDF_ANNOT_IS_LOCKED : flags & ~PDF_ANNOT_IS_LOCKED;
		if (locked != flags)
			pdf_set_annot_flags(ctx, annot, locked);
	}
	ex_annot_group(ctx, annot, w->group);
	/* A group's /IRT is the group's (A.2). */
	if (!(w->group && *w->group))
		ex_annot_reply(ctx, annot, w->reply_to, w->state);
	ex_annot_layer(ctx, annot, w->layer);
	/* Last: every setter above stamps the modification date with the time now. */
	if (pdf_annot_has_author(ctx, annot))
		pdf_set_annot_modification_date(ctx, annot, p->modified);
	pdf_update_annot(ctx, annot);
}

/* Page `page` of the PDF behind `doc`, a new reference. */
static pdf_page *ex_pdf_page(fz_context *ctx, fz_document *doc, int page)
{
	pdf_document *pdf = pdf_specifics(ctx, doc);
	if (!pdf)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "only a PDF has annotations to edit");
	if (page < 0 || page >= pdf_count_pages(ctx, pdf))
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "no page %d", page);
	return pdf_load_page(ctx, pdf, page);
}

/* The annotation of `page` whose /NM is `id`, borrowed from the page. */
static pdf_annot *ex_find_annot(fz_context *ctx, pdf_page *page, const char *id)
{
	pdf_annot *annot;
	if (!id || !*id)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "an annotation is named by its id");
	for (annot = pdf_first_annot(ctx, page); annot; annot = pdf_next_annot(ctx, annot))
	{
		const char *name = pdf_annot_name(ctx, annot);
		if (name && !strcmp(name, id))
			return annot;
	}
	fz_throw(ctx, FZ_ERROR_ARGUMENT, "no annotation %s on the page", id);
}

int ex_annot_create(fz_context *ctx, fz_document *doc, int page, const ex_annot_write *w, ex_error *err)
{
	pdf_page *p = NULL;
	pdf_annot *annot = NULL;
	fz_var(p);
	fz_var(annot);
	fz_try(ctx)
	{
		if (!w->id || !*w->id)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "a new annotation needs an id");
		p = ex_pdf_page(ctx, doc, page);
		annot = pdf_create_annot(ctx, p, (enum pdf_annot_type)w->props.type);
		ex_annot_write_to(ctx, annot, w);
	}
	fz_always(ctx)
	{
		pdf_drop_annot(ctx, annot);
		fz_drop_page(ctx, (fz_page *)p);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_annot_update(fz_context *ctx, fz_document *doc, int page, const ex_annot_write *w, ex_error *err)
{
	pdf_page *p = NULL;
	fz_var(p);
	fz_try(ctx)
	{
		pdf_annot *annot;
		p = ex_pdf_page(ctx, doc, page);
		annot = ex_find_annot(ctx, p, w->id);
		if ((int)pdf_annot_type(ctx, annot) != w->props.type)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "annotation %s is of another type", w->id);
		ex_annot_write_to(ctx, annot, w);
	}
	fz_always(ctx)
		fz_drop_page(ctx, (fz_page *)p);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* Deletes `annot` from `page`, the replies under it first, however deep (A.21): a reply left behind would answer
 * nothing. `depth` stops a loop of replies in a broken file. */
static void ex_delete_thread(fz_context *ctx, pdf_page *page, pdf_annot *annot, int depth)
{
	pdf_obj *obj = pdf_resolve_indirect(ctx, pdf_annot_obj(ctx, annot));
	pdf_annot *reply = pdf_first_annot(ctx, page);
	if (depth > 100)
		fz_throw(ctx, FZ_ERROR_FORMAT, "replies answer each other in a loop");
	while (reply)
	{
		pdf_obj *r = pdf_annot_obj(ctx, reply);
		if (reply != annot && ex_is_reply(ctx, r)
			&& pdf_resolve_indirect(ctx, pdf_dict_get(ctx, r, PDF_NAME(IRT))) == obj)
		{
			ex_delete_thread(ctx, page, reply, depth + 1);
			/* The list changed under us: look again from the start. */
			reply = pdf_first_annot(ctx, page);
		}
		else
			reply = pdf_next_annot(ctx, reply);
	}
	pdf_delete_annot(ctx, page, annot);
}

int ex_annot_delete(fz_context *ctx, fz_document *doc, int page, const char *id, ex_error *err)
{
	pdf_page *p = NULL;
	fz_var(p);
	fz_try(ctx)
	{
		p = ex_pdf_page(ctx, doc, page);
		ex_delete_thread(ctx, p, ex_find_annot(ctx, p, id), 0);
	}
	fz_always(ctx)
		fz_drop_page(ctx, (fz_page *)p);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_annot_set_stamp_art(fz_context *ctx, fz_document *doc, int page, const char *id, int kind,
	const unsigned char *art, size_t len, ex_error *err)
{
	pdf_page *p = NULL;
	fz_buffer *buf = NULL;
	fz_document *src = NULL;
	fz_page *first = NULL;
	fz_display_list *list = NULL;
	fz_image *image = NULL;
	fz_var(p);
	fz_var(buf);
	fz_var(src);
	fz_var(first);
	fz_var(list);
	fz_var(image);
	fz_try(ctx)
	{
		pdf_annot *annot;
		p = ex_pdf_page(ctx, doc, page);
		annot = ex_find_annot(ctx, p, id);
		if (pdf_annot_type(ctx, annot) != PDF_ANNOT_STAMP)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "annotation %s is not a stamp", id);
		buf = fz_new_buffer_from_copied_data(ctx, art, len);
		if (kind == 0)
		{
			src = fz_open_document_with_buffer(ctx, "application/pdf", buf);
			first = fz_load_page(ctx, src, 0);
			list = fz_new_display_list_from_page(ctx, first);
			pdf_set_annot_appearance_from_display_list(ctx, annot, "N", NULL, fz_identity, list);
		}
		else
		{
			image = fz_new_image_from_buffer(ctx, buf);
			pdf_set_annot_stamp_image(ctx, annot, image);
		}
	}
	fz_always(ctx)
	{
		fz_drop_image(ctx, image);
		fz_drop_display_list(ctx, list);
		fz_drop_page(ctx, first);
		fz_drop_document(ctx, src);
		fz_drop_buffer(ctx, buf);
		fz_drop_page(ctx, (fz_page *)p);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* Runs `draw` (a page, or a placed story) onto one page of a new PDF `box` big, into a new buffer. */
static fz_buffer *ex_one_page_pdf(fz_context *ctx, fz_rect box, fz_page *page, fz_story *story)
{
	fz_buffer *buf = NULL;
	fz_document_writer *wri = NULL;
	fz_var(buf);
	fz_var(wri);
	fz_try(ctx)
	{
		fz_device *dev;
		buf = fz_new_buffer(ctx, 4096);
		wri = fz_new_pdf_writer_with_output(ctx, fz_new_output_with_buffer(ctx, buf), "compress");
		dev = fz_begin_page(ctx, wri, box);
		if (page)
			fz_run_page(ctx, page, dev, fz_identity, NULL);
		else
			fz_draw_story(ctx, story, dev, fz_identity);
		fz_end_page(ctx, wri);
		fz_close_document_writer(ctx, wri);
	}
	fz_always(ctx)
		fz_drop_document_writer(ctx, wri);
	fz_catch(ctx)
	{
		fz_drop_buffer(ctx, buf);
		fz_rethrow(ctx);
	}
	return buf;
}

int ex_page_to_pdf(fz_context *ctx, fz_page *page, fz_buffer **out, ex_error *err)
{
	fz_try(ctx)
		*out = ex_one_page_pdf(ctx, fz_bound_page(ctx, page), page, NULL);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_html_to_pdf(fz_context *ctx, const char *html, const char *css, float width, fz_buffer **out,
	ex_error *err)
{
	fz_buffer *text = NULL;
	fz_story *story = NULL;
	fz_var(text);
	fz_var(story);
	fz_try(ctx)
	{
		fz_rect filled = fz_empty_rect;
		text = fz_new_buffer_from_copied_data(ctx, (const unsigned char *)html, strlen(html));
		story = fz_new_story(ctx, text, css, 12, NULL);
		/* A stamp is short: what doesn't fit in a tall page is left out. */
		fz_place_story(ctx, story, fz_make_rect(0, 0, width, 2000), &filled);
		*out = ex_one_page_pdf(ctx, fz_make_rect(0, 0, width, fz_max(filled.y1, 1)), NULL, story);
	}
	fz_always(ctx)
	{
		fz_drop_story(ctx, story);
		fz_drop_buffer(ctx, text);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_new_document_writer(fz_context *ctx, const char *path, const char *format, const char *options,
	fz_document_writer **out, ex_error *err)
{
	fz_document_writer *wri = NULL;
	fz_try(ctx)
		wri = fz_new_document_writer(ctx, path, format, options);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = wri;
	return 0;
}

int ex_write_page(fz_context *ctx, fz_document_writer *wri, fz_page *page, ex_error *err)
{
	fz_try(ctx)
	{
		fz_device *dev = fz_begin_page(ctx, wri, fz_bound_page(ctx, page));
		fz_run_page(ctx, page, dev, fz_identity, NULL);
		fz_end_page(ctx, wri);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_close_document_writer(fz_context *ctx, fz_document_writer *wri, ex_error *err)
{
	fz_try(ctx)
		fz_close_document_writer(ctx, wri);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

void ex_drop_document_writer(fz_context *ctx, fz_document_writer *wri)
{
	fz_drop_document_writer(ctx, wri);
}

/* ---- PDFs opened for editing (K.15) ---- */

#ifdef _WIN32
#include <windows.h>

typedef struct
{
	HANDLE file;
	unsigned char buffer[64 * 1024];
} ex_shared_file;

static int ex_shared_next(fz_context *ctx, fz_stream *stm, size_t n)
{
	ex_shared_file *state = stm->state;
	DWORD got = 0;
	(void)n; /* a hint only */
	if (!ReadFile(state->file, state->buffer, (DWORD)sizeof state->buffer, &got, NULL))
		fz_throw(ctx, FZ_ERROR_SYSTEM, "read error: %lu", (unsigned long)GetLastError());
	stm->rp = state->buffer;
	stm->wp = state->buffer + got;
	stm->pos += (int64_t)got;
	if (got == 0)
		return EOF;
	return *stm->rp++;
}

static void ex_shared_seek(fz_context *ctx, fz_stream *stm, int64_t offset, int whence)
{
	ex_shared_file *state = stm->state;
	LARGE_INTEGER to, at;
	DWORD method = whence == SEEK_END ? FILE_END : whence == SEEK_CUR ? FILE_CURRENT : FILE_BEGIN;
	/* The buffered bytes not read yet are ahead of the stream's position. */
	if (whence == SEEK_CUR)
		offset -= (int64_t)(stm->wp - stm->rp);
	to.QuadPart = offset;
	if (!SetFilePointerEx(state->file, to, &at, method))
		fz_throw(ctx, FZ_ERROR_SYSTEM, "cannot seek: %lu", (unsigned long)GetLastError());
	stm->pos = at.QuadPart;
	stm->rp = state->buffer;
	stm->wp = state->buffer;
}

static void ex_shared_drop(fz_context *ctx, void *state_)
{
	ex_shared_file *state = state_;
	CloseHandle(state->file);
	fz_free(ctx, state);
}

/* A read stream on `path` that lets others rename, replace or delete the file while it is open. */
static fz_stream *ex_open_shared_file(fz_context *ctx, const char *path)
{
	ex_shared_file *state;
	fz_stream *stm;
	wchar_t *wide;
	HANDLE file;
	int len = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
	if (len <= 0)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "the path is not valid UTF-8");
	wide = fz_malloc(ctx, (size_t)len * sizeof *wide);
	MultiByteToWideChar(CP_UTF8, 0, path, -1, wide, len);
	file = CreateFileW(wide, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL,
		OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
	fz_free(ctx, wide);
	if (file == INVALID_HANDLE_VALUE)
		fz_throw(ctx, FZ_ERROR_SYSTEM, "cannot open file %s: error %lu", path, (unsigned long)GetLastError());
	fz_try(ctx)
		state = fz_malloc_struct(ctx, ex_shared_file);
	fz_catch(ctx)
	{
		CloseHandle(file);
		fz_rethrow(ctx);
	}
	state->file = file;
	/* fz_new_stream drops the state if it fails. */
	stm = fz_new_stream(ctx, state, ex_shared_next, ex_shared_drop);
	stm->seek = ex_shared_seek;
	return stm;
}
#endif

int ex_open_pdf(fz_context *ctx, const char *path, fz_document **out, ex_error *err)
{
	fz_document *doc = NULL;
	fz_stream *stm = NULL;
	fz_var(stm);
	fz_try(ctx)
	{
#ifdef _WIN32
		stm = ex_open_shared_file(ctx, path);
#else
		/* Elsewhere a file may be replaced while it is open. */
		stm = fz_open_file(ctx, path);
#endif
		doc = fz_open_document_with_stream(ctx, "application/pdf", stm);
	}
	fz_always(ctx)
		fz_drop_stream(ctx, stm);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = doc;
	return 0;
}

int ex_is_pdf(fz_context *ctx, fz_document *doc)
{
	return pdf_specifics(ctx, doc) != NULL;
}

/* ---- Edits and undo (K.14) ---- */

static pdf_document *ex_pdf(fz_context *ctx, fz_document *doc)
{
	pdf_document *pdf = pdf_specifics(ctx, doc);
	if (!pdf)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "only a PDF can be edited");
	return pdf;
}

int ex_begin_operation(fz_context *ctx, fz_document *doc, const char *name, ex_error *err)
{
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		pdf_enable_journal(ctx, pdf);
		pdf_begin_operation(ctx, pdf, name);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_end_operation(fz_context *ctx, fz_document *doc, ex_error *err)
{
	fz_try(ctx)
		pdf_end_operation(ctx, ex_pdf(ctx, doc));
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

void ex_abandon_operation(fz_context *ctx, fz_document *doc)
{
	pdf_document *pdf = pdf_specifics(ctx, doc);
	if (!pdf)
		return;
	fz_try(ctx)
		pdf_abandon_operation(ctx, pdf);
	fz_catch(ctx)
		fz_report_error(ctx);
}

int ex_undo_state(fz_context *ctx, fz_document *doc, int *position, int *steps, ex_error *err)
{
	*position = 0;
	*steps = 0;
	fz_try(ctx)
	{
		pdf_document *pdf = pdf_specifics(ctx, doc);
		if (pdf)
			*position = pdf_undoredo_state(ctx, pdf, steps);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_undo(fz_context *ctx, fz_document *doc, ex_error *err)
{
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		pdf_undo(ctx, pdf);
		/* It may have taken back a layer's state (A.22). */
		ex_reset_layers(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_redo(fz_context *ctx, fz_document *doc, ex_error *err)
{
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		pdf_redo(ctx, pdf);
		ex_reset_layers(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* ---- Outline edits (B.1-B.5) ---- */

/* Walks a new iterator to `path`; throws when the path is not in the outline. Returns the iterator, which the caller
 * drops, and sets `*at` to FZ_OUTLINE_ITERATOR_AT_ITEM or _AT_EMPTY (one past the end of a level). */
static fz_outline_iterator *ex_outline_walk(fz_context *ctx, fz_document *doc, const int *path, int depth, int *at)
{
	fz_outline_iterator *iter = fz_new_outline_iterator(ctx, doc);
	fz_try(ctx)
	{
		int level, step;
		/* A new iterator stands on the first entry, or on the empty place of an outline without one. */
		*at = fz_outline_iterator_item(ctx, iter) ? FZ_OUTLINE_ITERATOR_AT_ITEM : FZ_OUTLINE_ITERATOR_AT_EMPTY;
		if (depth < 1)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "an outline path names at least one level");
		for (level = 0; level < depth; level++)
		{
			if (level > 0)
			{
				if (*at != FZ_OUTLINE_ITERATOR_AT_ITEM)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at level %d of the path", level);
				*at = fz_outline_iterator_down(ctx, iter);
				if (*at < 0)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "cannot go under the outline entry");
			}
			for (step = 0; step < path[level]; step++)
			{
				if (*at != FZ_OUTLINE_ITERATOR_AT_ITEM)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "the outline path is past the end of level %d", level);
				*at = fz_outline_iterator_next(ctx, iter);
				if (*at < 0)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "the outline path is past the end of level %d", level);
			}
		}
	}
	fz_catch(ctx)
	{
		fz_drop_outline_iterator(ctx, iter);
		fz_rethrow(ctx);
	}
	return iter;
}

static fz_outline_item ex_outline_item_from(const ex_outline_edit *edit)
{
	fz_outline_item item;
	item.title = (char *)edit->title;
	item.uri = (char *)edit->uri;
	item.is_open = edit->is_open;
	item.flags = edit->flags;
	/* The PDF iterator writes /C as the item's colour over 255. */
	item.r = edit->r * 255;
	item.g = edit->g * 255;
	item.b = edit->b * 255;
	return item;
}

int ex_outline_insert(fz_context *ctx, fz_document *doc, const int *path, int depth, const ex_outline_edit *edit,
	ex_error *err)
{
	fz_outline_iterator *iter = NULL;
	fz_var(iter);
	fz_try(ctx)
	{
		int at;
		fz_outline_item item = ex_outline_item_from(edit);
		iter = ex_outline_walk(ctx, doc, path, depth, &at);
		fz_outline_iterator_insert(ctx, iter, &item);
	}
	fz_always(ctx)
		fz_drop_outline_iterator(ctx, iter);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_outline_update(fz_context *ctx, fz_document *doc, const int *path, int depth, const ex_outline_edit *edit,
	ex_error *err)
{
	fz_outline_iterator *iter = NULL;
	char *title = NULL;
	char *uri = NULL;
	fz_var(iter);
	fz_var(title);
	fz_var(uri);
	fz_try(ctx)
	{
		int at;
		fz_outline_item *now, item;
		iter = ex_outline_walk(ctx, doc, path, depth, &at);
		now = fz_outline_iterator_item(ctx, iter);
		if (at != FZ_OUTLINE_ITERATOR_AT_ITEM || !now)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at the path");
		/* The item is valid only until the next iterator call, which the update is: copy what is kept. */
		item = *now;
		title = fz_strdup(ctx, edit->title ? edit->title : now->title ? now->title : "");
		if (edit->uri || now->uri)
			uri = fz_strdup(ctx, edit->uri ? edit->uri : now->uri);
		item.title = title;
		item.uri = uri;
		item.is_open = edit->is_open;
		fz_outline_iterator_update(ctx, iter, &item);
	}
	fz_always(ctx)
	{
		fz_free(ctx, title);
		fz_free(ctx, uri);
		fz_drop_outline_iterator(ctx, iter);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_outline_style(fz_context *ctx, fz_document *doc, const int *path, int depth, int flags, int color,
	ex_error *err)
{
	fz_outline_iterator *iter = NULL;
	char *title = NULL;
	char *uri = NULL;
	fz_var(iter);
	fz_var(title);
	fz_var(uri);
	fz_try(ctx)
	{
		int at;
		fz_outline_item *now, item;
		iter = ex_outline_walk(ctx, doc, path, depth, &at);
		now = fz_outline_iterator_item(ctx, iter);
		if (at != FZ_OUTLINE_ITERATOR_AT_ITEM || !now)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at the path");
		/* As in ex_outline_update: the item lasts only until the update, so what is kept is copied. */
		item = *now;
		title = fz_strdup(ctx, now->title ? now->title : "");
		if (now->uri)
			uri = fz_strdup(ctx, now->uri);
		item.title = title;
		item.uri = uri;
		item.flags = flags & 127;
		item.r = color < 0 ? 0 : (float)((color >> 16) & 255);
		item.g = color < 0 ? 0 : (float)((color >> 8) & 255);
		item.b = color < 0 ? 0 : (float)(color & 255);
		fz_outline_iterator_update(ctx, iter, &item);
	}
	fz_always(ctx)
	{
		fz_free(ctx, title);
		fz_free(ctx, uri);
		fz_drop_outline_iterator(ctx, iter);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_outline_delete(fz_context *ctx, fz_document *doc, const int *path, int depth, ex_error *err)
{
	fz_outline_iterator *iter = NULL;
	fz_var(iter);
	fz_try(ctx)
	{
		int at;
		iter = ex_outline_walk(ctx, doc, path, depth, &at);
		if (at != FZ_OUTLINE_ITERATOR_AT_ITEM)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at the path");
		fz_outline_iterator_delete(ctx, iter);
	}
	fz_always(ctx)
		fz_drop_outline_iterator(ctx, iter);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* An entry and everything under it, copied out of the outline. */
typedef struct ex_onode
{
	fz_outline_item item;
	struct ex_onode *down;
	struct ex_onode *next;
} ex_onode;

static void ex_onode_drop(fz_context *ctx, ex_onode *node)
{
	while (node)
	{
		ex_onode *next = node->next;
		ex_onode_drop(ctx, node->down);
		fz_free(ctx, node->item.title);
		fz_free(ctx, node->item.uri);
		fz_free(ctx, node);
		node = next;
	}
}

/* Copies the entry the iterator stands on and the entries under it; leaves the iterator where it was. A hostile
 * file's nesting is cut at 64 levels, as reading does. */
static ex_onode *ex_onode_read(fz_context *ctx, fz_outline_iterator *iter, int depth)
{
	fz_outline_item *now = fz_outline_iterator_item(ctx, iter);
	ex_onode *node;
	if (!now)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at the path");
	node = fz_malloc_struct(ctx, ex_onode);
	fz_try(ctx)
	{
		node->item = *now;
		node->item.title = NULL;
		node->item.uri = NULL;
		node->item.title = fz_strdup(ctx, now->title ? now->title : "");
		if (now->uri)
			node->item.uri = fz_strdup(ctx, now->uri);
		if (depth < 64)
		{
			int at = fz_outline_iterator_down(ctx, iter);
			if (at >= 0)
			{
				ex_onode **tail = &node->down;
				while (at == FZ_OUTLINE_ITERATOR_AT_ITEM)
				{
					*tail = ex_onode_read(ctx, iter, depth + 1);
					tail = &(*tail)->next;
					at = fz_outline_iterator_next(ctx, iter);
				}
				fz_outline_iterator_up(ctx, iter);
			}
		}
	}
	fz_catch(ctx)
	{
		ex_onode_drop(ctx, node);
		fz_rethrow(ctx);
	}
	return node;
}

/* Inserts `node` and everything under it before the iterator's place; leaves the iterator where it was. */
static void ex_onode_write(fz_context *ctx, fz_outline_iterator *iter, const ex_onode *node)
{
	const ex_onode *child;
	fz_outline_iterator_insert(ctx, iter, (fz_outline_item *)&node->item);
	if (!node->down)
		return;
	if (fz_outline_iterator_prev(ctx, iter) != FZ_OUTLINE_ITERATOR_AT_ITEM)
		fz_throw(ctx, FZ_ERROR_GENERIC, "cannot find the outline entry just inserted");
	if (fz_outline_iterator_down(ctx, iter) < 0)
		fz_throw(ctx, FZ_ERROR_GENERIC, "cannot go under the outline entry just inserted");
	for (child = node->down; child; child = child->next)
		ex_onode_write(ctx, iter, child);
	fz_outline_iterator_up(ctx, iter);
	/* A new entry is closed until it has children; now it may be open as it was. */
	fz_outline_iterator_update(ctx, iter, (fz_outline_item *)&node->item);
	fz_outline_iterator_next(ctx, iter);
}

int ex_outline_move(fz_context *ctx, fz_document *doc, const int *from, int from_depth, const int *to, int to_depth,
	ex_error *err)
{
	fz_outline_iterator *iter = NULL;
	ex_onode *node = NULL;
	fz_var(iter);
	fz_var(node);
	fz_try(ctx)
	{
		int at;
		iter = ex_outline_walk(ctx, doc, from, from_depth, &at);
		if (at != FZ_OUTLINE_ITERATOR_AT_ITEM)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no outline entry at the path");
		node = ex_onode_read(ctx, iter, 0);
		fz_outline_iterator_delete(ctx, iter);
		fz_drop_outline_iterator(ctx, iter);
		iter = NULL;
		iter = ex_outline_walk(ctx, doc, to, to_depth, &at);
		ex_onode_write(ctx, iter, node);
	}
	fz_always(ctx)
	{
		ex_onode_drop(ctx, node);
		fz_drop_outline_iterator(ctx, iter);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_link_uri(fz_context *ctx, fz_document *doc, int page, float x, float y, float zoom, char *buf, int size,
	ex_error *err)
{
	char *uri = NULL;
	fz_var(uri);
	if (size < 1)
		return 0;
	buf[0] = 0;
	fz_try(ctx)
	{
		fz_link_dest dest = fz_make_link_dest_xyz(0, page, x, y, zoom * 100);
		uri = fz_format_link_uri(ctx, doc, dest);
		fz_strlcpy(buf, uri, (size_t)size);
	}
	fz_always(ctx)
		fz_free(ctx, uri);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* ---- Saving (K.15) ---- */

int ex_save_pdf(fz_context *ctx, fz_document *doc, const char *path, int *incremental, ex_error *err)
{
	*incremental = 0;
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		if (pdf_can_be_saved_incrementally(ctx, pdf))
		{
			/* Incremental, and the edits stay live for undo and the next save (patch 0088). */
			pdf_save_snapshot(ctx, pdf, path);
			*incremental = 1;
		}
		else
		{
			pdf_write_options opts = pdf_default_write_options;
			pdf_save_document(ctx, pdf, path, &opts);
		}
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* ---- Exchange (A.24): page space for XFDF, and FDF files read and written through MuPDF ---- */

int ex_page_transform(fz_context *ctx, fz_document *doc, int number, fz_matrix *out, ex_error *err)
{
	fz_matrix ctm = fz_identity;
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		pdf_page_obj_transform(ctx, pdf_lookup_page_obj(ctx, pdf, number), NULL, &ctm);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	*out = ctm;
	return 0;
}

/* Whether an annotation dictionary travels in an FDF: not a link, a popup (its parent brings its own) or a field. */
static int ex_fdf_kind(fz_context *ctx, pdf_obj *annot)
{
	pdf_obj *subtype = pdf_dict_get(ctx, annot, PDF_NAME(Subtype));
	return pdf_is_name(ctx, subtype) && !pdf_name_eq(ctx, subtype, PDF_NAME(Link)) &&
		!pdf_name_eq(ctx, subtype, PDF_NAME(Popup)) && !pdf_name_eq(ctx, subtype, PDF_NAME(Widget));
}

int ex_fdf_write(fz_context *ctx, fz_document *doc, fz_buffer **out, ex_error *err)
{
	pdf_document *fdf = NULL;
	pdf_graft_map *map = NULL;
	fz_buffer *buf = NULL;
	fz_output *output = NULL;
	int *from = NULL, *irt = NULL;
	pdf_obj *copy = NULL, *moved = NULL;
	fz_var(fdf);
	fz_var(map);
	fz_var(buf);
	fz_var(output);
	fz_var(from);
	fz_var(irt);
	fz_var(copy);
	fz_var(moved);
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		int pages = pdf_count_pages(ctx, pdf), count = 0, cap = 0, i, k, j;
		fdf = pdf_create_document(ctx);
		map = pdf_new_graft_map(ctx, fdf);
		pdf_obj *root = pdf_dict_get(ctx, pdf_trailer(ctx, fdf), PDF_NAME(Root));
		/* MuPDF has no name constant for /FDF. */
		pdf_obj *body = pdf_new_dict(ctx, fdf, 2);
		pdf_dict_puts_drop(ctx, root, "FDF", body);
		pdf_obj *annots = pdf_dict_put_array(ctx, body, PDF_NAME(Annots), 8);
		for (i = 0; i < pages; i++)
		{
			pdf_obj *list = pdf_dict_get(ctx, pdf_lookup_page_obj(ctx, pdf, i), PDF_NAME(Annots));
			int n = pdf_array_len(ctx, list);
			for (k = 0; k < n; k++)
			{
				pdf_obj *annot = pdf_array_get(ctx, list, k);
				if (!ex_fdf_kind(ctx, annot))
					continue;
				/* A copy without what ties it to this file's pages (/P, its popup), or to another annotation
				 * (/IRT, put back below), so grafting doesn't pull the page tree in. */
				copy = pdf_copy_dict(ctx, annot);
				pdf_dict_del(ctx, copy, PDF_NAME(P));
				pdf_dict_del(ctx, copy, PDF_NAME(Popup));
				pdf_dict_del(ctx, copy, PDF_NAME(IRT));
				pdf_dict_del(ctx, copy, PDF_NAME(Parent));
				moved = pdf_graft_mapped_object(ctx, map, copy);
				pdf_drop_obj(ctx, copy);
				copy = NULL;
				pdf_dict_put_int(ctx, moved, PDF_NAME(Page), i);
				pdf_array_push_drop(ctx, annots, pdf_add_object(ctx, fdf, moved));
				pdf_drop_obj(ctx, moved);
				moved = NULL;
				if (count == cap)
				{
					cap = cap ? cap * 2 : 32;
					from = fz_realloc_array(ctx, from, cap, int);
					irt = fz_realloc_array(ctx, irt, cap, int);
				}
				from[count] = pdf_to_num(ctx, annot);
				irt[count] = pdf_to_num(ctx, pdf_dict_get(ctx, annot, PDF_NAME(IRT)));
				count++;
			}
		}
		/* Each reply's /IRT names its parent's copy. */
		for (i = 0; i < count; i++)
			for (j = 0; irt[i] && j < count; j++)
				if (from[j] == irt[i])
				{
					pdf_dict_put(ctx, pdf_array_get(ctx, annots, i), PDF_NAME(IRT), pdf_array_get(ctx, annots, j));
					break;
				}
		buf = fz_new_buffer(ctx, 4096);
		output = fz_new_output_with_buffer(ctx, buf);
		pdf_write_options opts = pdf_default_write_options;
		pdf_write_document(ctx, fdf, output, &opts);
		fz_close_output(ctx, output);
		/* An FDF is a PDF file with its own header (ISO 32000-1, 12.7.8). */
		if (buf->len >= 5 && memcmp(buf->data, "%PDF-", 5) == 0)
			memcpy(buf->data, "%FDF-", 5);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, copy);
		pdf_drop_obj(ctx, moved);
		fz_drop_output(ctx, output);
		fz_drop_buffer(ctx, buf);
		fz_free(ctx, from);
		fz_free(ctx, irt);
		pdf_drop_graft_map(ctx, map);
		pdf_drop_document(ctx, fdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_fdf_read(fz_context *ctx, fz_document *doc, fz_document *fdf_doc, fz_document **out, ex_error *err)
{
	pdf_document *scratch = NULL;
	pdf_graft_map *map = NULL;
	pdf_obj *page = NULL, *moved = NULL;
	fz_var(scratch);
	fz_var(map);
	fz_var(page);
	fz_var(moved);
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		pdf_document *fdf = pdf_specifics(ctx, fdf_doc);
		if (!fdf)
			fz_throw(ctx, FZ_ERROR_FORMAT, "not an FDF file");
		pdf_obj *annots = pdf_dict_getp(ctx, pdf_trailer(ctx, fdf), "Root/FDF/Annots");
		int pages = pdf_count_pages(ctx, pdf), i, k, n = pdf_array_len(ctx, annots);
		scratch = pdf_create_document(ctx);
		/* Blank pages shaped like the document's, so the annotations read in its page space. */
		for (i = 0; i < pages; i++)
		{
			pdf_obj *from = pdf_lookup_page_obj(ctx, pdf, i);
			fz_rect media = pdf_to_rect(ctx, pdf_dict_get_inheritable(ctx, from, PDF_NAME(MediaBox)));
			int rotate = pdf_to_int(ctx, pdf_dict_get_inheritable(ctx, from, PDF_NAME(Rotate)));
			page = pdf_add_page(ctx, scratch, media, rotate, NULL, NULL);
			pdf_obj *crop = pdf_dict_get_inheritable(ctx, from, PDF_NAME(CropBox));
			if (pdf_is_array(ctx, crop))
				pdf_dict_put_rect(ctx, page, PDF_NAME(CropBox), pdf_to_rect(ctx, crop));
			pdf_obj *unit = pdf_dict_get(ctx, from, PDF_NAME(UserUnit));
			if (pdf_is_number(ctx, unit))
				pdf_dict_put_real(ctx, page, PDF_NAME(UserUnit), pdf_to_real(ctx, unit));
			pdf_insert_page(ctx, scratch, -1, page);
			pdf_drop_obj(ctx, page);
			page = NULL;
		}
		map = pdf_new_graft_map(ctx, scratch);
		for (k = 0; k < n; k++)
		{
			pdf_obj *annot = pdf_array_get(ctx, annots, k);
			int at = pdf_dict_get_int(ctx, annot, PDF_NAME(Page));
			if (at < 0 || at >= pages || !ex_fdf_kind(ctx, annot))
				continue;
			/* Mapped, so a reply's /IRT and its parent come over as one object. */
			moved = pdf_graft_mapped_object(ctx, map, annot);
			if (!pdf_is_indirect(ctx, moved))
			{
				pdf_obj *direct = moved;
				moved = NULL;
				fz_try(ctx)
					moved = pdf_add_object(ctx, scratch, direct);
				fz_always(ctx)
					pdf_drop_obj(ctx, direct);
				fz_catch(ctx)
					fz_rethrow(ctx);
			}
			pdf_obj *target = pdf_lookup_page_obj(ctx, scratch, at);
			pdf_dict_del(ctx, moved, PDF_NAME(Page));
			pdf_dict_del(ctx, moved, PDF_NAME(Popup));
			pdf_dict_put(ctx, moved, PDF_NAME(P), target);
			pdf_obj *list = pdf_dict_get(ctx, target, PDF_NAME(Annots));
			if (!pdf_is_array(ctx, list))
				list = pdf_dict_put_array(ctx, target, PDF_NAME(Annots), 4);
			pdf_array_push(ctx, list, moved);
			pdf_drop_obj(ctx, moved);
			moved = NULL;
		}
		*out = (fz_document *)scratch;
		scratch = NULL;
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, page);
		pdf_drop_obj(ctx, moved);
		pdf_drop_graft_map(ctx, map);
		pdf_drop_document(ctx, scratch);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* ---- Flatten on export (A.25) ---- */

int ex_save_flattened(fz_context *ctx, fz_document *doc, const char *path, int forms, ex_error *err)
{
	fz_buffer *buf = NULL;
	fz_output *output = NULL;
	fz_stream *stream = NULL;
	pdf_document *copy = NULL;
	fz_var(buf);
	fz_var(output);
	fz_var(stream);
	fz_var(copy);
	fz_try(ctx)
	{
		pdf_document *pdf = ex_pdf(ctx, doc);
		/* The document as it is now, edits and all, written to memory and opened again: the copy is baked, never the
		 * open original. */
		buf = fz_new_buffer(ctx, 64 * 1024);
		output = fz_new_output_with_buffer(ctx, buf);
		pdf_write_options opts = pdf_default_write_options;
		pdf_write_document(ctx, pdf, output, &opts);
		fz_close_output(ctx, output);
		stream = fz_open_buffer(ctx, buf);
		copy = pdf_open_document_with_stream(ctx, stream);
		pdf_bake_document(ctx, copy, 1, forms);
		pdf_write_options save = pdf_default_write_options;
		save.do_garbage = 1;
		pdf_save_document(ctx, copy, path, &save);
	}
	fz_always(ctx)
	{
		pdf_drop_document(ctx, copy);
		fz_drop_stream(ctx, stream);
		fz_drop_output(ctx, output);
		fz_drop_buffer(ctx, buf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}
