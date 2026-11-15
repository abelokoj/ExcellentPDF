-- Version 2: named tab groups (plan R.13). A tab's group is `workspace_tab.tab_group`; closed tabs keep none.
CREATE TABLE tab_group (
    id INTEGER PRIMARY KEY,
    workspace INTEGER NOT NULL REFERENCES workspace (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX tab_group_by_workspace ON tab_group (workspace, position);

ALTER TABLE workspace_tab ADD COLUMN tab_group INTEGER REFERENCES tab_group (id) ON DELETE SET NULL;
