-- Version 5: split view (V.7). A workspace's tabs, groups and closed tabs each belong to one of its panes (`pane`, 0
-- the first in the layout's reading order); `workspace_pane` keeps each pane's shown tab, and `workspace.layout` the
-- panes' arrangement as the app's JSON. `workspace.active_tab` was pane 0's: it is copied here and no longer read.
ALTER TABLE workspace_tab ADD COLUMN pane INTEGER NOT NULL DEFAULT 0;
ALTER TABLE closed_tab ADD COLUMN pane INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tab_group ADD COLUMN pane INTEGER NOT NULL DEFAULT 0;

CREATE TABLE workspace_pane (
    workspace INTEGER NOT NULL REFERENCES workspace (id) ON DELETE CASCADE,
    pane INTEGER NOT NULL,
    active_tab INTEGER,
    PRIMARY KEY (workspace, pane)
) STRICT;
INSERT INTO workspace_pane (workspace, pane, active_tab) SELECT id, 0, active_tab FROM workspace;
