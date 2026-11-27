-- Reading progress, frequency count, and completed status for the enhanced home page.
ALTER TABLE recent ADD COLUMN total_pages INTEGER NOT NULL DEFAULT 0;
ALTER TABLE recent ADD COLUMN open_count INTEGER NOT NULL DEFAULT 1;
ALTER TABLE recent ADD COLUMN completed INTEGER NOT NULL DEFAULT 0;
