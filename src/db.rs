use std::collections::HashSet;
use std::path::Path;

use rusqlite::Connection;

use crate::{ExvmsError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStoreKind {
    CloudRecordings,
    LocalRecordings,
}

impl RecordingStoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloudRecordings => "cloud-recordings",
            Self::LocalRecordings => "local-recordings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingPathKind {
    Relative,
    Absolute,
    Missing,
}

impl RecordingPathKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relative => "relative",
            Self::Absolute => "absolute",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    ModernCloud,
    LegacyCloud,
    LegacyLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseProbe {
    pub kind: DatabaseKind,
    pub row_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub id: i64,
    pub title: String,
    pub recorded_at: f64,
    pub duration: f64,
    pub local_duration: f64,
    pub eviction_date: Option<f64>,
    pub db_path: String,
    pub path_kind: RecordingPathKind,
    pub store_kind: RecordingStoreKind,
    pub folder_id: Option<i64>,
    pub folder_name: Option<String>,
}

pub fn probe_database(db_path: &Path) -> Result<Option<DatabaseProbe>> {
    let conn = open_readonly(db_path)?;
    probe_connection(&conn)
}

pub fn load_recordings(db_path: &Path) -> Result<Vec<Recording>> {
    let conn = open_readonly(db_path)?;
    load_recordings_from_connection(&conn)
}

pub fn load_recordings_from_connection(conn: &Connection) -> Result<Vec<Recording>> {
    let Some(probe) = probe_connection(conn)? else {
        return Err(ExvmsError::UnsupportedDatabase(
            "no supported Voice Memos recording table was found".to_string(),
        ));
    };

    match probe.kind {
        DatabaseKind::ModernCloud | DatabaseKind::LegacyCloud => {
            load_cloud_recordings(conn, probe.kind)
        }
        DatabaseKind::LegacyLocal => load_legacy_local_recordings(conn),
    }
}

fn open_readonly(db_path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?)
}

fn probe_connection(conn: &Connection) -> Result<Option<DatabaseProbe>> {
    let cloud = probe_cloud_table(conn)?;
    let legacy = probe_legacy_local_table(conn)?;

    match (cloud, legacy) {
        (Some(cloud), Some(legacy)) if cloud.row_count == 0 && legacy.row_count > 0 => {
            Ok(Some(legacy))
        }
        (Some(cloud), _) => Ok(Some(cloud)),
        (None, Some(legacy)) => Ok(Some(legacy)),
        (None, None) => Ok(None),
    }
}

fn probe_cloud_table(conn: &Connection) -> Result<Option<DatabaseProbe>> {
    let columns = table_columns(conn, "ZCLOUDRECORDING")?;
    if columns.is_empty() {
        return Ok(None);
    }

    let required = ["Z_PK", "ZDATE", "ZDURATION", "ZPATH"];
    let missing = required
        .iter()
        .copied()
        .filter(|column| !columns.contains(*column))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(ExvmsError::MissingColumns {
            table: "ZCLOUDRECORDING",
            columns: missing,
        });
    }

    let kind = if columns.contains("ZLOCALDURATION") && columns.contains("ZENCRYPTEDTITLE") {
        DatabaseKind::ModernCloud
    } else {
        DatabaseKind::LegacyCloud
    };
    Ok(Some(DatabaseProbe {
        kind,
        row_count: count_rows(conn, "ZCLOUDRECORDING")?,
    }))
}

fn probe_legacy_local_table(conn: &Connection) -> Result<Option<DatabaseProbe>> {
    let columns = table_columns(conn, "ZRECORDING")?;
    if columns.is_empty() {
        return Ok(None);
    }

    let required = ["Z_PK", "ZDATE", "ZDURATION", "ZPATH"];
    let missing = required
        .iter()
        .copied()
        .filter(|column| !columns.contains(*column))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(ExvmsError::MissingColumns {
            table: "ZRECORDING",
            columns: missing,
        });
    }

    Ok(Some(DatabaseProbe {
        kind: DatabaseKind::LegacyLocal,
        row_count: count_rows(conn, "ZRECORDING")?,
    }))
}

