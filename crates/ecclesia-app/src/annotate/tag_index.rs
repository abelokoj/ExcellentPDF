//! Keeps the library's index of annotation tags (A.18) current: after a file is saved, its annotations are read on
//! the render workers and their tags written to `library.db`, where a search of the whole library (`tag:`, F.4)
//! finds them without opening every file.
//!
//! shortcut: reads every page's annotations after every save; F.3's indexer, which reads changed pages only, takes
//! this over when it lands.

use std::sync::Arc;

use ecclesia_core::annot::load_all;
use ecclesia_core::document::Document;
use ecclesia_core::shared::pool;
use ecclesia_store::annotation_tags::{self, Tagged};
use nsh_core::prelude::*;

/// Indexes the tags of `document`'s annotations, as it was just saved, in the background.
pub(crate) fn index(document: &Arc<Document>) {
    let Some(store) = crate::library::store() else {
        return;
    };
    let mupdf = match document.mupdf() {
        Ok(mupdf) => mupdf,
        Err(err) => {
            log::warn!(
                "tag index: cannot read {}: {err}",
                document.path().display()
            );
            return;
        }
    };
    let path = document.path().to_path_buf();
    let job = load_all(pool(), mupdf);
    spawn(async move {
        match job.await {
            Ok(annotations) => {
                let tagged: Vec<Tagged> = annotations
                    .into_iter()
                    .filter(|annotation| !annotation.tags.is_empty())
                    .map(|annotation| Tagged {
                        annotation: annotation.id,
                        page: annotation.page,
                        tags: annotation.tags,
                    })
                    .collect();
                let _ = store.write(move |tx| {
                    let result = annotation_tags::index(tx, &path, &tagged);
                    if let Err(err) = &result {
                        // An index a save behind is mended by the next save.
                        log::warn!("tag index: {err}");
                    }
                    result
                });
            }
            Err(err) if !err.is_aborted() => log::warn!("tag index: {err}"),
            Err(_) => {}
        }
    });
}
