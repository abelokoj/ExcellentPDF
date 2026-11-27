/* EcclesiaPDF's C shim around MuPDF. See shim.h for the conventions. */

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

static ex_system_font_lookup g_system_font_lookup = NULL;

void ex_set_system_font_lookup(ex_system_font_lookup lookup)
{
	g_system_font_lookup = lookup;
}

static fz_font *ex_load_system_font_cb(fz_context *ctx, const char *name, int bold, int italic,
	int needs_exact_metrics)
{
	ex_system_font_lookup lookup = g_system_font_lookup;
	char path[4096];
	int index = 0;

	(void)needs_exact_metrics;
	if (!lookup || !name || !name[0])
		return NULL;
	if (!lookup(name, bold, italic, path, (int)sizeof path, &index))
		return NULL;
	return fz_new_font_from_file(ctx, NULL, path, index, 0);
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
		fz_install_load_system_font_funcs(ctx, ex_load_system_font_cb, NULL, NULL);
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

int ex_style_document(fz_context *ctx, fz_document *doc, int use_document_css, const char *user_css, ex_error *err)
{
	fz_try(ctx)
		fz_style_document(ctx, doc, use_document_css ? 1 : 0, (user_css && user_css[0]) ? user_css : NULL);
	fz_catch(ctx)
		return ex_fail(ctx, err);
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
#define EX_APP_KEY "EcclesiaPDF"
/* The private key holding an ink annotation's point widths (H.9). */
#define EX_WIDTHS_KEY "EcclesiaWidths"

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
	props->multiply = pdf_name_eq(ctx, pdf_dict_get(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(BM)), PDF_NAME(Multiply));
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

int ex_annot_ink_widths(fz_context *ctx, pdf_annot *annot, float *widths, int capacity, int *count, ex_error *err)
{
	fz_try(ctx)
	{
		pdf_obj *list = pdf_dict_gets(ctx, pdf_annot_obj(ctx, annot), EX_WIDTHS_KEY);
		int i, n = pdf_is_array(ctx, list) ? pdf_array_len(ctx, list) : 0;
		*count = n;
		for (i = 0; widths && i < n && i < capacity; i++)
			widths[i] = pdf_array_get_real(ctx, list, i);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
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


/* Writes `w`'s point widths into `annot`, or removes them when it has none (H.9). */
static void ex_ink_widths(fz_context *ctx, pdf_annot *annot, const ex_annot_write *w)
{
	pdf_obj *obj = pdf_annot_obj(ctx, annot);
	pdf_obj *widths;
	int i;
	if (w->width_count <= 0 || !w->widths)
	{
		pdf_dict_dels(ctx, obj, EX_WIDTHS_KEY);
		return;
	}
	widths = pdf_new_array(ctx, pdf_get_bound_document(ctx, obj), w->width_count);
	pdf_dict_puts_drop(ctx, obj, EX_WIDTHS_KEY, widths);
	for (i = 0; i < w->width_count; i++)
		pdf_array_push_real(ctx, widths, w->widths[i]);
}

/* `annot`'s appearance: `w`'s outlines filled in its colour and opacity, multiplied for a highlighter (H.3), in place
 * of MuPDF's constant-width line (H.9). The outlines come in page space; the appearance is drawn in the page's
 * unrotated PDF space, y down as a display list is, and /Rect becomes their bounds, so every reader places it as
 * MuPDF draws it. */
static void ex_ink_appearance(fz_context *ctx, pdf_annot *annot, const ex_annot_write *w)
{
	const ex_annot_props *p = &w->props;
	fz_display_list *list = NULL;
	fz_device *dev = NULL;
	fz_path *path = NULL;
	fz_var(list);
	fz_var(dev);
	fz_var(path);
	if (p->color_n <= 0)
		return;
	fz_try(ctx)
	{
		fz_matrix page_ctm, to_pdf;
		fz_rect bounds = fz_empty_rect;
		fz_rect area;
		fz_colorspace *cs = p->color_n == 1 ? fz_device_gray(ctx) : p->color_n == 4 ? fz_device_cmyk(ctx) : fz_device_rgb(ctx);
		int i, k, at = 0;
		pdf_page_transform(ctx, pdf_annot_page(ctx, annot), NULL, &page_ctm);
		to_pdf = fz_invert_matrix(page_ctm);
		path = fz_new_path(ctx);
		for (i = 0; i < w->outline_count; i++)
		{
			for (k = 0; k < w->outline_lengths[i]; k++)
			{
				fz_point q = fz_transform_point(w->outline[at + k], to_pdf);
				bounds = fz_include_point_in_rect(bounds, q);
				if (k == 0)
					fz_moveto(ctx, path, q.x, -q.y);
				else
					fz_lineto(ctx, path, q.x, -q.y);
			}
			if (w->outline_lengths[i] > 0)
				fz_closepath(ctx, path);
			at += w->outline_lengths[i];
		}
		if (fz_is_empty_rect(bounds))
			break;
		/* The list's bounds are the form's bounding box: the outlines', y down. */
		area = fz_make_rect(bounds.x0, -bounds.y1, bounds.x1, -bounds.y0);
		list = fz_new_display_list(ctx, area);
		dev = fz_new_list_device(ctx, list);
		if (p->multiply)
			fz_begin_group(ctx, dev, area, NULL, 0, 0, FZ_BLEND_MULTIPLY, 1);
		/* Nonzero winding: where a stroke crosses itself it stays filled. */
		fz_fill_path(ctx, dev, path, 0, fz_identity, cs, p->color, p->opacity, fz_default_color_params);
		if (p->multiply)
			fz_end_group(ctx, dev);
		fz_close_device(ctx, dev);
		pdf_dict_put_rect(ctx, pdf_annot_obj(ctx, annot), PDF_NAME(Rect), bounds);
		pdf_set_annot_appearance_from_display_list(ctx, annot, "N", NULL, fz_identity, list);
	}
	fz_always(ctx)
	{
		fz_drop_device(ctx, dev);
		fz_drop_display_list(ctx, list);
		fz_drop_path(ctx, path);
	}
	fz_catch(ctx)
		fz_rethrow(ctx);
}

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
	{
		/* PDF 2.0's /BM, Multiply or none (H.3); the appearance is drawn again when it changes. */
		pdf_obj *obj = pdf_annot_obj(ctx, annot);
		int multiply = pdf_name_eq(ctx, pdf_dict_get(ctx, obj, PDF_NAME(BM)), PDF_NAME(Multiply));
		if (p->multiply && !multiply)
			pdf_dict_put(ctx, obj, PDF_NAME(BM), PDF_NAME(Multiply));
		else if (!p->multiply && multiply)
			pdf_dict_del(ctx, obj, PDF_NAME(BM));
		if (!p->multiply != !multiply)
			pdf_dirty_annot(ctx, annot);
	}
	ex_annot_group(ctx, annot, w->group);
	/* A group's /IRT is the group's (A.2). */
	if (!(w->group && *w->group))
		ex_annot_reply(ctx, annot, w->reply_to, w->state);
	ex_annot_layer(ctx, annot, w->layer);
	if (pdf_annot_type(ctx, annot) == PDF_ANNOT_INK)
		ex_ink_widths(ctx, annot, w);
	/* Last: every setter above stamps the modification date with the time now. */
	if (pdf_annot_has_author(ctx, annot))
		pdf_set_annot_modification_date(ctx, annot, p->modified);
	pdf_update_annot(ctx, annot);
	/* After MuPDF's own: a pressure pen's strokes are its outlines (H.9), drawn again on every write. */
	if (pdf_annot_type(ctx, annot) == PDF_ANNOT_INK && w->outline_count > 0)
		ex_ink_appearance(ctx, annot, w);
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

/* Compatible annotation subtype groups that may convert in-place on update while keeping the
 * same PDF indirect object (preserving /NM id, /IRT reply threads, and /Popup links):
 *   1 = block-highlight / shape geometries (Square, Circle, Polygon)
 *   2 = text markup styles (Highlight, Underline, Squiggly, StrikeOut) */
static int ex_annot_type_group(enum pdf_annot_type type)
{
	switch (type)
	{
	case PDF_ANNOT_SQUARE:
	case PDF_ANNOT_CIRCLE:
	case PDF_ANNOT_POLYGON:
		return 1;
	case PDF_ANNOT_HIGHLIGHT:
	case PDF_ANNOT_UNDERLINE:
	case PDF_ANNOT_SQUIGGLY:
	case PDF_ANNOT_STRIKE_OUT:
		return 2;
	default:
		return 0;
	}
}

int ex_annot_update(fz_context *ctx, fz_document *doc, int page, const ex_annot_write *w, ex_error *err)
{
	pdf_page *p = NULL;
	fz_var(p);
	fz_try(ctx)
	{
		pdf_annot *annot;
		enum pdf_annot_type current, target;
		p = ex_pdf_page(ctx, doc, page);
		annot = ex_find_annot(ctx, p, w->id);
		current = pdf_annot_type(ctx, annot);
		target = (enum pdf_annot_type)w->props.type;
		if (current != target)
		{
			int g_cur = ex_annot_type_group(current);
			int g_dst = ex_annot_type_group(target);
			pdf_obj *obj;
			if (g_cur == 0 || g_cur != g_dst)
				fz_throw(ctx, FZ_ERROR_ARGUMENT, "annotation %s is of another type", w->id);
			obj = pdf_annot_obj(ctx, annot);
			pdf_dict_put_drop(ctx, obj, PDF_NAME(Subtype),
				pdf_new_name(ctx, pdf_string_from_annot_type(ctx, target)));
			pdf_dict_del(ctx, obj, PDF_NAME(Vertices));
			pdf_dict_del(ctx, obj, PDF_NAME(RD));
			pdf_dict_del(ctx, obj, PDF_NAME(BE));
			pdf_dict_del(ctx, obj, PDF_NAME(AP));
			pdf_dirty_annot(ctx, annot);
		}
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


/* The attachment `index` counts to (see ex_attachment_count), kept, and its page (-1 for the document's own); NULL
 * past the last. With `index` -1, counts them into `*count`. */
static pdf_obj *ex_nth_attachment(fz_context *ctx, pdf_document *pdf, int index, int *page_out, int *count)
{
	pdf_obj *tree = NULL;
	pdf_obj *found = NULL;
	pdf_page *page = NULL;
	int at = 0, p, n, i;
	fz_var(tree);
	fz_var(page);
	fz_var(found);
	fz_try(ctx)
	{
		tree = pdf_load_name_tree(ctx, pdf, PDF_NAME(EmbeddedFiles));
		n = pdf_dict_len(ctx, tree);
		for (i = 0; i < n && !found; i++)
		{
			pdf_obj *fs = pdf_dict_get_val(ctx, tree, i);
			if (!pdf_is_filespec(ctx, fs))
				continue;
			if (at++ == index)
			{
				found = pdf_keep_obj(ctx, fs);
				*page_out = -1;
			}
		}
		for (p = 0; p < pdf_count_pages(ctx, pdf) && !found; p++)
		{
			pdf_annot *annot;
			page = pdf_load_page(ctx, pdf, p);
			for (annot = pdf_first_annot(ctx, page); annot && !found; annot = pdf_next_annot(ctx, annot))
			{
				pdf_obj *fs;
				if (pdf_annot_type(ctx, annot) != PDF_ANNOT_FILE_ATTACHMENT)
					continue;
				fs = pdf_annot_filespec(ctx, annot);
				if (!pdf_is_filespec(ctx, fs))
					continue;
				if (at++ == index)
				{
					found = pdf_keep_obj(ctx, fs);
					*page_out = p;
				}
			}
			fz_drop_page(ctx, (fz_page *)page);
			page = NULL;
		}
		if (count)
			*count = at;
	}
	fz_always(ctx)
	{
		fz_drop_page(ctx, (fz_page *)page);
		pdf_drop_obj(ctx, tree);
	}
	fz_catch(ctx)
	{
		pdf_drop_obj(ctx, found);
		fz_rethrow(ctx);
	}
	return found;
}

static pdf_document *ex_attachment_pdf(fz_context *ctx, fz_document *doc)
{
	pdf_document *pdf = pdf_specifics(ctx, doc);
	if (!pdf)
		fz_throw(ctx, FZ_ERROR_ARGUMENT, "only a PDF has attachments");
	return pdf;
}

int ex_attachment_count(fz_context *ctx, fz_document *doc, int *count, ex_error *err)
{
	fz_try(ctx)
	{
		int page;
		*count = 0;
		/* Not a PDF: none. */
		if (pdf_specifics(ctx, doc))
			pdf_drop_obj(ctx, ex_nth_attachment(ctx, ex_attachment_pdf(ctx, doc), -1, &page, count));
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_attachment_info(fz_context *ctx, fz_document *doc, int index, fz_buffer **name, int *size, int *page,
	ex_error *err)
{
	pdf_obj *fs = NULL;
	fz_var(fs);
	*name = NULL;
	fz_try(ctx)
	{
		pdf_filespec_params params;
		fs = ex_nth_attachment(ctx, ex_attachment_pdf(ctx, doc), index, page, NULL);
		if (!fs)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no attachment %d", index);
		pdf_get_filespec_params(ctx, fs, &params);
		*size = params.size;
		*name = fz_new_buffer_from_copied_data(ctx, (const unsigned char *)(params.filename ? params.filename : ""),
			params.filename ? strlen(params.filename) : 0);
	}
	fz_always(ctx)
		pdf_drop_obj(ctx, fs);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_attachment_contents(fz_context *ctx, fz_document *doc, int index, fz_buffer **out, ex_error *err)
{
	pdf_obj *fs = NULL;
	fz_var(fs);
	*out = NULL;
	fz_try(ctx)
	{
		int page;
		fs = ex_nth_attachment(ctx, ex_attachment_pdf(ctx, doc), index, &page, NULL);
		if (!fs)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "no attachment %d", index);
		if (!pdf_is_embedded_file(ctx, fs))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "attachment %d names a file outside the PDF", index);
		*out = pdf_load_embedded_file_contents(ctx, fs);
	}
	fz_always(ctx)
		pdf_drop_obj(ctx, fs);
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_attachment_add(fz_context *ctx, fz_document *doc, const char *name, const unsigned char *data, size_t len,
	ex_error *err)
{
	fz_buffer *buf = NULL;
	pdf_obj *fs = NULL;
	fz_var(buf);
	fz_var(fs);
	fz_try(ctx)
	{
		pdf_document *pdf = ex_attachment_pdf(ctx, doc);
		pdf_obj *root = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
		pdf_obj *names = pdf_dict_get(ctx, root, PDF_NAME(Names));
		pdf_obj *tree, *list;
		int at, n;
		if (!names)
			names = pdf_dict_put_dict(ctx, root, PDF_NAME(Names), 1);
		tree = pdf_dict_get(ctx, names, PDF_NAME(EmbeddedFiles));
		if (!tree)
			tree = pdf_dict_put_dict(ctx, names, PDF_NAME(EmbeddedFiles), 1);
		list = pdf_dict_get(ctx, tree, PDF_NAME(Names));
		if (!list)
		{
			/* A tree of /Kids gets one more leaf of its own, so the others stay as they are. */
			pdf_obj *kids = pdf_dict_get(ctx, tree, PDF_NAME(Kids));
			if (pdf_is_array(ctx, kids))
			{
				pdf_obj *leaf = pdf_add_new_dict(ctx, pdf, 2);
				pdf_obj *limits;
				/* The array keeps the leaf; `leaf` stays valid while it does. */
				pdf_array_push_drop(ctx, kids, leaf);
				list = pdf_dict_put_array(ctx, leaf, PDF_NAME(Names), 2);
				limits = pdf_dict_put_array(ctx, leaf, PDF_NAME(Limits), 2);
				pdf_array_push_text_string(ctx, limits, name);
				pdf_array_push_text_string(ctx, limits, name);
			}
			else
				list = pdf_dict_put_array(ctx, tree, PDF_NAME(Names), 2);
		}
		buf = fz_new_buffer_from_copied_data(ctx, data, len);
		fs = pdf_add_embedded_file(ctx, pdf, name, NULL, buf, -1, -1, 1);
		/* A name tree's keys stay in order. */
		n = pdf_array_len(ctx, list);
		for (at = 0; at + 1 < n; at += 2)
			if (strcmp(pdf_to_text_string(ctx, pdf_array_get(ctx, list, at)), name) > 0)
				break;
		pdf_array_insert_drop(ctx, list, pdf_new_text_string(ctx, name), at);
		pdf_array_insert(ctx, list, fs, at + 1);
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, fs);
		fz_drop_buffer(ctx, buf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_new_pixmap_rgb(fz_context *ctx, int width, int height, fz_pixmap **out, ex_error *err)
{
	*out = NULL;
	fz_try(ctx)
	{
		*out = fz_new_pixmap(ctx, fz_device_rgb(ctx), width, height, NULL, 0);
		fz_clear_pixmap_with_value(ctx, *out, 255);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* ---- PDF redaction, compression, and page operations (plan D3, D4, O.1-O.3, O.7, O.8) ---- */

/* Opens a PDF document from raw bytes in memory and runs pdf_check_document so any repairs settle before editing. */
static pdf_document *ex_open_pdf_from_bytes(fz_context *ctx, const unsigned char *data, size_t len)
{
	fz_stream *stm = NULL;
	pdf_document *pdf = NULL;
	fz_var(stm);
	fz_var(pdf);
	if (!data || len == 0)
		fz_throw(ctx, FZ_ERROR_FORMAT, "empty PDF input");
	fz_try(ctx)
	{
		stm = fz_open_memory(ctx, data, len);
		pdf = pdf_open_document_with_stream(ctx, stm);
		pdf_check_document(ctx, pdf);
	}
	fz_always(ctx)
		fz_drop_stream(ctx, stm);
	fz_catch(ctx)
	{
		pdf_drop_document(ctx, pdf);
		fz_rethrow(ctx);
	}
	return pdf;
}

/* Writes `pdf` to a freshly allocated fz_buffer using `opts`. */
static fz_buffer *ex_write_pdf_to_buffer(fz_context *ctx, pdf_document *pdf, const pdf_write_options *opts)
{
	fz_buffer *buf = NULL;
	fz_output *output = NULL;
	fz_var(buf);
	fz_var(output);
	fz_try(ctx)
	{
		buf = fz_new_buffer(ctx, 16 * 1024);
		output = fz_new_output_with_buffer(ctx, buf);
		pdf_write_document(ctx, pdf, output, opts);
		fz_close_output(ctx, output);
	}
	fz_always(ctx)
		fz_drop_output(ctx, output);
	fz_catch(ctx)
	{
		fz_drop_buffer(ctx, buf);
		fz_rethrow(ctx);
	}
	return buf;
}

static int ex_rects_intersect(fz_rect a, fz_rect b)
{
	return a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0;
}

/* Returns 1 if `ocg` is hidden in the document's default /OCProperties /D configuration. */
static int ex_is_ocg_hidden_default(fz_context *ctx, pdf_obj *props, pdf_obj *ocg)
{
	pdf_obj *d;
	int base_off;
	if (!props || !ocg)
		return 0;
	d = pdf_dict_get(ctx, props, PDF_NAME(D));
	if (!d)
		return 0;
	base_off = pdf_name_eq(ctx, pdf_dict_get(ctx, d, PDF_NAME(BaseState)), PDF_NAME(OFF));
	if (pdf_array_contains(ctx, pdf_dict_get(ctx, d, PDF_NAME(OFF)), ocg))
		return 1;
	if (base_off && !pdf_array_contains(ctx, pdf_dict_get(ctx, d, PDF_NAME(ON)), ocg))
		return 1;
	return 0;
}

int ex_pdf_redact_and_save(fz_context *ctx, const unsigned char *data, size_t len,
	const ex_redact_rect *rects, int rect_count, const ex_redact_flags *flags,
	fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	pdf_page *page = NULL;
	fz_buffer *buf = NULL;
	fz_buffer *cbuf = NULL;
	fz_rect *pdf_rects = NULL;
	pdf_obj *saved_ocprops = NULL;
	fz_var(pdf);
	fz_var(page);
	fz_var(buf);
	fz_var(cbuf);
	fz_var(pdf_rects);
	fz_var(saved_ocprops);
	*out = NULL;
	fz_try(ctx)
	{
		int page_count, i, p;
		pdf_obj *trailer, *root;
		pdf_write_options wopts;

		if (!flags)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "redaction flags must not be NULL");
		if (rect_count < 0 || (rect_count > 0 && !rects))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "invalid redaction rect array");

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		page_count = pdf_count_pages(ctx, pdf);

		for (i = 0; i < rect_count; i++)
		{
			if (rects[i].page < 0 || rects[i].page >= page_count)
				fz_throw(ctx, FZ_ERROR_ARGUMENT, "redaction mark page %d out of range (count %d)",
					rects[i].page, page_count);
		}

		trailer = pdf_trailer(ctx, pdf);
		root = pdf_dict_get(ctx, trailer, PDF_NAME(Root));

		if (flags->scrub_hidden_layers)
		{
			saved_ocprops = pdf_keep_obj(ctx, pdf_dict_get(ctx, root, PDF_NAME(OCProperties)));
		}

		if (flags->scrub_metadata)
		{
			pdf_obj *info = pdf_dict_get(ctx, trailer, PDF_NAME(Info));
			if (pdf_is_dict(ctx, info))
			{
				while (pdf_dict_len(ctx, info) > 0)
					pdf_dict_del(ctx, info, pdf_dict_get_key(ctx, info, 0));
			}
			for (i = 0; i < pdf->num_xref_sections; i++)
			{
				if (pdf->xref_sections[i].trailer)
					pdf_dict_del(ctx, pdf->xref_sections[i].trailer, PDF_NAME(Info));
			}
			pdf_dict_del(ctx, trailer, PDF_NAME(Info));
			pdf_dict_del(ctx, root, PDF_NAME(Metadata));
			pdf_dict_del(ctx, root, PDF_NAME(PieceInfo));
			if (rect_count > 0)
				pdf_dict_del(ctx, root, PDF_NAME(StructTreeRoot));
		}

		if (flags->scrub_attachments)
		{
			pdf_obj *names = pdf_dict_get(ctx, root, PDF_NAME(Names));
			if (pdf_is_dict(ctx, names))
				pdf_dict_del(ctx, names, PDF_NAME(EmbeddedFiles));
			pdf_dict_del(ctx, root, PDF_NAME(AF));
		}

		for (p = 0; p < page_count; p++)
		{
			pdf_obj *page_obj = pdf_lookup_page_obj(ctx, pdf, p);
			int page_rect_count = 0;
			int all_black = 1;
			int need_load = 0;

			if (flags->scrub_metadata)
			{
				pdf_dict_del(ctx, page_obj, PDF_NAME(Thumb));
				pdf_dict_del(ctx, page_obj, PDF_NAME(PieceInfo));
				pdf_dict_del(ctx, page_obj, PDF_NAME(Metadata));
			}
			if (flags->scrub_attachments)
			{
				pdf_dict_del(ctx, page_obj, PDF_NAME(AF));
			}

			for (i = 0; i < rect_count; i++)
			{
				if (rects[i].page == p)
				{
					page_rect_count++;
					if (rects[i].r > 0.001f || rects[i].g > 0.001f || rects[i].b > 0.001f)
						all_black = 0;
				}
			}

			/* Remove hidden-layer or file-attachment annotations directly on page /Annots. */
			{
				pdf_obj *annots = pdf_dict_get(ctx, page_obj, PDF_NAME(Annots));
				int k = 0;
				while (k < pdf_array_len(ctx, annots))
				{
					pdf_obj *a = pdf_array_get(ctx, annots, k);
					pdf_obj *subtype = pdf_dict_get(ctx, a, PDF_NAME(Subtype));
					int drop_it = 0;
					if (flags->scrub_attachments && pdf_name_eq(ctx, subtype, PDF_NAME(FileAttachment)))
						drop_it = 1;
					if (!drop_it && saved_ocprops)
					{
						pdf_obj *oc = pdf_dict_get(ctx, a, PDF_NAME(OC));
						if (oc && ex_is_ocg_hidden_default(ctx, saved_ocprops, oc))
							drop_it = 1;
					}
					if (drop_it)
					{
						pdf_array_delete(ctx, annots, k);
						continue;
					}
					k++;
				}
			}

			if (page_rect_count > 0)
				need_load = 1;

			if (!need_load)
				continue;

			page = pdf_load_page(ctx, pdf, p);
			{
				fz_matrix page_ctm, inv_ctm;
				int idx = 0;
				pdf_page_transform(ctx, page, NULL, &page_ctm);
				inv_ctm = fz_invert_matrix(page_ctm);

				fz_free(ctx, pdf_rects);
				pdf_rects = fz_calloc(ctx, (size_t)page_rect_count, sizeof(fz_rect));
				for (i = 0; i < rect_count; i++)
				{
					if (rects[i].page == p)
						pdf_rects[idx++] = fz_transform_rect(rects[i].rect, inv_ctm);
				}

				/* Scrub overlapping annotations and form widget values before adding Redact annots. */
				if (flags->scrub_overlapping_annots || flags->scrub_overlapping_forms)
				{
					pdf_obj *annots = pdf_dict_get(ctx, page->obj, PDF_NAME(Annots));
					int k = 0;
					while (k < pdf_array_len(ctx, annots))
					{
						pdf_obj *a = pdf_array_get(ctx, annots, k);
						pdf_obj *subtype = pdf_dict_get(ctx, a, PDF_NAME(Subtype));
						fz_rect arect = pdf_dict_get_rect(ctx, a, PDF_NAME(Rect));
						int overlap = 0;
						int r_i;
						for (r_i = 0; r_i < page_rect_count; r_i++)
						{
							if (ex_rects_intersect(arect, pdf_rects[r_i]))
							{
								overlap = 1;
								break;
							}
						}
						if (overlap)
						{
							if (pdf_name_eq(ctx, subtype, PDF_NAME(Widget)))
							{
								if (flags->scrub_overlapping_forms)
								{
									pdf_obj *parent = pdf_dict_get(ctx, a, PDF_NAME(Parent));
									pdf_dict_del(ctx, a, PDF_NAME(V));
									pdf_dict_del(ctx, a, PDF_NAME(DV));
									pdf_dict_del(ctx, a, PDF_NAME(AP));
									if (pdf_is_dict(ctx, parent))
									{
										pdf_dict_del(ctx, parent, PDF_NAME(V));
										pdf_dict_del(ctx, parent, PDF_NAME(DV));
									}
								}
							}
							else if (!pdf_name_eq(ctx, subtype, PDF_NAME(Redact)))
							{
								if (flags->scrub_overlapping_annots)
								{
									pdf_array_delete(ctx, annots, k);
									continue;
								}
							}
						}
						k++;
					}
					pdf_sync_annots(ctx, page);
					pdf_sync_links(ctx, page);
				}

				/* Add one PDF_ANNOT_REDACT per mark on this page using MuPDF's native doc-space rect setter. */
				for (i = 0; i < rect_count; i++)
				{
					if (rects[i].page == p)
					{
						pdf_annot *annot = pdf_create_annot(ctx, page, PDF_ANNOT_REDACT);
						fz_try(ctx)
							pdf_set_annot_rect(ctx, annot, rects[i].rect);
						fz_always(ctx)
							pdf_drop_annot(ctx, annot);
						fz_catch(ctx)
							fz_rethrow(ctx);
					}
				}

				{
					pdf_redact_options ropts;
					memset(&ropts, 0, sizeof ropts);
					ropts.black_boxes = (flags->black_boxes && all_black) ? 1 : 0;
					ropts.image_method = flags->redact_images ? PDF_REDACT_IMAGE_PIXELS : PDF_REDACT_IMAGE_NONE;
					ropts.line_art = flags->redact_vectors ? PDF_REDACT_LINE_ART_REMOVE_IF_TOUCHED : PDF_REDACT_LINE_ART_NONE;
					ropts.text = flags->redact_text ? PDF_REDACT_TEXT_REMOVE : PDF_REDACT_TEXT_NONE;
					pdf_redact_page(ctx, pdf, page, &ropts);
				}

				/* For custom non-black overlay colors, append solid RGB boxes in PDF user space. */
				if (flags->black_boxes && !all_black)
				{
					pdf_obj *contents = pdf_dict_get(ctx, page->obj, PDF_NAME(Contents));
					if (pdf_is_stream(ctx, contents))
					{
						int r_i = 0;
						cbuf = pdf_load_stream(ctx, contents);
						fz_append_byte(ctx, cbuf, '\n');
						for (i = 0; i < rect_count; i++)
						{
							if (rects[i].page == p)
							{
								fz_rect pr = pdf_rects[r_i++];
								float rr = fz_clamp(rects[i].r, 0.0f, 1.0f);
								float gg = fz_clamp(rects[i].g, 0.0f, 1.0f);
								float bb = fz_clamp(rects[i].b, 0.0f, 1.0f);
								fz_append_printf(ctx, cbuf, "q\n%g %g %g rg\n%g %g %g %g re f\nQ\n",
									rr, gg, bb, pr.x0, pr.y0, pr.x1 - pr.x0, pr.y1 - pr.y0);
							}
						}
						pdf_update_stream(ctx, pdf, contents, cbuf, 0);
						fz_drop_buffer(ctx, cbuf);
						cbuf = NULL;
					}
				}
			}

			pdf_drop_page(ctx, page);
			page = NULL;
		}

		if (flags->scrub_hidden_layers)
		{
			pdf_dict_del(ctx, root, PDF_NAME(OCProperties));
			ex_reset_layers(ctx, pdf);
		}

		if (flags->scrub_metadata)
		{
			for (i = 0; i < pdf->num_xref_sections; i++)
			{
				pdf_obj *sec_trailer = pdf->xref_sections[i].trailer;
				if (!sec_trailer)
					continue;
				pdf_obj *sec_info = pdf_dict_get(ctx, sec_trailer, PDF_NAME(Info));
				if (pdf_is_dict(ctx, sec_info))
				{
					while (pdf_dict_len(ctx, sec_info) > 0)
						pdf_dict_del(ctx, sec_info, pdf_dict_get_key(ctx, sec_info, 0));
				}
				pdf_dict_del(ctx, sec_trailer, PDF_NAME(Info));
			}
			pdf_dict_del(ctx, root, PDF_NAME(Metadata));
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_garbage = 4;
		wopts.do_compress = 1;
		wopts.do_compress_images = 1;
		wopts.do_compress_fonts = 1;
		wopts.do_clean = 1;
		wopts.do_sanitize = 1;
		wopts.do_preserve_metadata = flags->scrub_metadata ? 0 : 1;

		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_free(ctx, pdf_rects);
		fz_drop_buffer(ctx, cbuf);
		fz_drop_buffer(ctx, buf);
		pdf_drop_obj(ctx, saved_ocprops);
		pdf_drop_page(ctx, page);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_pdf_info_entries(fz_context *ctx, const unsigned char *data, size_t len, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	fz_buffer *buf = NULL;
	fz_buffer *xmp = NULL;
	fz_var(pdf);
	fz_var(buf);
	fz_var(xmp);
	*out = NULL;
	fz_try(ctx)
	{
		pdf_obj *trailer, *info, *root, *meta;
		int i, n;

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		buf = fz_new_buffer(ctx, 256);
		trailer = pdf_trailer(ctx, pdf);
		info = pdf_dict_get(ctx, trailer, PDF_NAME(Info));
		if (pdf_is_dict(ctx, info))
		{
			n = pdf_dict_len(ctx, info);
			for (i = 0; i < n; i++)
			{
				pdf_obj *k = pdf_dict_get_key(ctx, info, i);
				pdf_obj *v = pdf_dict_get_val(ctx, info, i);
				const char *kname = pdf_to_name(ctx, k);
				const char *vtext = pdf_to_text_string(ctx, v);
				if (kname && *kname && vtext && *vtext)
				{
					fz_append_string(ctx, buf, kname);
					fz_append_byte(ctx, buf, 0);
					fz_append_string(ctx, buf, vtext);
					fz_append_byte(ctx, buf, 0);
				}
			}
		}
		root = pdf_dict_get(ctx, trailer, PDF_NAME(Root));
		meta = pdf_dict_get(ctx, root, PDF_NAME(Metadata));
		if (pdf_is_stream(ctx, meta))
		{
			unsigned char *xmp_data = NULL;
			size_t xmp_len, j;
			xmp = pdf_load_stream(ctx, meta);
			xmp_len = fz_buffer_storage(ctx, xmp, &xmp_data);
			if (xmp_len > 0)
			{
				fz_append_string(ctx, buf, "Metadata");
				fz_append_byte(ctx, buf, 0);
				for (j = 0; j < xmp_len; j++)
					fz_append_byte(ctx, buf, xmp_data[j] == 0 ? ' ' : xmp_data[j]);
				fz_append_byte(ctx, buf, 0);
			}
		}
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_buffer(ctx, xmp);
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

static int ex_count_live_objects(fz_context *ctx, pdf_document *pdf)
{
	int xref_len = pdf_xref_len(ctx, pdf);
	int num, count = 0;
	for (num = 1; num < xref_len; num++)
	{
		if (pdf_object_exists(ctx, pdf, num))
			count++;
	}
	return count;
}

int ex_pdf_compress_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	const ex_compress_flags *flags, ex_compress_stats *stats, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	pdf_document *reopened = NULL;
	fz_stream *reopen_stm = NULL;
	fz_buffer *buf = NULL;
	fz_buffer *img_decoded = NULL;
	fz_buffer *img_scaled = NULL;
	pdf_obj *obj_ref = NULL;
	fz_var(pdf);
	fz_var(reopened);
	fz_var(reopen_stm);
	fz_var(buf);
	fz_var(img_decoded);
	fz_var(img_scaled);
	fz_var(obj_ref);
	*out = NULL;
	if (stats)
		memset(stats, 0, sizeof *stats);
	fz_try(ctx)
	{
		int xref_len, num;
		int objects_before, objects_after, stripped_objs = 0;
		int streams_deflated = 0, images_downsampled = 0;
		pdf_obj *trailer, *root;
		pdf_write_options wopts;

		if (!flags)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "compression flags must not be NULL");

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		objects_before = ex_count_live_objects(ctx, pdf);
		xref_len = pdf_xref_len(ctx, pdf);

		for (num = 1; num < xref_len; num++)
		{
			pdf_obj *obj;
			pdf_obj *type;
			if (!pdf_object_exists(ctx, pdf, num) || !pdf_obj_num_is_stream(ctx, pdf, num))
				continue;
			obj = pdf_load_object(ctx, pdf, num);
			fz_try(ctx)
			{
				type = pdf_dict_get(ctx, obj, PDF_NAME(Type));
				if (pdf_name_eq(ctx, type, PDF_NAME(XRef)) || pdf_name_eq(ctx, type, PDF_NAME(ObjStm)))
				{
					/* Handled natively by MuPDF's full non-incremental writer. */
				}
				else
				{
					int downsampled = 0;
					if (flags->max_image_dim > 0
						&& pdf_name_eq(ctx, pdf_dict_get(ctx, obj, PDF_NAME(Subtype)), PDF_NAME(Image))
						&& !pdf_dict_get_bool(ctx, obj, PDF_NAME(ImageMask))
						&& !pdf_dict_get(ctx, obj, PDF_NAME(SMask)))
					{
						int w = pdf_dict_get_int(ctx, obj, PDF_NAME(Width));
						int h = pdf_dict_get_int(ctx, obj, PDF_NAME(Height));
						int bpc = pdf_dict_get_int_default(ctx, obj, PDF_NAME(BitsPerComponent), 8);
						pdf_obj *cs = pdf_dict_get(ctx, obj, PDF_NAME(ColorSpace));
						int channels = pdf_name_eq(ctx, cs, PDF_NAME(DeviceRGB)) ? 3
							: pdf_name_eq(ctx, cs, PDF_NAME(DeviceGray)) ? 1 : 0;
						if (bpc == 8 && channels > 0 && w > 0 && h > 0
							&& (w > flags->max_image_dim || h > flags->max_image_dim))
						{
							unsigned char *src_px = NULL;
							size_t src_len;
							size_t expected = (size_t)w * (size_t)h * (size_t)channels;
							img_decoded = pdf_load_stream_number(ctx, pdf, num);
							src_len = fz_buffer_storage(ctx, img_decoded, &src_px);
							if (src_len == expected)
							{
								float scale = (float)flags->max_image_dim / (float)(w > h ? w : h);
								int nw = (int)(w * scale + 0.5f);
								int nh = (int)(h * scale + 0.5f);
								int ny, nx;
								if (nw < 1) nw = 1;
								if (nh < 1) nh = 1;
								img_scaled = fz_new_buffer(ctx, (size_t)nw * (size_t)nh * (size_t)channels);
								for (ny = 0; ny < nh; ny++)
								{
									int sy = (int)(((float)ny / (float)nh) * (float)h);
									if (sy >= h) sy = h - 1;
									for (nx = 0; nx < nw; nx++)
									{
										int sx = (int)(((float)nx / (float)nw) * (float)w);
										size_t off;
										if (sx >= w) sx = w - 1;
										off = ((size_t)sy * (size_t)w + (size_t)sx) * (size_t)channels;
										fz_append_data(ctx, img_scaled, src_px + off, (size_t)channels);
									}
								}
								pdf_dict_put_int(ctx, obj, PDF_NAME(Width), nw);
								pdf_dict_put_int(ctx, obj, PDF_NAME(Height), nh);
								pdf_dict_del(ctx, obj, PDF_NAME(DecodeParms));
								obj_ref = pdf_new_indirect(ctx, pdf, num, 0);
								pdf_update_stream(ctx, pdf, obj_ref, img_scaled, 0);
								pdf_drop_obj(ctx, obj_ref);
								obj_ref = NULL;
								images_downsampled++;
								streams_deflated++;
								downsampled = 1;
							}
							fz_drop_buffer(ctx, img_scaled);
							img_scaled = NULL;
							fz_drop_buffer(ctx, img_decoded);
							img_decoded = NULL;
						}
					}
					if (!downsampled && !pdf_dict_get(ctx, obj, PDF_NAME(Filter)))
					{
						int stm_len = pdf_dict_get_int(ctx, obj, PDF_NAME(Length));
						if (stm_len >= 16)
							streams_deflated++;
					}
				}
			}
			fz_always(ctx)
				pdf_drop_obj(ctx, obj);
			fz_catch(ctx)
				fz_rethrow(ctx);
		}

		trailer = pdf_trailer(ctx, pdf);
		root = pdf_dict_get(ctx, trailer, PDF_NAME(Root));
		if (flags->strip_metadata)
		{
			if (pdf_dict_get(ctx, trailer, PDF_NAME(Info)))
			{
				pdf_dict_del(ctx, trailer, PDF_NAME(Info));
				stripped_objs++;
			}
			if (pdf_dict_get(ctx, root, PDF_NAME(Metadata)))
			{
				pdf_dict_del(ctx, root, PDF_NAME(Metadata));
				stripped_objs++;
			}
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_compress = 1;
		wopts.do_compress_images = 1;
		wopts.do_compress_fonts = 1;
		wopts.do_garbage = flags->deduplicate_objects ? 4 : 2;
		wopts.do_clean = flags->clean_streams ? 1 : 0;
		wopts.do_sanitize = flags->clean_streams ? 1 : 0;
		wopts.do_linear = flags->linearize ? 1 : 0;
		wopts.do_preserve_metadata = flags->strip_metadata ? 0 : 1;
		{
			int lvl = fz_clampi(flags->deflate_level, 1, 9);
			wopts.compression_effort = fz_clampi((lvl * 100) / 9, 1, 100);
		}

		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);

		reopen_stm = fz_open_buffer(ctx, buf);
		reopened = pdf_open_document_with_stream(ctx, reopen_stm);
		objects_after = ex_count_live_objects(ctx, reopened);

		if (stats)
		{
			int base_before = objects_before > stripped_objs ? objects_before - stripped_objs : 0;
			stats->objects_before = objects_before;
			stats->objects_after = objects_after;
			stats->deduplicated_objects = (flags->deduplicate_objects && base_before > objects_after)
				? (base_before - objects_after) : 0;
			stats->streams_deflated = streams_deflated;
			stats->images_downsampled = images_downsampled;
		}

		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, obj_ref);
		fz_drop_buffer(ctx, img_scaled);
		fz_drop_buffer(ctx, img_decoded);
		pdf_drop_document(ctx, reopened);
		fz_drop_stream(ctx, reopen_stm);
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

/* Tree node used while remapping/synthesizing outlines in ex_pdf_merge_bytes. */
typedef struct ex_merge_outline {
	char *title;
	int dst_page; /* zero-based page in dst, or -1 */
	char *uri;    /* external URI, or NULL */
	struct ex_merge_outline *down;
	struct ex_merge_outline *next;
} ex_merge_outline;

static void ex_merge_outline_drop(fz_context *ctx, ex_merge_outline *node)
{
	while (node)
	{
		ex_merge_outline *next = node->next;
		ex_merge_outline_drop(ctx, node->down);
		fz_free(ctx, node->title);
		fz_free(ctx, node->uri);
		fz_free(ctx, node);
		node = next;
	}
}

/* Converts a MuPDF fz_outline tree from `src` into remapped ex_merge_outline nodes using `src_to_dst`. */
static ex_merge_outline *ex_remap_outline(fz_context *ctx, fz_document *src_doc, fz_outline *outline,
	const int *src_to_dst, int src_page_count, int depth)
{
	ex_merge_outline *head = NULL;
	ex_merge_outline **tail = &head;
	ex_merge_outline *children = NULL;
	fz_var(head);
	fz_var(children);
	if (depth > 64)
		return NULL;
	fz_try(ctx)
	{
		for (; outline; outline = outline->next)
		{
			int dst_page = -1;
			const char *ext_uri = NULL;
			children = NULL;
			if (outline->page.page >= 0)
			{
				int sp = fz_page_number_from_location(ctx, src_doc, outline->page);
				if (sp >= 0 && sp < src_page_count && src_to_dst[sp] >= 0)
					dst_page = src_to_dst[sp];
			}
			else if (outline->uri && fz_is_external_link(ctx, outline->uri))
			{
				ext_uri = outline->uri;
			}
			if (outline->down)
				children = ex_remap_outline(ctx, src_doc, outline->down, src_to_dst, src_page_count, depth + 1);
			if (dst_page >= 0 || ext_uri || children)
			{
				ex_merge_outline *node = fz_malloc_struct(ctx, ex_merge_outline);
				node->dst_page = dst_page;
				node->down = children;
				children = NULL;
				fz_try(ctx)
				{
					node->title = fz_strdup(ctx, outline->title ? outline->title : "");
					if (ext_uri)
						node->uri = fz_strdup(ctx, ext_uri);
				}
				fz_catch(ctx)
				{
					ex_merge_outline_drop(ctx, node);
					fz_rethrow(ctx);
				}
				*tail = node;
				tail = &node->next;
			}
			else
			{
				ex_merge_outline_drop(ctx, children);
			}
		}
	}
	fz_catch(ctx)
	{
		ex_merge_outline_drop(ctx, head);
		fz_rethrow(ctx);
	}
	return head;
}

/* Emits one sibling level of ex_merge_outline nodes into `dst` under `parent_obj`. Returns total visible count. */
static int ex_emit_outline_level(fz_context *ctx, pdf_document *dst, pdf_obj *parent_obj,
	const ex_merge_outline *nodes, pdf_obj **first_out, pdf_obj **last_out)
{
	const ex_merge_outline *cur;
	pdf_obj *prev = NULL;
	pdf_obj *first = NULL;
	pdf_obj *last = NULL;
	pdf_obj *item = NULL;
	int total = 0;

	fz_var(first);
	fz_var(last);
	fz_var(item);

	for (cur = nodes; cur; cur = cur->next)
	{
		item = pdf_add_new_dict(ctx, dst, 6);
		fz_try(ctx)
		{
			pdf_dict_put_text_string(ctx, item, PDF_NAME(Title), cur->title ? cur->title : "");
			pdf_dict_put(ctx, item, PDF_NAME(Parent), parent_obj);
			if (prev)
			{
				pdf_dict_put(ctx, item, PDF_NAME(Prev), prev);
				pdf_dict_put(ctx, prev, PDF_NAME(Next), item);
			}
			if (cur->dst_page >= 0)
			{
				pdf_obj *page_ref = pdf_lookup_page_obj(ctx, dst, cur->dst_page);
				pdf_obj *dest = pdf_dict_put_array(ctx, item, PDF_NAME(Dest), 2);
				pdf_array_push(ctx, dest, page_ref);
				pdf_array_push(ctx, dest, PDF_NAME(Fit));
			}
			else if (cur->uri)
			{
				pdf_obj *action = pdf_dict_put_dict(ctx, item, PDF_NAME(A), 2);
				pdf_dict_put(ctx, action, PDF_NAME(S), PDF_NAME(URI));
				pdf_dict_put_text_string(ctx, action, PDF_NAME(URI), cur->uri);
			}
			if (cur->down)
			{
				pdf_obj *cfirst = NULL, *clast = NULL;
				int ccount = ex_emit_outline_level(ctx, dst, item, cur->down, &cfirst, &clast);
				if (ccount > 0 && cfirst && clast)
				{
					pdf_dict_put(ctx, item, PDF_NAME(First), cfirst);
					pdf_dict_put(ctx, item, PDF_NAME(Last), clast);
					pdf_dict_put_int(ctx, item, PDF_NAME(Count), ccount);
					total += ccount;
				}
				pdf_drop_obj(ctx, cfirst);
				pdf_drop_obj(ctx, clast);
			}
		}
		fz_catch(ctx)
		{
			pdf_drop_obj(ctx, item);
			pdf_drop_obj(ctx, first);
			pdf_drop_obj(ctx, last);
			fz_rethrow(ctx);
		}
		if (!first)
			first = pdf_keep_obj(ctx, item);
		pdf_drop_obj(ctx, last);
		last = item; /* transfers ownership */
		prev = last;
		total++;
	}

	*first_out = first;
	*last_out = last;
	return total;
}

/* Finds destination page index for a PDF destination object (array `[page_ref ...]` or page index) from `src`. */
static int ex_resolve_src_dest_page(fz_context *ctx, pdf_document *src, pdf_obj *dest,
	const int *src_to_dst, int src_page_count)
{
	if (!pdf_is_array(ctx, dest) || pdf_array_len(ctx, dest) < 1)
		return -1;
	{
		pdf_obj *first = pdf_array_get(ctx, dest, 0);
		int sp = -1;
		if (pdf_is_indirect(ctx, first))
			sp = pdf_lookup_page_number(ctx, src, first);
		else if (pdf_is_int(ctx, first))
			sp = pdf_to_int(ctx, first);
		if (sp >= 0 && sp < src_page_count)
			return src_to_dst[sp];
	}
	return -1;
}

/* Grafts non-Popup annotations (including remapped internal /Link destinations) from `src_page` onto `dst_page`. */
static void ex_graft_page_annots(fz_context *ctx, pdf_document *dst, pdf_graft_map *map,
	pdf_document *src, pdf_obj *src_page, pdf_obj *dst_page,
	const int *src_to_dst, int src_page_count)
{
	pdf_obj *src_annots = pdf_dict_get(ctx, src_page, PDF_NAME(Annots));
	int n = pdf_array_len(ctx, src_annots);
	pdf_obj *dst_annots = NULL;
	int i;
	if (n <= 0)
		return;
	dst_annots = pdf_dict_put_array(ctx, dst_page, PDF_NAME(Annots), n);
	for (i = 0; i < n; i++)
	{
		pdf_obj *a = pdf_array_get(ctx, src_annots, i);
		pdf_obj *subtype;
		pdf_obj *copy = NULL;
		pdf_obj *moved = NULL;
		pdf_obj *ind = NULL;
		if (!pdf_is_dict(ctx, a))
			continue;
		subtype = pdf_dict_get(ctx, a, PDF_NAME(Subtype));
		if (pdf_name_eq(ctx, subtype, PDF_NAME(Popup)))
			continue;
		fz_var(copy);
		fz_var(moved);
		fz_var(ind);
		fz_try(ctx)
		{
			pdf_obj *orig_dest = pdf_dict_get(ctx, a, PDF_NAME(Dest));
			pdf_obj *orig_action = pdf_dict_get(ctx, a, PDF_NAME(A));
			copy = pdf_copy_dict(ctx, a);
			pdf_dict_del(ctx, copy, PDF_NAME(P));
			pdf_dict_del(ctx, copy, PDF_NAME(Popup));
			pdf_dict_del(ctx, copy, PDF_NAME(Parent));
			pdf_dict_del(ctx, copy, PDF_NAME(IRT));
			if (pdf_name_eq(ctx, subtype, PDF_NAME(Link)))
			{
				pdf_dict_del(ctx, copy, PDF_NAME(Dest));
				pdf_dict_del(ctx, copy, PDF_NAME(A));
			}
			moved = pdf_graft_mapped_object(ctx, map, copy);
			pdf_dict_put(ctx, moved, PDF_NAME(P), dst_page);
			if (pdf_name_eq(ctx, subtype, PDF_NAME(Link)))
			{
				int target = -1;
				if (orig_dest)
					target = ex_resolve_src_dest_page(ctx, src, orig_dest, src_to_dst, src_page_count);
				if (target >= 0)
				{
					pdf_obj *darr = pdf_dict_put_array(ctx, moved, PDF_NAME(Dest), 2);
					pdf_array_push(ctx, darr, pdf_lookup_page_obj(ctx, dst, target));
					pdf_array_push(ctx, darr, PDF_NAME(Fit));
				}
				else if (pdf_is_dict(ctx, orig_action))
				{
					pdf_obj *as = pdf_dict_get(ctx, orig_action, PDF_NAME(S));
					if (pdf_name_eq(ctx, as, PDF_NAME(GoTo)))
					{
						int gt = ex_resolve_src_dest_page(ctx, src,
							pdf_dict_get(ctx, orig_action, PDF_NAME(D)), src_to_dst, src_page_count);
						if (gt >= 0)
						{
							pdf_obj *act = pdf_dict_put_dict(ctx, moved, PDF_NAME(A), 2);
							pdf_obj *darr = pdf_dict_put_array(ctx, act, PDF_NAME(D), 2);
							pdf_dict_put(ctx, act, PDF_NAME(S), PDF_NAME(GoTo));
							pdf_array_push(ctx, darr, pdf_lookup_page_obj(ctx, dst, gt));
							pdf_array_push(ctx, darr, PDF_NAME(Fit));
						}
					}
					else if (pdf_name_eq(ctx, as, PDF_NAME(URI)))
					{
						const char *u = pdf_dict_get_text_string(ctx, orig_action, PDF_NAME(URI));
						if (u && *u)
						{
							pdf_obj *act = pdf_dict_put_dict(ctx, moved, PDF_NAME(A), 2);
							pdf_dict_put(ctx, act, PDF_NAME(S), PDF_NAME(URI));
							pdf_dict_put_text_string(ctx, act, PDF_NAME(URI), u);
						}
					}
				}
			}
			ind = pdf_add_object(ctx, dst, moved);
			pdf_array_push(ctx, dst_annots, ind);
		}
		fz_always(ctx)
		{
			pdf_drop_obj(ctx, ind);
			pdf_drop_obj(ctx, moved);
			pdf_drop_obj(ctx, copy);
		}
		fz_catch(ctx)
			fz_rethrow(ctx);
	}
}

int ex_pdf_merge_bytes(fz_context *ctx, const ex_merge_source *sources, int source_count,
	int outline_mode, fz_buffer **out, ex_error *err)
{
	pdf_document *dst = NULL;
	pdf_document *src = NULL;
	pdf_graft_map *map = NULL;
	fz_outline *src_outline = NULL;
	ex_merge_outline *outline_head = NULL;
	ex_merge_outline **outline_tail = &outline_head;
	int *src_to_dst = NULL;
	int *chosen_pages = NULL;
	fz_buffer *buf = NULL;
	fz_var(dst);
	fz_var(src);
	fz_var(map);
	fz_var(src_outline);
	fz_var(outline_head);
	fz_var(src_to_dst);
	fz_var(chosen_pages);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		int s;
		pdf_write_options wopts;

		if (source_count <= 0 || !sources)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "merge requires at least one source PDF");

		dst = pdf_create_document(ctx);

		for (s = 0; s < source_count; s++)
		{
			int src_pages, chosen_count, i, start_dst;
			src = ex_open_pdf_from_bytes(ctx, sources[s].data, sources[s].len);
			src_pages = pdf_count_pages(ctx, src);

			if (sources[s].pages)
			{
				chosen_count = sources[s].page_count;
				for (i = 0; i < chosen_count; i++)
				{
					if (sources[s].pages[i] < 0 || sources[s].pages[i] >= src_pages)
						fz_throw(ctx, FZ_ERROR_ARGUMENT, "merge source page %d out of range (count %d)",
							sources[s].pages[i], src_pages);
				}
			}
			else
			{
				chosen_count = src_pages;
			}

			if (chosen_count <= 0)
			{
				pdf_drop_document(ctx, src);
				src = NULL;
				continue;
			}

			fz_free(ctx, chosen_pages);
			chosen_pages = fz_calloc(ctx, (size_t)chosen_count, sizeof(int));
			for (i = 0; i < chosen_count; i++)
				chosen_pages[i] = sources[s].pages ? sources[s].pages[i] : i;

			fz_free(ctx, src_to_dst);
			src_to_dst = fz_calloc(ctx, (size_t)(src_pages > 0 ? src_pages : 1), sizeof(int));
			for (i = 0; i < src_pages; i++)
				src_to_dst[i] = -1;

			start_dst = pdf_count_pages(ctx, dst);
			map = pdf_new_graft_map(ctx, dst);

			for (i = 0; i < chosen_count; i++)
			{
				int sp = chosen_pages[i];
				int dp = start_dst + i;
				if (src_to_dst[sp] < 0)
					src_to_dst[sp] = dp;
				pdf_graft_mapped_page(ctx, map, -1, src, sp);
			}

			for (i = 0; i < chosen_count; i++)
			{
				pdf_obj *sp_obj = pdf_lookup_page_obj(ctx, src, chosen_pages[i]);
				pdf_obj *dp_obj = pdf_lookup_page_obj(ctx, dst, start_dst + i);
				ex_graft_page_annots(ctx, dst, map, src, sp_obj, dp_obj, src_to_dst, src_pages);
			}

			if (outline_mode != 0)
			{
				ex_merge_outline *remapped = NULL;
				src_outline = fz_load_outline(ctx, (fz_document *)src);
				if (src_outline)
				{
					remapped = ex_remap_outline(ctx, (fz_document *)src, src_outline, src_to_dst, src_pages, 0);
					fz_drop_outline(ctx, src_outline);
					src_outline = NULL;
				}
				if (outline_mode == 2)
				{
					ex_merge_outline *wrapper = fz_malloc_struct(ctx, ex_merge_outline);
					wrapper->dst_page = start_dst;
					wrapper->down = remapped;
					remapped = NULL;
					fz_try(ctx)
					{
						if (sources[s].title && *sources[s].title)
							wrapper->title = fz_strdup(ctx, sources[s].title);
						else
						{
							char fallback[64];
							fz_snprintf(fallback, sizeof fallback, "Document %d", start_dst + 1);
							wrapper->title = fz_strdup(ctx, fallback);
						}
					}
					fz_catch(ctx)
					{
						ex_merge_outline_drop(ctx, wrapper);
						fz_rethrow(ctx);
					}
					*outline_tail = wrapper;
					outline_tail = &wrapper->next;
				}
				else if (remapped)
				{
					*outline_tail = remapped;
					while (*outline_tail)
						outline_tail = &(*outline_tail)->next;
				}
			}

			pdf_drop_graft_map(ctx, map);
			map = NULL;
			pdf_drop_document(ctx, src);
			src = NULL;
		}

		if (pdf_count_pages(ctx, dst) == 0)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "merged PDF contains zero pages");

		if (outline_head)
		{
			pdf_obj *root = pdf_dict_get(ctx, pdf_trailer(ctx, dst), PDF_NAME(Root));
			pdf_obj *outlines = pdf_add_new_dict(ctx, dst, 4);
			pdf_obj *first = NULL, *last = NULL;
			fz_var(first);
			fz_var(last);
			fz_try(ctx)
			{
				int count = ex_emit_outline_level(ctx, dst, outlines, outline_head, &first, &last);
				if (count > 0 && first && last)
				{
					pdf_dict_put(ctx, outlines, PDF_NAME(Type), PDF_NAME(Outlines));
					pdf_dict_put(ctx, outlines, PDF_NAME(First), first);
					pdf_dict_put(ctx, outlines, PDF_NAME(Last), last);
					pdf_dict_put_int(ctx, outlines, PDF_NAME(Count), count);
					pdf_dict_put(ctx, root, PDF_NAME(Outlines), outlines);
					pdf_dict_put(ctx, root, PDF_NAME(PageMode), PDF_NAME(UseOutlines));
				}
			}
			fz_always(ctx)
			{
				pdf_drop_obj(ctx, first);
				pdf_drop_obj(ctx, last);
				pdf_drop_obj(ctx, outlines);
			}
			fz_catch(ctx)
				fz_rethrow(ctx);
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_garbage = 2;
		wopts.do_compress = 1;
		buf = ex_write_pdf_to_buffer(ctx, dst, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_outline(ctx, src_outline);
		ex_merge_outline_drop(ctx, outline_head);
		fz_free(ctx, src_to_dst);
		fz_free(ctx, chosen_pages);
		pdf_drop_graft_map(ctx, map);
		pdf_drop_document(ctx, src);
		pdf_drop_document(ctx, dst);
		fz_drop_buffer(ctx, buf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_pdf_rearrange_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	const int *pages, int page_count, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	unsigned char *seen = NULL;
	pdf_obj *kept_labels = NULL;
	pdf_obj *kept_vp = NULL;
	pdf_obj *kept_pagemode = NULL;
	pdf_obj *kept_meta = NULL;
	pdf_obj *kept_acroform = NULL;
	pdf_obj *kept_lang = NULL;
	fz_buffer *buf = NULL;
	int use_merge = 0;
	fz_var(pdf);
	fz_var(seen);
	fz_var(kept_labels);
	fz_var(kept_vp);
	fz_var(kept_pagemode);
	fz_var(kept_meta);
	fz_var(kept_acroform);
	fz_var(kept_lang);
	fz_var(buf);
	fz_var(use_merge);
	*out = NULL;

	if (page_count <= 0 || !pages)
		return ex_fail_message(FZ_ERROR_ARGUMENT, "page list must not be empty", err);

	fz_try(ctx)
	{
		int total, i, has_dup = 0;
		pdf_obj *oldroot, *newroot;
		pdf_write_options wopts;

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		total = pdf_count_pages(ctx, pdf);
		seen = fz_calloc(ctx, (size_t)(total > 0 ? total : 1), 1);
		for (i = 0; i < page_count; i++)
		{
			if (pages[i] < 0 || pages[i] >= total)
				fz_throw(ctx, FZ_ERROR_ARGUMENT, "page %d out of range (count %d)", pages[i], total);
			if (seen[pages[i]])
				has_dup = 1;
			seen[pages[i]] = 1;
		}

		if (has_dup)
		{
			/* Duplicate page entries require distinct page dicts via grafting (run outside fz_try). */
			use_merge = 1;
		}
		else
		{
			oldroot = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
			kept_labels = pdf_keep_obj(ctx, pdf_dict_get(ctx, oldroot, PDF_NAME(PageLabels)));
			kept_vp = pdf_keep_obj(ctx, pdf_dict_gets(ctx, oldroot, "ViewerPreferences"));
			kept_pagemode = pdf_keep_obj(ctx, pdf_dict_get(ctx, oldroot, PDF_NAME(PageMode)));
			kept_meta = pdf_keep_obj(ctx, pdf_dict_get(ctx, oldroot, PDF_NAME(Metadata)));
			kept_acroform = pdf_keep_obj(ctx, pdf_dict_get(ctx, oldroot, PDF_NAME(AcroForm)));
			kept_lang = pdf_keep_obj(ctx, pdf_dict_get(ctx, oldroot, PDF_NAME(Lang)));

			pdf_rearrange_pages(ctx, pdf, page_count, pages, PDF_CLEAN_STRUCTURE_KEEP);

			newroot = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
			if (kept_labels) pdf_dict_put(ctx, newroot, PDF_NAME(PageLabels), kept_labels);
			if (kept_vp) pdf_dict_puts(ctx, newroot, "ViewerPreferences", kept_vp);
			if (kept_pagemode) pdf_dict_put(ctx, newroot, PDF_NAME(PageMode), kept_pagemode);
			if (kept_meta) pdf_dict_put(ctx, newroot, PDF_NAME(Metadata), kept_meta);
			if (kept_acroform) pdf_dict_put(ctx, newroot, PDF_NAME(AcroForm), kept_acroform);
			if (kept_lang) pdf_dict_put(ctx, newroot, PDF_NAME(Lang), kept_lang);

			wopts = pdf_default_write_options;
			wopts.do_incremental = 0;
			wopts.do_garbage = 2;
			wopts.do_compress = 1;
			buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
			*out = buf;
			buf = NULL;
		}
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, kept_labels);
		pdf_drop_obj(ctx, kept_vp);
		pdf_drop_obj(ctx, kept_pagemode);
		pdf_drop_obj(ctx, kept_meta);
		pdf_drop_obj(ctx, kept_acroform);
		pdf_drop_obj(ctx, kept_lang);
		fz_free(ctx, seen);
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);

	if (use_merge)
	{
		ex_merge_source src_spec;
		src_spec.data = data;
		src_spec.len = len;
		src_spec.title = NULL;
		src_spec.pages = pages;
		src_spec.page_count = page_count;
		return ex_pdf_merge_bytes(ctx, &src_spec, 1, 1, out, err);
	}
	return 0;
}

int ex_pdf_rotate_pages_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	const int *pages, int page_count, int delta_degrees, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	fz_buffer *buf = NULL;
	fz_var(pdf);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		int total, i;
		pdf_write_options wopts;

		if (page_count < 0 || (page_count > 0 && !pages))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "invalid pages array");

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		total = pdf_count_pages(ctx, pdf);
		for (i = 0; i < page_count; i++)
		{
			pdf_obj *pobj;
			int cur, next;
			if (pages[i] < 0 || pages[i] >= total)
				fz_throw(ctx, FZ_ERROR_ARGUMENT, "page %d out of range (count %d)", pages[i], total);
			pobj = pdf_lookup_page_obj(ctx, pdf, pages[i]);
			cur = pdf_dict_get_inheritable_int(ctx, pobj, PDF_NAME(Rotate));
			next = ((cur + delta_degrees) % 360 + 360) % 360;
			pdf_dict_put_int(ctx, pobj, PDF_NAME(Rotate), next);
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_compress = 1;
		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_pdf_insert_blank_page_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	int at_index, float width_pt, float height_pt, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	pdf_obj *page = NULL;
	fz_buffer *buf = NULL;
	fz_var(pdf);
	fz_var(page);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		int total;
		fz_rect media;
		pdf_write_options wopts;

		if (!(width_pt > 0.0f && height_pt > 0.0f))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "blank page dimensions must be positive");

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		total = pdf_count_pages(ctx, pdf);
		if (at_index < 0 || at_index > total)
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "insert index %d exceeds page count %d", at_index, total);

		media = fz_make_rect(0.0f, 0.0f, width_pt, height_pt);
		page = pdf_add_page(ctx, pdf, media, 0, NULL, NULL);
		pdf_insert_page(ctx, pdf, at_index, page);
		pdf_drop_obj(ctx, page);
		page = NULL;

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_compress = 1;
		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		pdf_drop_obj(ctx, page);
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

static void ex_walk_page_labels(fz_context *ctx, pdf_obj *node, fz_buffer *out, int depth)
{
	pdf_obj *nums, *kids;
	int i, n;
	if (!pdf_is_dict(ctx, node) || depth > 32)
		return;
	nums = pdf_dict_get(ctx, node, PDF_NAME(Nums));
	n = pdf_array_len(ctx, nums);
	for (i = 0; i + 1 < n; i += 2)
	{
		int start_page = pdf_array_get_int(ctx, nums, i);
		pdf_obj *entry = pdf_array_get(ctx, nums, i + 1);
		const char *sname;
		const char *prefix;
		int st;
		char style_ch = '-';
		if (!pdf_is_dict(ctx, entry) || start_page < 0)
			continue;
		sname = pdf_to_name(ctx, pdf_dict_get(ctx, entry, PDF_NAME(S)));
		if (sname && (sname[0] == 'D' || sname[0] == 'R' || sname[0] == 'r' || sname[0] == 'A' || sname[0] == 'a') && sname[1] == 0)
			style_ch = sname[0];
		prefix = pdf_dict_get_text_string(ctx, entry, PDF_NAME(P));
		if (!prefix)
			prefix = "";
		st = pdf_dict_get_int_default(ctx, entry, PDF_NAME(St), 1);
		if (st < 1)
			st = 1;
		fz_append_printf(ctx, out, "%d\t%c\t%d\t%s", start_page, style_ch, st, prefix);
		fz_append_byte(ctx, out, 0);
	}
	kids = pdf_dict_get(ctx, node, PDF_NAME(Kids));
	n = pdf_array_len(ctx, kids);
	for (i = 0; i < n; i++)
		ex_walk_page_labels(ctx, pdf_array_get(ctx, kids, i), out, depth + 1);
}

int ex_pdf_read_page_labels(fz_context *ctx, const unsigned char *data, size_t len,
	fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	fz_buffer *buf = NULL;
	fz_var(pdf);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		pdf_obj *root, *labels;
		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		buf = fz_new_buffer(ctx, 128);
		root = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
		labels = pdf_dict_get(ctx, root, PDF_NAME(PageLabels));
		if (labels)
			ex_walk_page_labels(ctx, labels, buf, 0);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_pdf_set_page_labels_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	const ex_page_label_range *ranges, int range_count, fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	fz_buffer *buf = NULL;
	fz_var(pdf);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		pdf_obj *root;
		pdf_write_options wopts;

		if (range_count < 0 || (range_count > 0 && !ranges))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "invalid page labels array");

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		root = pdf_dict_get(ctx, pdf_trailer(ctx, pdf), PDF_NAME(Root));
		if (range_count == 0)
		{
			pdf_dict_del(ctx, root, PDF_NAME(PageLabels));
		}
		else
		{
			pdf_obj *labels = pdf_dict_put_dict(ctx, root, PDF_NAME(PageLabels), 1);
			pdf_obj *nums = pdf_dict_put_array(ctx, labels, PDF_NAME(Nums), range_count * 2);
			int i;
			for (i = 0; i < range_count; i++)
			{
				pdf_obj *entry;
				if (ranges[i].start_number <= 0)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "page label start_number must be >= 1");
				pdf_array_push_int(ctx, nums, ranges[i].start_page);
				entry = pdf_array_push_dict(ctx, nums, 3);
				if (ranges[i].style != 0)
				{
					char sbuf[2];
					sbuf[0] = (char)ranges[i].style;
					sbuf[1] = 0;
					pdf_dict_put_name(ctx, entry, PDF_NAME(S), sbuf);
				}
				if (ranges[i].prefix && *ranges[i].prefix)
					pdf_dict_put_text_string(ctx, entry, PDF_NAME(P), ranges[i].prefix);
				if (ranges[i].start_number != 1)
					pdf_dict_put_int(ctx, entry, PDF_NAME(St), ranges[i].start_number);
			}
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_compress = 1;
		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}

int ex_pdf_set_page_boxes_bytes(fz_context *ctx, const unsigned char *data, size_t len,
	int box_kind, const ex_page_box_edit *edits, int edit_count, int uncrop,
	fz_buffer **out, ex_error *err)
{
	pdf_document *pdf = NULL;
	fz_buffer *buf = NULL;
	fz_var(pdf);
	fz_var(buf);
	*out = NULL;
	fz_try(ctx)
	{
		int total, i;
		pdf_obj *box_key;
		pdf_write_options wopts;

		if (edit_count < 0 || (edit_count > 0 && !edits))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "invalid page box edit array");

		switch (box_kind)
		{
		case 0: box_key = PDF_NAME(CropBox); break;
		case 1: box_key = PDF_NAME(TrimBox); break;
		case 2: box_key = PDF_NAME(BleedBox); break;
		case 3: box_key = PDF_NAME(ArtBox); break;
		default: fz_throw(ctx, FZ_ERROR_ARGUMENT, "unknown page box kind %d", box_kind);
		}

		pdf = ex_open_pdf_from_bytes(ctx, data, len);
		total = pdf_count_pages(ctx, pdf);

		for (i = 0; i < edit_count; i++)
		{
			pdf_obj *pobj;
			if (edits[i].page < 0 || edits[i].page >= total)
				fz_throw(ctx, FZ_ERROR_ARGUMENT, "page %d out of range (count %d)", edits[i].page, total);
			pobj = pdf_lookup_page_obj(ctx, pdf, edits[i].page);
			if (uncrop)
			{
				pdf_obj *orig = pdf_dict_gets(ctx, pobj, "EcclesiaPdfOrigCrop");
				if (orig)
				{
					pdf_obj *kept = pdf_keep_obj(ctx, orig);
					pdf_dict_dels(ctx, pobj, "EcclesiaPdfOrigCrop");
					pdf_dict_put_drop(ctx, pobj, PDF_NAME(CropBox), kept);
				}
				else
				{
					pdf_dict_del(ctx, pobj, PDF_NAME(CropBox));
				}
			}
			else
			{
				if (edits[i].rect.x1 <= edits[i].rect.x0 || edits[i].rect.y1 <= edits[i].rect.y0)
					fz_throw(ctx, FZ_ERROR_ARGUMENT, "crop box must have positive width and height");
				if (box_kind == 0 && !pdf_dict_gets(ctx, pobj, "EcclesiaPdfOrigCrop"))
				{
					pdf_obj *orig_crop = pdf_dict_get_inheritable(ctx, pobj, PDF_NAME(CropBox));
					pdf_obj *orig_media = pdf_dict_get_inheritable(ctx, pobj, PDF_NAME(MediaBox));
					fz_rect orig_rect = orig_crop ? pdf_to_rect(ctx, orig_crop)
						: orig_media ? pdf_to_rect(ctx, orig_media) : edits[i].rect;
					pdf_dict_puts_drop(ctx, pobj, "EcclesiaPdfOrigCrop", pdf_new_rect(ctx, pdf, orig_rect));
				}
				pdf_dict_put_rect(ctx, pobj, box_key, edits[i].rect);
			}
		}

		wopts = pdf_default_write_options;
		wopts.do_incremental = 0;
		wopts.do_compress = 1;
		buf = ex_write_pdf_to_buffer(ctx, pdf, &wopts);
		*out = buf;
		buf = NULL;
	}
	fz_always(ctx)
	{
		fz_drop_buffer(ctx, buf);
		pdf_drop_document(ctx, pdf);
	}
	fz_catch(ctx)
		return ex_fail(ctx, err);
	return 0;
}
