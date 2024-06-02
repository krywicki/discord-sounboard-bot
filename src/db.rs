use std::borrow::Borrow;
use std::path;

use chrono;
use r2d2_sqlite::rusqlite::OptionalExtension;
use r2d2_sqlite::{
    rusqlite::{self},
    SqliteConnectionManager,
};
use regex::Regex;
use rusqlite::{MappedRows, Row, ToSql};

use crate::audio;
use crate::common::LogResult;

pub struct AudioTableRow {
    pub id: i64,
    pub name: String,
    pub tags: String,
    pub audio_file: audio::AudioFile,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub author_id: Option<u64>,
    pub author_name: Option<String>,
    pub author_global_name: Option<String>,
}

impl TryFrom<&rusqlite::Row<'_>> for AudioTableRow {
    type Error = rusqlite::Error;

    fn try_from(row: &rusqlite::Row) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.get("id").log_err_msg("From row.id fail")?,
            name: row.get("name").log_err_msg("From row.name fail")?,
            tags: row.get("tags").log_err_msg("From row.tags fail")?,
            audio_file: row
                .get("audio_file")
                .log_err_msg("From row.audio_file fail")?,
            created_at: row
                .get("created_at")
                .log_err_msg("From row.created_at fail")?,
            author_id: row
                .get("author_id")
                .log_err_msg("From row.author_id fail")?,
            author_name: row
                .get("author_name")
                .log_err_msg("From row.author_name fail")?,
            author_global_name: row
                .get("author_global_name")
                .log_err_msg("From row.author_global_name fail")?,
        })
    }
}

pub struct AudioTableRowInsert {
    pub name: String,
    pub tags: String,
    pub audio_file: audio::AudioFile,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub author_id: Option<u64>,
    pub author_name: Option<String>,
    pub author_global_name: Option<String>,
}

pub type Connection = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

pub trait FtsCleanText {
    fn fts_clean(&self) -> String;
}

impl FtsCleanText for String {
    fn fts_clean(&self) -> String {
        fts_clean_text(&self)
    }
}

impl<'a> FtsCleanText for &'a str {
    fn fts_clean(&self) -> String {
        fts_clean_text(&self)
    }
}

pub fn fts_clean_text(text: impl AsRef<str>) -> String {
    let text = text.as_ref();

    // Convert words like It's -> Its
    let text = text.replace("'", "");

    // Replace all non alphanumeric & space chars with space char
    let re = Regex::new(r"[^a-zA-Z0-9, ]").unwrap();
    let text = re.replace_all(text.as_str(), " ");

    // Remove replace 2x or more space chars to single space char
    let re = Regex::new(r"\s{2,}").unwrap();
    let text = re.replace_all(text.borrow(), " ");

    text.trim().into()
}

#[derive(Debug)]
pub enum UniqueAudioTableCol {
    Id(i64),
    Name(String),
    AudioFile(String),
}

impl UniqueAudioTableCol {
    pub fn sql_condition(&self) -> String {
        match self {
            Self::Id(id) => format!("id = '{id}' "),
            Self::Name(name) => format!("name = '{name}' "),
            Self::AudioFile(audio_file) => format!("audio_file = '{audio_file}' "),
        }
    }
}

pub trait Table {
    const NAME: &'static str;

    fn connection(&self) -> &Connection;

    fn create_table(&self);

    fn drop_table(&self) {
        match self
            .connection()
            .execute(format!("DROP TABLE {}", Self::NAME).as_str(), ())
        {
            Ok(_) => log::info!("Dropped table: {}", Self::NAME),
            Err(err) => log::error!("Error dropping table: {} - {}", Self::NAME, err),
        }
    }
}

pub struct AudioTable {
    conn: Connection,
}

impl AudioTable {
    pub const DATETIME_FMT: &str = "%Y-%m-%d %H:%M:%SZ";

    pub fn new(connection: Connection) -> Self {
        Self { conn: connection }
    }

