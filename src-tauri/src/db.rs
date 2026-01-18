use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
  #[error("database error: {0}")]
  Sql(#[from] rusqlite::Error),
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),
  #[error("database lock poisoned")]
  Lock,
}

pub struct Db {
  conn: Mutex<Connection>,
}

impl Db {
  pub fn new(db_path: PathBuf) -> Result<Self, DbError> {
    if let Some(parent) = db_path.parent() {
      std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(db_path)?;
    conn.execute_batch(include_str!("db/schema.sql"))?;
    let _ = conn.execute("ALTER TABLE live_settings ADD COLUMN record_path TEXT", []);
    let _ = conn.execute("ALTER TABLE submission_task ADD COLUMN aid INTEGER", []);
    let _ = conn.execute("ALTER TABLE submission_task ADD COLUMN remote_state INTEGER", []);
    let _ = conn.execute("ALTER TABLE submission_task ADD COLUMN reject_reason TEXT", []);
    let _ = conn.execute("ALTER TABLE video_download ADD COLUMN cid INTEGER", []);
    let _ = conn.execute("ALTER TABLE video_download ADD COLUMN content TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_progress REAL DEFAULT 0.0", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_uploaded_bytes INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_total_bytes INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_cid INTEGER", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_file_name TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_biz_id INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_endpoint TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_auth TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_uri TEXT", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_chunk_size INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE merged_video ADD COLUMN upload_last_part_index INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_progress REAL DEFAULT 0.0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_uploaded_bytes INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_total_bytes INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_biz_id INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_endpoint TEXT", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_auth TEXT", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_uri TEXT", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_chunk_size INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE task_output_segment ADD COLUMN upload_last_part_index INTEGER DEFAULT 0", []);

    Ok(Self {
      conn: Mutex::new(conn),
    })
  }

  pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, rusqlite::Error>) -> Result<T, DbError> {
    let conn = self.conn.lock().map_err(|_| DbError::Lock)?;
    Ok(f(&conn)?)
  }

  pub fn with_conn_mut<T>(
    &self,
    f: impl FnOnce(&mut Connection) -> Result<T, rusqlite::Error>,
  ) -> Result<T, DbError> {
    let mut conn = self.conn.lock().map_err(|_| DbError::Lock)?;
    Ok(f(&mut conn)?)
  }
}
