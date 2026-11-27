-- Version 4: personal bookmarks (B.6), and each file's content (its size and a sample of its bytes, K.5), which
-- finds them again after the file is renamed or moved.
ALTER TABLE file ADD COLUMN content INTEGER;
CREATE INDEX file_by_content ON file (content);

-- A place the reader saved. The place is in page points (`zoom` 1 is 100 %, null in a fit mode); `quote` is the
-- text selected when it was made, which K.28 re-anchors by when the file changes; `created` is in seconds since 1970.
CREATE TABLE bookmark (
    id INTEGER PRIMARY KEY,
    file INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    page INTEGER NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    zoom REAL,
    quote TEXT,
    title TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL,
    created INTEGER NOT NULL
) STRICT;
CREATE INDEX bookmark_by_file ON bookmark (file, page);

CREATE TABLE bookmark_tag (
    bookmark INTEGER NOT NULL REFERENCES bookmark (id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (bookmark, tag)
) STRICT;
