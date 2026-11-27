-- Version 6: the tags on each file's annotations (A.18), as the file had them when last saved, for searching the
-- whole library by tag (F.3, F.4). The annotations themselves, and their tags, live in the file; this is an index.
CREATE TABLE annotation_tag (
    file INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    annotation TEXT NOT NULL,
    page INTEGER NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (file, annotation, tag)
) STRICT;
CREATE INDEX annotation_tag_by_tag ON annotation_tag (tag COLLATE NOCASE);
