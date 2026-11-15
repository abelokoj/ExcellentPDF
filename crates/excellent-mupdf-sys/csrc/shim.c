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
	fz_try(ctx)
		pix = fz_new_pixmap_from_page(ctx, page, ctm, fz_device_rgb(ctx), alpha);
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

int ex_new_display_list(fz_context *ctx, fz_page *page, fz_cookie *cookie, fz_display_list **out, ex_error *err)
{
	/* fz_new_display_list_from_page with a cookie. */
	fz_display_list *list = NULL;
	fz_device *dev = NULL;

	fz_var(list);
	fz_var(dev);
	fz_try(ctx)
	{
		list = fz_new_display_list(ctx, fz_bound_page(ctx, page));
		dev = fz_new_list_device(ctx, list);
		fz_run_page(ctx, page, dev, fz_identity, cookie);
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
	int i;

	fz_var(dev);
	fz_try(ctx)
	{
		dev = fz_new_derived_device(ctx, ex_image_device);
		dev->super.fill_image = ex_list_image;
		fz_run_display_list(ctx, list, &dev->super, fz_identity, fz_infinite_rect, cookie);
		fz_close_device(ctx, &dev->super);
	}
	fz_always(ctx)
	{
		/* The items outlive the device: the base device frees only itself. */
		fz_drop_device(ctx, &dev->super);
	}
	fz_catch(ctx)
	{
		if (dev)
		{
			for (i = 0; i < dev->count; i++)
				fz_drop_image(ctx, dev->items[i].image);
			fz_free(ctx, dev->items);
		}
		return ex_fail(ctx, err);
	}
	*out = dev->items;
	*count = dev->count;
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

int ex_load_outline(fz_context *ctx, fz_document *doc, fz_outline **out, ex_error *err)
{
	fz_outline *outline = NULL;
	fz_try(ctx)
		outline = fz_load_outline(ctx, doc);
	fz_catch(ctx)
		return ex_fail(ctx, err);
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
