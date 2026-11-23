BEGIN IMMEDIATE;
CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
PRAGMA application_id = 1246907980;
PRAGMA user_version = 1;
COMMIT;