fn load_cloud_recordings(conn: &Connection, kind: DatabaseKind) -> Result<Vec<Recording>> {
    let recording_columns = table_columns(conn, "ZCLOUDRECORDING")?;
    let folder_columns = table_columns(conn, "ZFOLDER")?;
    let has_folder_join = recording_columns.contains("ZFOLDER")
        && folder_columns.contains("Z_PK")
        && folder_columns.contains("ZENCRYPTEDNAME");

    let encrypted_title_expr = optional_column_expr(&recording_columns, "ZENCRYPTEDTITLE");
    let custom_label_expr = optional_column_expr(&recording_columns, "ZCUSTOMLABEL");
    let unique_id_expr = optional_column_expr(&recording_columns, "ZUNIQUEID");
    let local_duration_expr = optional_column_expr(&recording_columns, "ZLOCALDURATION");
    let eviction_date_expr = optional_column_expr(&recording_columns, "ZEVICTIONDATE");
    let folder_id_expr = optional_column_expr(&recording_columns, "ZFOLDER");
    let folder_name_expr = if has_folder_join {
        "f.ZENCRYPTEDNAME".to_string()
    } else {
        "NULL".to_string()
    };
    let join_expr = if has_folder_join {
        "LEFT JOIN ZFOLDER f ON f.Z_PK = r.ZFOLDER"
    } else {
        ""
    };

    let sql = format!(
        r#"
        SELECT
            r.Z_PK,
            {encrypted_title_expr},
            {custom_label_expr},
            {unique_id_expr},
            COALESCE(r.ZPATH, ''),
            COALESCE(r.ZDATE, 0.0),
            COALESCE(r.ZDURATION, 0.0),
            {local_duration_expr},
            {eviction_date_expr},
            {folder_id_expr},
            {folder_name_expr}
        FROM ZCLOUDRECORDING r
        {join_expr}
        ORDER BY r.ZDATE DESC, r.Z_PK DESC
        "#
    );

    let store_kind = match kind {
        DatabaseKind::ModernCloud | DatabaseKind::LegacyCloud => {
            RecordingStoreKind::CloudRecordings
        }
        DatabaseKind::LegacyLocal => unreachable!("legacy local recordings use ZRECORDING"),
    };

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id = row.get(0)?;
        let encrypted_title = row.get(1)?;
        let custom_label = row.get(2)?;
        let unique_id = row.get(3)?;
        let raw_path: String = row.get(4)?;
        let duration = row.get(6)?;
        let local_duration = row
            .get::<_, Option<f64>>(7)?
            .filter(|value| *value > 0.0)
            .unwrap_or(if duration > 0.0 { duration } else { 0.0 });

        Ok(Recording {
            id,
            title: title_from_fields(id, encrypted_title, custom_label, unique_id, &raw_path),
            recorded_at: row.get(5)?,
            duration,
            local_duration,
            eviction_date: row.get(8)?,
            db_path: raw_path.clone(),
            path_kind: classify_path_kind(&raw_path),
            store_kind,
            folder_id: row.get(9)?,
            folder_name: row.get(10)?,
        })
    })?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn load_legacy_local_recordings(conn: &Connection) -> Result<Vec<Recording>> {
    let recording_columns = table_columns(conn, "ZRECORDING")?;
    let custom_label_expr = optional_column_expr(&recording_columns, "ZCUSTOMLABEL");
    let unique_id_expr = optional_column_expr(&recording_columns, "ZUNIQUEID");

    let sql = format!(
        r#"
        SELECT
            r.Z_PK,
            {custom_label_expr},
            {unique_id_expr},
            COALESCE(r.ZPATH, ''),
            COALESCE(r.ZDATE, 0.0),
            COALESCE(r.ZDURATION, 0.0)
        FROM ZRECORDING r
        ORDER BY r.ZDATE DESC, r.Z_PK DESC
        "#
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id = row.get(0)?;
        let custom_label = row.get(1)?;
        let unique_id = row.get(2)?;
        let raw_path: String = row.get(3)?;
        let duration = row.get(5)?;

        Ok(Recording {
            id,
            title: title_from_fields(id, None, custom_label, unique_id, &raw_path),
            recorded_at: row.get(4)?,
            duration,
            local_duration: if duration > 0.0 { duration } else { 0.0 },
            eviction_date: None,
            db_path: raw_path.clone(),
            path_kind: classify_path_kind(&raw_path),
            store_kind: RecordingStoreKind::LocalRecordings,
            folder_id: None,
            folder_name: None,
        })
    })?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn optional_column_expr(columns: &HashSet<String>, column: &str) -> String {
    if columns.contains(column) {
        format!("r.{column}")
    } else {
        "NULL".to_string()
    }
}

fn title_from_fields(
    id: i64,
    encrypted_title: Option<String>,
    custom_label: Option<String>,
    unique_id: Option<String>,
    raw_path: &str,
) -> String {
    for value in [encrypted_title, custom_label, unique_id] {
        if let Some(value) = value.map(|value| value.trim().to_string())
            && !value.is_empty()
        {
            return value;
        }
    }

    if let Some(file_name) = Path::new(raw_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return file_name.to_string();
    }

    format!("Untitled {id}")
}

