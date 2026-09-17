CREATE TABLE hxs_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    format_version INTEGER NOT NULL,
    game_version TEXT NOT NULL,
    language TEXT NOT NULL,
    scope TEXT NOT NULL,
    content_id TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    lumina_version TEXT NOT NULL,
    sheet_count INTEGER NOT NULL,
    row_count INTEGER NOT NULL,
    string_cell_count INTEGER NOT NULL
);

CREATE TABLE sheets (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    variant INTEGER NOT NULL CHECK (variant IN (0, 1)),
    effective_language TEXT NOT NULL,
    column_count INTEGER NOT NULL CHECK (column_count >= 0),
    row_count INTEGER NOT NULL CHECK (row_count >= 0),
    schema_hash BLOB NOT NULL,
    technical_hash BLOB NOT NULL,
    string_hash BLOB NOT NULL,
    content_hash BLOB NOT NULL
);

CREATE TABLE columns (
    sheet_id INTEGER NOT NULL,
    column_index INTEGER NOT NULL CHECK (column_index >= 0),
    offset INTEGER NOT NULL CHECK (offset >= 0),
    type INTEGER NOT NULL,
    PRIMARY KEY (sheet_id, column_index),
    FOREIGN KEY (sheet_id) REFERENCES sheets(id)
);

CREATE TABLE "rows" (
    sheet_id INTEGER NOT NULL,
    row_id INTEGER NOT NULL CHECK (row_id >= 0),
    subrow_id INTEGER NOT NULL CHECK (subrow_id >= 0),
    technical_payload BLOB NOT NULL,
    row_hash BLOB NOT NULL,
    technical_hash BLOB NOT NULL,
    string_hash BLOB NOT NULL,
    PRIMARY KEY (sheet_id, row_id, subrow_id),
    FOREIGN KEY (sheet_id) REFERENCES sheets(id)
);

CREATE TABLE string_cells (
    sheet_id INTEGER NOT NULL,
    row_id INTEGER NOT NULL,
    subrow_id INTEGER NOT NULL,
    column_index INTEGER NOT NULL,
    macro_text TEXT NOT NULL,
    raw_value BLOB,
    macro_hash BLOB NOT NULL,
    raw_hash BLOB,
    PRIMARY KEY (sheet_id, row_id, subrow_id, column_index),
    FOREIGN KEY (sheet_id, row_id, subrow_id)
        REFERENCES "rows" (sheet_id, row_id, subrow_id)
);