    pub fn find_audio_row(&self, col: UniqueAudioTableCol) -> Option<AudioTableRow> {
        let table_name = Self::NAME;

        let sql_condition = col.sql_condition();
        let sql = format!("SELECT * FROM {table_name} WHERE {sql_condition}");

        self.conn
            .query_row(sql.as_str(), (), |row| AudioTableRow::try_from(row))
            .log_err_msg(format!("Failed to find audio row - {col:?}"))
            .ok()
    }

    pub fn insert_audio_row(&self, audio_row: AudioTableRowInsert) -> Result<(), String> {
        log::info!(
            "Inserting audio row. Name: {}, File: {}",
            audio_row.name,
            audio_row.audio_file.to_string_lossy()
        );
        let table_name = Self::NAME;
        let sql = format!(
            "
            INSERT INTO {table_name}
                (name, tags, audio_file, created_at, author_id, author_name, author_global_name)
            VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        );

        let num_inserted = self
            .connection()
            .execute(
                sql.as_str(),
                (
                    &audio_row.name,
                    &audio_row.tags,
                    &audio_row.audio_file,
                    &audio_row.created_at,
                    &audio_row.author_id,
                    &audio_row.author_name,
                    &audio_row.author_global_name,
                ),
            )
            .map_err(|err| {
                log::error!("Failed to insert audio row - {err}");
                err.to_string()
            })?;

        Ok(())
    }

    pub fn has_audio_file(&self, audio_file: &path::PathBuf) -> bool {
        let audio_file = audio_file.to_str().unwrap_or("<?>");

        log::debug!("Checking for existence of audio_file: {}", audio_file);

        let value: rusqlite::Result<String> = self.conn.query_row(
            format!(
                "
                SELECT id FROM {table_name} WHERE audio_file = '{audio_file}'
                ",
                table_name = Self::NAME,
                audio_file = audio_file
            )
            .as_str(),
            (),
            |row| row.get(0),
        );

        match value.optional() {
            Ok(val) => match val {
                Some(v) => {
                    log::debug!("Audio table does not contain audio file: {}", audio_file);
                    true
                }
                None => {
                    log::debug!("Audio table does contain audio file: {}", audio_file);
                    false
                }
            },
            Err(err) => {
                log::error!(
                    "Failed query row on table: {table_name} in has_audio_file",
                    table_name = Self::NAME
                );
                false
            }
        }
    }

    pub fn delete_row_by_audio_file(&self, audio_file: impl AsRef<str>) {
        let audio_file = audio_file.as_ref();
        match self.conn.execute(
            format!(
                "DELETE FROM {table_name} WHERE audio_file = '{audio_file}'",
                table_name = Self::NAME,
                audio_file = audio_file
            )
            .as_str(),
            (),
        ) {
            Ok(_) => {}
            Err(err) => {
                log::error!("Failed to delete row by audio_file = '{}'", audio_file)
            }
        };
    }
}

impl Table for AudioTable {
    const NAME: &'static str = "audio";

    fn connection(&self) -> &Connection {
        &self.conn
    }