fn classify_path_kind(raw_path: &str) -> RecordingPathKind {
    if raw_path.trim().is_empty() {
        RecordingPathKind::Missing
    } else if Path::new(raw_path).is_absolute() {
        RecordingPathKind::Absolute
    } else {
        RecordingPathKind::Relative
    }
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

fn count_rows(conn: &Connection, table: &str) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |row| row.get::<_, i64>(0))? as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_modern_recordings_with_folder_join() {
        let conn = modern_fixture_db();
        let rows = load_recordings_from_connection(&conn).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, 3);
        assert_eq!(rows[0].store_kind, RecordingStoreKind::CloudRecordings);
        assert_eq!(rows[1].folder_name.as_deref(), Some("Projects"));
        assert_eq!(rows[2].title, "One");
    }

    #[test]
    fn loads_legacy_cloud_recordings_with_custom_label_and_duration_fallback() {
        let conn = legacy_cloud_fixture_db();
        let rows = load_recordings_from_connection(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Legacy Title");
        assert_eq!(rows[0].local_duration, 9.0);
        assert_eq!(rows[0].path_kind, RecordingPathKind::Absolute);
        assert_eq!(rows[0].store_kind, RecordingStoreKind::CloudRecordings);
        assert_eq!(rows[1].title, "legacy-empty.m4a");
        assert_eq!(rows[1].path_kind, RecordingPathKind::Missing);
    }

    #[test]
    fn loads_legacy_local_recordings_from_recordings_table() {
        let conn = legacy_local_fixture_db();
        let rows = load_recordings_from_connection(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Local Legacy");
        assert_eq!(rows[0].store_kind, RecordingStoreKind::LocalRecordings);
        assert_eq!(rows[0].path_kind, RecordingPathKind::Absolute);
    }

    #[test]
    fn cloud_table_with_no_rows_takes_precedence_over_empty_legacy_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZCLOUDRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZLOCALDURATION FLOAT,
                ZENCRYPTEDTITLE VARCHAR,
                ZPATH VARCHAR
            );
            CREATE TABLE ZRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZPATH VARCHAR
            );
            "#,
        )
        .unwrap();
        let probe = probe_connection(&conn).unwrap().unwrap();
        assert_eq!(probe.kind, DatabaseKind::ModernCloud);
        assert_eq!(probe.row_count, 0);
    }

    #[test]
    fn non_empty_legacy_local_table_wins_when_cloud_table_is_empty() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZCLOUDRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZLOCALDURATION FLOAT,
                ZENCRYPTEDTITLE VARCHAR,
                ZPATH VARCHAR
            );
            CREATE TABLE ZRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZCUSTOMLABEL VARCHAR,
                ZPATH VARCHAR
            );
            INSERT INTO ZRECORDING
                (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZPATH)
            VALUES
                (1, 10.0, 11.0, 'Old', '/Users/example/old.m4a');
            "#,
        )
        .unwrap();
        let probe = probe_connection(&conn).unwrap().unwrap();
        assert_eq!(probe.kind, DatabaseKind::LegacyLocal);
        assert_eq!(probe.row_count, 1);
    }

    pub(crate) fn modern_fixture_db() -> Connection {
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

    fn legacy_cloud_fixture_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZCLOUDRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                Z_ENT INTEGER,
                Z_OPT INTEGER,
                ZFLAGS INTEGER,
                ZITUNESPERSISTENTID INTEGER,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZEVICTIONDATE TIMESTAMP,
                ZCKRECORDID VARCHAR,
                ZCUSTOMLABEL VARCHAR,
                ZPATH VARCHAR,
                ZUNIQUEID VARCHAR,
                ZAUDIODIGEST BLOB,
                ZAUDIOFUTURE BLOB,
                ZCKRECORDSYSTEMFIELDS BLOB,
                ZENCRYPTEDTITLE VARCHAR,
                ZFOLDER INTEGER,
                ZSHAREDFLAGS INTEGER
            );
            INSERT INTO ZCLOUDRECORDING
                (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZPATH, ZUNIQUEID, ZENCRYPTEDTITLE)
            VALUES
                (1, 20.0, 9.0, 'Legacy Title', '/Users/example/Library/Application Support/com.apple.voicememos/Recordings/legacy.m4a', 'uuid-1', ''),
                (2, 10.0, -1.0, '', '', 'legacy-empty.m4a', '');
            "#,
        )
        .unwrap();
        conn
    }

    fn legacy_local_fixture_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                Z_ENT INTEGER,
                Z_OPT INTEGER,
                ZITUNESPERSISTENTID INTEGER,
                ZLABELPRESET INTEGER,
                ZPENDINGRESTORE INTEGER,
                ZRECORDINGID INTEGER,
                ZSYNCED INTEGER,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZCUSTOMLABEL VARCHAR,
                ZPATH VARCHAR,
                ZUNIQUEID VARCHAR
            );
            INSERT INTO ZRECORDING
                (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZPATH, ZUNIQUEID)
            VALUES
                (1, 10.0, 11.0, 'Local Legacy', '/Users/example/Library/Application Support/com.apple.voicememos/Recordings/local.m4a', 'uuid-local');
            "#,
        )
        .unwrap();
        conn
    }
}
