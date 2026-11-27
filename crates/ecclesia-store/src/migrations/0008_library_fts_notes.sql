-- Version 8: full-text FTS5 index (F.2–F.5), library collections/tags/metadata/duplicates/watchers (F.6–F.9),
-- markdown notes & wiki-links (F.12), and workspace rules/templates (W.12).

-- Library metadata, read status, and duplicate-detection fingerprints per tracked file (F.6–F.9).
CREATE TABLE library_item (
    file INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT '',
    authors TEXT NOT NULL DEFAULT '',
    year INTEGER,
    doi TEXT NOT NULL DEFAULT '',
    arxiv_id TEXT NOT NULL DEFAULT '',
    isbn TEXT NOT NULL DEFAULT '',
    read_status TEXT NOT NULL DEFAULT 'unread',
    progress REAL NOT NULL DEFAULT 0.0,
    page_count INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT NOT NULL DEFAULT '',
    minhash_json TEXT NOT NULL DEFAULT '[]',
    file_size INTEGER NOT NULL DEFAULT 0,
    modified_at INTEGER NOT NULL DEFAULT 0,
    added_at INTEGER NOT NULL DEFAULT 0,
    last_opened INTEGER NOT NULL DEFAULT 0,
    meta_json TEXT NOT NULL DEFAULT '{}'
) STRICT;
CREATE INDEX library_item_by_doi ON library_item (doi);
CREATE INDEX library_item_by_hash ON library_item (content_hash);
CREATE INDEX library_item_by_status ON library_item (read_status);

-- Nested collections of library files (F.6, W.10).
CREATE TABLE library_collection (
    id INTEGER PRIMARY KEY,
    parent INTEGER REFERENCES library_collection (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    icon TEXT,
    color TEXT,
    position INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX library_collection_by_parent ON library_collection (parent, position);

-- Files belonging to a collection, keeping explicit item order (W.10).
CREATE TABLE library_collection_item (
    collection INTEGER NOT NULL REFERENCES library_collection (id) ON DELETE CASCADE,
    file INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (collection, file)
) STRICT;
CREATE INDEX library_collection_item_order ON library_collection_item (collection, position);

-- Free-form library tags on files (F.6).
CREATE TABLE library_file_tag (
    file INTEGER NOT NULL REFERENCES file (id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (file, tag)
) STRICT;
CREATE INDEX library_file_tag_by_tag ON library_file_tag (tag COLLATE NOCASE);

-- Folders watched for incremental reconciliation (F.9).
CREATE TABLE watched_folder (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    recursive INTEGER NOT NULL DEFAULT 1,
    last_scan INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Named saved searches / smart folders (F.5).
CREATE TABLE saved_search (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    query TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Backing content table for full-text search across pages, annotations, handwriting, transcripts and notes (F.3).
CREATE TABLE fts_entry (
    id INTEGER PRIMARY KEY,
    file INTEGER REFERENCES file (id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL DEFAULT '',
    page INTEGER NOT NULL DEFAULT 0,
    author TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0,
    quads_json TEXT NOT NULL DEFAULT '[]',
    title TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX fts_entry_by_file ON fts_entry (file, source_kind, page);
CREATE INDEX fts_entry_by_source ON fts_entry (source_kind, source_id);

-- Primary FTS5 word index with Unicode diacritics folding (F.3).
CREATE VIRTUAL TABLE fts_index USING fts5(
    title,
    body,
    author,
    tags,
    content='fts_entry',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Trigram FTS5 table for substring & fuzzy candidate search (F.3, F.4).
CREATE VIRTUAL TABLE fts_trigram USING fts5(
    title,
    body,
    content='fts_entry',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER fts_entry_ai AFTER INSERT ON fts_entry BEGIN
    INSERT INTO fts_index(rowid, title, body, author, tags)
    VALUES (new.id, new.title, new.body, new.author, new.tags);
    INSERT INTO fts_trigram(rowid, title, body)
    VALUES (new.id, new.title, new.body);
END;

CREATE TRIGGER fts_entry_ad AFTER DELETE ON fts_entry BEGIN
    INSERT INTO fts_index(fts_index, rowid, title, body, author, tags)
    VALUES ('delete', old.id, old.title, old.body, old.author, old.tags);
    INSERT INTO fts_trigram(fts_trigram, rowid, title, body)
    VALUES ('delete', old.id, old.title, old.body);
END;

CREATE TRIGGER fts_entry_au AFTER UPDATE ON fts_entry BEGIN
    INSERT INTO fts_index(fts_index, rowid, title, body, author, tags)
    VALUES ('delete', old.id, old.title, old.body, old.author, old.tags);
    INSERT INTO fts_index(rowid, title, body, author, tags)
    VALUES (new.id, new.title, new.body, new.author, new.tags);
    INSERT INTO fts_trigram(fts_trigram, rowid, title, body)
    VALUES ('delete', old.id, old.title, old.body);
    INSERT INTO fts_trigram(rowid, title, body)
    VALUES (new.id, new.title, new.body);
END;

-- Tracks last-indexed file size/mtime so incremental scans skip unchanged documents (F.2, F.3).
CREATE TABLE fts_file_state (
    file INTEGER PRIMARY KEY REFERENCES file (id) ON DELETE CASCADE,
    size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
) STRICT;

-- Markdown notes with optional document/page/annotation anchors (F.12).
CREATE TABLE note (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL UNIQUE COLLATE NOCASE,
    body TEXT NOT NULL DEFAULT '',
    file INTEGER REFERENCES file (id) ON DELETE SET NULL,
    page INTEGER,
    annotation_id TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL
) STRICT;
CREATE INDEX note_by_file ON note (file, page);

-- Outgoing [[wiki-links]] extracted from notes for backlinks and rename propagation (F.12).
CREATE TABLE note_link (
    source_note TEXT NOT NULL REFERENCES note (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    target_raw TEXT NOT NULL,
    target_note TEXT REFERENCES note (id) ON DELETE SET NULL,
    target_file INTEGER REFERENCES file (id) ON DELETE SET NULL,
    target_page INTEGER,
    target_annot TEXT,
    PRIMARY KEY (source_note, position)
) STRICT;
CREATE INDEX note_link_by_target_note ON note_link (target_note);
CREATE INDEX note_link_by_raw ON note_link (target_raw COLLATE NOCASE);

-- Workspace templates and path-based auto-routing rules (W.12).
CREATE TABLE workspace_template (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    icon TEXT,
    color TEXT,
    layout TEXT,
    items_json TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE workspace_rule (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    folder_prefix TEXT NOT NULL,
    workspace_id INTEGER REFERENCES workspace (id) ON DELETE CASCADE,
    workspace_name TEXT NOT NULL DEFAULT '',
    priority INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1
) STRICT;
CREATE INDEX workspace_rule_by_priority ON workspace_rule (enabled, priority DESC);
