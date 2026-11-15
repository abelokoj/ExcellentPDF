-- Recent and pinned files (R.10): one row per file the reader has shown, with when and the page it was left at.
CREATE TABLE recent (
    file INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    -- When it was last shown, in seconds since 1970.
    seen INTEGER NOT NULL,
    page INTEGER NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX recent_by_seen ON recent (pinned, seen);

-- Facts about the library itself. `home`: the portable app's folder when it last ran, so files that moved with it
-- are found again.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
