-- Version 7: each file's annotation filter (A.19), as the app's own JSON, so the Annotations tab shows the same
-- annotations when the file is opened again.
CREATE TABLE annotation_filter (
    file INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    filter TEXT NOT NULL
) STRICT;
