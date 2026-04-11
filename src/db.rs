use std::collections::HashSet;
use std::path::Path;

use rusqlite::Connection;

use crate::{ExvmsError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub id: i64,
    pub title: String,
    pub recorded_at: f64,
    pub duration: f64,
    pub local_duration: f64,
    pub eviction_date: Option<f64>,
    pub relative_path: String,
    pub folder_id: Option<i64>,
    pub folder_name: Option<String>,
}

pub fn load_recordings(db_path: &Path) -> Result<Vec<Recording>> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    load_recordings_from_connection(&conn)
}

pub fn load_recordings_from_connection(conn: &Connection) -> Result<Vec<Recording>> {
    validate_columns(
        conn,
        "ZCLOUDRECORDING",
        &[
            "Z_PK",
            "ZENCRYPTEDTITLE",
            "ZDATE",
            "ZDURATION",
            "ZLOCALDURATION",
            "ZEVICTIONDATE",
            "ZPATH",
            "ZFOLDER",
        ],
    )?;
    validate_columns(conn, "ZFOLDER", &["Z_PK", "ZENCRYPTEDNAME"])?;

    let mut stmt = conn.prepare(
        r#"
        SELECT
            r.Z_PK,
            COALESCE(r.ZENCRYPTEDTITLE, ''),
            COALESCE(r.ZDATE, 0.0),
            COALESCE(r.ZDURATION, 0.0),
            COALESCE(r.ZLOCALDURATION, 0.0),
            r.ZEVICTIONDATE,
            COALESCE(r.ZPATH, ''),
            r.ZFOLDER,
            f.ZENCRYPTEDNAME
        FROM ZCLOUDRECORDING r
        LEFT JOIN ZFOLDER f ON f.Z_PK = r.ZFOLDER
        ORDER BY r.ZDATE DESC, r.Z_PK DESC
        "#,
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(Recording {
            id: row.get(0)?,
            title: row.get(1)?,
            recorded_at: row.get(2)?,
            duration: row.get(3)?,
            local_duration: row.get(4)?,
            eviction_date: row.get(5)?,
            relative_path: row.get(6)?,
            folder_id: row.get(7)?,
            folder_name: row.get(8)?,
        })
    })?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn validate_columns(
    conn: &Connection,
    table: &'static str,
    required: &[&'static str],
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }

    let missing = required
        .iter()
        .copied()
        .filter(|column| !columns.contains(*column))
        .collect::<Vec<_>>();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(ExvmsError::MissingColumns {
            table,
            columns: missing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_recordings_with_folder_join() {
        let conn = fixture_db();
        let rows = load_recordings_from_connection(&conn).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, 3);
        assert_eq!(rows[1].folder_name.as_deref(), Some("Projects"));
        assert_eq!(rows[2].title, "One");
    }

    pub(crate) fn fixture_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZFOLDER (
                Z_PK INTEGER PRIMARY KEY,
                Z_ENT INTEGER,
                Z_OPT INTEGER,
                ZCOUNTOFRECORDINGS INTEGER,
                ZRANK INTEGER,
                ZENCRYPTEDNAME VARCHAR,
                ZUUID VARCHAR
            );
            CREATE TABLE ZCLOUDRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                Z_ENT INTEGER,
                Z_OPT INTEGER,
                ZFLAGS INTEGER,
                ZSHAREDFLAGS INTEGER,
                ZFOLDER INTEGER,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZEVICTIONDATE TIMESTAMP,
                ZLOCALDURATION FLOAT,
                ZCUSTOMLABEL VARCHAR,
                ZCUSTOMLABELFORSORTING VARCHAR,
                ZENCRYPTEDTITLE VARCHAR,
                ZPATH VARCHAR,
                ZUNIQUEID VARCHAR,
                ZAUDIODIGEST BLOB,
                ZAUDIOFUTURE BLOB,
                ZAUDIOFUTUREUUIDS BLOB,
                ZSILENCEREMOVERENABLED INTEGER,
                ZPLAYBACKRATE FLOAT,
                ZPLAYBACKPOSITION FLOAT,
                ZMTAUDIOFUTURE BLOB,
                ZMTLAYERMIX FLOAT
            );
            INSERT INTO ZFOLDER (Z_PK, ZENCRYPTEDNAME) VALUES (10, 'Projects');
            INSERT INTO ZCLOUDRECORDING
                (Z_PK, ZFOLDER, ZDATE, ZDURATION, ZLOCALDURATION, ZENCRYPTEDTITLE, ZPATH)
            VALUES
                (1, NULL, 10.0, 61.0, 61.0, 'One', 'one.m4a'),
                (2, 10, 20.0, 62.0, 62.0, 'Two', 'two.m4a'),
                (3, NULL, 30.0, 63.0, 0.0, 'Three', 'three.m4a');
            "#,
        )
        .unwrap();
        conn
    }
}