    fn create_table(&self) {
        let table_name = Self::NAME;
        let fts5_table_name = format!("fts5_{}", Self::NAME);

        log::info!("Creating tables {table_name}, {fts5_table_name}...");

        let sql = format!(
            "
            BEGIN;
                CREATE TABLE IF NOT EXISTS {table_name} (
                    id INTEGER PRIMARY KEY,
                    name VARCHAR(50) NOT NULL UNIQUE,
                    tags VARCHAR(2048) NOT NULL,
                    audio_file VARCHAR(500) NOT NULL UNIQUE,
                    created_at VARCHAR(25) NOT NULL,
                    user_id INTEGER,
                    user_name VARCHAR(256),
                    user_global_name VARCHAR(256)
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS {fts5_table_name} USING FTS5(
                    name, audio_file, content={table_name}, content_rowid=id
                );

                CREATE TRIGGER IF NOT EXISTS {table_name}_insert AFTER INSERT ON {table_name} BEGIN
                    INSERT INTO {fts5_table_name}(rowid, name, audio_file)
                        VALUES (new.id, new.name, new.audio_file);
                END;

                CREATE TRIGGER IF NOT EXISTS {table_name}_delete AFTER DELETE ON {table_name} BEGIN
                    INSERT INTO {fts5_table_name}({fts5_table_name}, rowid, name, audio_file)
                        VALUES('delete', old.id, old.name, old.audio_file);
                END;

                CREATE TRIGGER {table_name}_update AFTER UPDATE ON {table_name} BEGIN
                    INSERT INTO {fts5_table_name}({fts5_table_name}, rowid, name, audio_file)
                        VALUES('delete', old.id, old.name, old.audio_file);

                    INSERT INTO {fts5_table_name}(rowid, name, audio_file)
                        VALUES (new.id, new.name, new.audio_file);
                END;
            COMMIT;"
        );

        self.conn
            .execute_batch(sql.as_str())
            .log_err_msg(format!("Failed creating table:{table_name}"))
            .unwrap();

        log::info!("Created tables {table_name}, {fts5_table_name}!");
    }
}

pub enum AudioTableOrderBy {
    CreatedAt,
    Id,
    Name,
}

impl AudioTableOrderBy {
    pub fn col_name(&self) -> String {
        match &self {
            Self::CreatedAt => "created_at".into(),
            Self::Id => "id".into(),
            Self::Name => "name".into(),
        }
    }
}

pub struct AudioTablePaginator {
    conn: Connection,
    order_by: AudioTableOrderBy,
    page_limit: u64,
    offset: u64,
}

impl AudioTablePaginator {
    pub fn builder(conn: Connection) -> AudioTablePaginatorBuilder {
        AudioTablePaginatorBuilder::new(conn)
    }

    pub fn next_page(&mut self) -> Result<Vec<AudioTableRow>, String> {
        let conn = &self.conn;
        let table_name = AudioTable::NAME;
        let order_by = self.order_by.col_name();
        let page_limit = self.page_limit;
        let offset = self.offset;

        let sql = format!(
            "SELECT * FROM {table_name}
            ORDER BY {order_by}
            LIMIT {page_limit}
            OFFSET {offset};"
        );

        let mut stmt = conn
            .prepare(sql.as_ref())
            .expect("Failed to prepare sql stmt");

        let row_iter = stmt
            .query_map([], |row| AudioTableRow::try_from(row))
            .map_err(|err| format!("Error in AudioTablePaginator - {err}"))?;

        Ok(row_iter
            .filter_map(|row| match row {
                Ok(val) => Some(val),
                Err(err) => {
                    log::error!("{err}");
                    None
                }
            })
            .collect())
    }
}

pub struct AudioTablePaginatorBuilder {
    conn: Connection,
    order_by: AudioTableOrderBy,
    page_limit: u64,
}

impl AudioTablePaginatorBuilder {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: conn,
            order_by: AudioTableOrderBy::Id,
            page_limit: 500,
        }
    }

    pub fn order_by(mut self, value: AudioTableOrderBy) -> Self {
        self.order_by = value;
        self
    }

    pub fn page_limit(mut self, value: u64) -> Self {
        self.page_limit = value;
        self
    }

    pub fn build(self) -> AudioTablePaginator {
        AudioTablePaginator {
            conn: self.conn,
            order_by: self.order_by,
            page_limit: self.page_limit,
            offset: 0,
        }
    }
}

impl Iterator for AudioTablePaginator {
    type Item = Result<Vec<AudioTableRow>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        let rows = self.next_page();
        let mut is_empty = false;

        match rows {
            Ok(ref _rows) => {
                if _rows.is_empty() {
                    return None;
                } else {
                    return Some(rows);
                }
            }

            Err(err) => {
                log::error!("AudiotablePaginator error - {err}");
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_clean_text_test() {
        assert_eq!("I love star wars", fts_clean_text("I love star-wars!  "));

        assert_eq!(
            "I think its borked",
            fts_clean_text("I think it's borked!?!?!?!?")
        );

        assert_eq!(
            "I like code",
            fts_clean_text("I like !@#$%^&*(_){}[]/\\., code")
        );

        assert_eq!(
            "This is a single line",
            fts_clean_text("This\nis\na\nsingle\nline\n")
        )
    }
}
