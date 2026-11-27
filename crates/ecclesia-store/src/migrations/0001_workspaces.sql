-- Version 1: files, workspaces and their tabs (plan §11.16, "Storage").

-- A file the app has had open. shortcut: the path alone; the identity of K.5 (size, time, sample) joins it when
-- moved files are found again (W.11, R.10).
CREATE TABLE file (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
) STRICT;

-- A named set of tabs. `active_tab` is the place in the strip of the tab shown, if any.
CREATE TABLE workspace (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    icon TEXT,
    color TEXT,
    position INTEGER NOT NULL,
    layout TEXT,
    active_tab INTEGER
) STRICT;

-- A workspace's open tabs, in strip order. `view` is the app's View state as JSON.
CREATE TABLE workspace_tab (
    id INTEGER PRIMARY KEY,
    workspace INTEGER NOT NULL REFERENCES workspace (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    file INTEGER NOT NULL REFERENCES file (id),
    view TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX workspace_tab_by_workspace ON workspace_tab (workspace, position);

-- A workspace's recently closed tabs: `rank` 0 is the newest, `position` the place it had in the strip.
CREATE TABLE closed_tab (
    id INTEGER PRIMARY KEY,
    workspace INTEGER NOT NULL REFERENCES workspace (id) ON DELETE CASCADE,
    rank INTEGER NOT NULL,
    position INTEGER NOT NULL,
    file INTEGER NOT NULL REFERENCES file (id),
    view TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX closed_tab_by_workspace ON closed_tab (workspace, rank);

-- There is always a workspace: the first, unnamed (the app shows its default name).
INSERT INTO workspace (id, name, position) VALUES (1, '', 0);
