use anyhow::{Context, Result, ensure};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use std::{fs, io::{ErrorKind, Read}, path::Path};
use tempfile::NamedTempFile;

pub enum Kind { Missing, Plain, Locked }

pub fn inspect(path: &Path) -> Result<Kind> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Kind::Missing),
        Err(e) => return Err(e.into()),
    };
    let mut header = [0; 16];
    file.read_exact(&mut header).context("Journal file is incomplete")?;
    Ok(if &header == b"SQLite format 3\0" { Kind::Plain } else { Kind::Locked })
}

pub struct Entry {
    pub id: i64,
    pub content: String,
    pub created: i64,
    pub updated: i64,
}

pub struct Store(Connection);

impl Store {
    fn connect(path: &Path, password: Option<&str>) -> Result<Connection> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        if let Some(password) = password {
            ensure!(!password.is_empty(), "Enter a passphrase.");
            conn.pragma_update(None, "key", password)
                .map_err(|_| anyhow::anyhow!("Could not apply the journal key"))?;
            // Verify the codec is present before ever writing a new database.
            let version: String = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
            ensure!(!version.is_empty(), "SQLCipher is unavailable");
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .context("Cannot read journal. Check the passphrase and file integrity")?;
        conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA synchronous=FULL;")?;
        Ok(conn)
    }

    pub fn create(path: &Path, password: Option<&str>) -> Result<Self> {
        let dir = path.parent().context("Missing database directory")?;
        fs::create_dir_all(dir)?;
        let staging = NamedTempFile::new_in(dir)?;
        let conn = Self::connect(staging.path(), password)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.close().map_err(|(_, e)| e)?;
        let expected_header = matches!(
            (password.is_some(), inspect(staging.path())?),
            (true, Kind::Locked) | (false, Kind::Plain)
        );
        ensure!(expected_header, "Database format differs from the requested storage mode");
        staging.as_file().sync_all()?;
        staging.persist_noclobber(path).context("Could not install new journal")?;
        Self::open(path, password)
    }

    pub fn open(path: &Path, password: Option<&str>) -> Result<Self> {
        let conn = Self::connect(path, password)?;
        let identity: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        ensure!(identity == 1246907980, "This file is not a Journal database");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version == 1, "Unsupported journal schema version {version}");
        Ok(Self(conn))
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.0.prepare(
            "SELECT id, content, created_at, updated_at FROM entries ORDER BY created_at DESC, id DESC"
        )?;
        let rows = stmt.query_map([], |r| Ok(Entry {
            id: r.get(0)?, content: r.get(1)?, created: r.get(2)?, updated: r.get(3)?,
        }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn insert(&self) -> Result<Entry> {
        let now = Utc::now().timestamp();
        self.0.execute("INSERT INTO entries(content,created_at,updated_at) VALUES('',?1,?1)", [now])?;
        Ok(Entry { id: self.0.last_insert_rowid(), content: String::new(), created: now, updated: now })
    }

    pub fn save(&self, entry: &mut Entry, text: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let count = self.0.execute(
            "UPDATE entries SET content=?1, updated_at=?2 WHERE id=?3 AND content=?4",
            (text, now, entry.id, &entry.content)
        )?;
        ensure!(count == 1, "Entry changed in another window. Copy your draft before reloading.");
        entry.content = text.to_owned();
        entry.updated = now;
        Ok(())
    }

    pub fn delete(&self, entry: &Entry) -> Result<()> {
        let count = self.0.execute("DELETE FROM entries WHERE id=?1 AND content=?2", (entry.id, &entry.content))?;
        ensure!(count == 1, "Entry changed in another window. Reopen the journal to refresh.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_journal_requires_original_passphrase() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("journal.db");
        let key = "rain's quiet 夜";
        let secret = "an identifiable journal sentence";
        {
            let store = Store::create(&file, Some(key))?;
            let mut entry = store.insert()?;
            store.save(&mut entry, secret)?;
        }
        assert!(matches!(inspect(&file)?, Kind::Locked));
        let bytes = fs::read(&file)?;
        assert!(!bytes.windows(secret.len()).any(|s| s == secret.as_bytes()));
        assert!(Store::open(&file, None).is_err());
        assert!(Store::open(&file, Some("wrong")).is_err());
        assert_eq!(fs::read(&file)?, bytes);
        assert_eq!(Store::open(&file, Some(key))?.entries()?[0].content, secret);
        Ok(())
    }

    #[test]
    fn failed_setup_leaves_path_available_and_existing_file_is_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("journal.db");
        assert!(Store::create(&file, Some("")).is_err());
        assert!(matches!(inspect(&file)?, Kind::Missing));
        fs::write(&file, b"existing unrelated file")?;
        assert!(Store::create(&file, None).is_err());
        assert_eq!(fs::read(file)?, b"existing unrelated file");
        Ok(())
    }

    #[test]
    fn plain_journal_reopens_and_stale_edits_are_detected() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("journal.db");
        let first = Store::create(&file, None)?;
        assert!(matches!(inspect(&file)?, Kind::Plain));
        let mut entry = first.insert()?;
        let second = Store::open(&file, None)?;
        let mut stale = second.entries()?.remove(0);
        first.save(&mut entry, "new text")?;
        assert!(second.save(&mut stale, "stale edit").is_err());
        assert!(second.delete(&stale).is_err());
        assert_eq!(second.entries()?[0].content, "new text");
        first.delete(&entry)?;
        assert!(second.entries()?.is_empty());
        Ok(())
    }
}
