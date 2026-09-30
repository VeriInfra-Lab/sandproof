//! Durable opt-in build inbox. Never executes or unpacks tenant input on the host.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Mutex, time::Duration};

pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub max_jobs_global: u32,
    pub max_jobs_per_tenant: u32,
    pub max_input_bytes_global: u64,
    pub max_input_bytes_per_tenant: u64,
    pub max_log_bytes: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    pub build_id: String,
    pub kind: BuildKind,
    pub disk_mib: u32,
    pub min_free_mib: u32,
    pub cpu_count: u8,
    pub memory_mib: u32,
    pub host_memory_mib: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildKind {
    Dockerfile,
    Oci,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    Queued,
    Building,
    Candidate,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuildView {
    pub build_id: String,
    pub state: BuildState,
    pub input_sha256: String,
    pub input_bytes: u64,
    pub candidate_id: Option<String>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildLog {
    /// Raw log bytes are encoded, not interpreted as terminal control sequences.
    pub bytes: Vec<u8>,
    pub next_offset: u64,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum BuildError {
    Invalid,
    Missing,
    Conflict,
    Quota,
    Storage,
}
impl From<rusqlite::Error> for BuildError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

pub struct BuildStore {
    db: Mutex<Connection>,
    config: BuildConfig,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl BuildRequest {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_id(&self.build_id)
            && (512..=32768).contains(&self.disk_mib)
            && (16..self.disk_mib).contains(&self.min_free_mib)
            && self.cpu_count > 0
            && (128..=32768).contains(&self.memory_mib)
            && self.host_memory_mib > self.memory_mib
            && self.host_memory_mib <= 65536
    }
}

impl BuildStore {
    /// Open a dedicated protected queue; no controller schema or lifecycle is changed.
    /// # Errors
    /// Rejects unsafe paths, incompatible schemas, invalid bounds and storage errors.
    pub fn open(config: BuildConfig, uid: u32) -> Result<Self, BuildError> {
        use std::os::unix::fs::MetadataExt;
        let root = &config.root;
        if !root.is_absolute()
            || std::fs::canonicalize(root).ok().as_ref() != Some(root)
            || config.max_jobs_global == 0
            || config.max_jobs_global > 1024
            || config.max_jobs_per_tenant == 0
            || config.max_jobs_per_tenant > config.max_jobs_global
            || config.max_input_bytes_per_tenant == 0
            || config.max_input_bytes_global < config.max_input_bytes_per_tenant
            || config.max_input_bytes_global > 16 * 1024 * 1024 * 1024
            || !(1..=1024 * 1024).contains(&config.max_log_bytes)
        {
            return Err(BuildError::Invalid);
        }
        let meta = std::fs::symlink_metadata(root).map_err(|_| BuildError::Invalid)?;
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(BuildError::Invalid);
        }
        let path = root.join("builds.db");
        for name in [
            "builds.db",
            "builds.db-wal",
            "builds.db-shm",
            "builds.db-journal",
        ] {
            match std::fs::symlink_metadata(root.join(name)) {
                Ok(m) if !m.is_file() || m.uid() != uid || m.nlink() != 1 => {
                    return Err(BuildError::Invalid);
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err(BuildError::Invalid),
            }
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let app: u32 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if version == 0 && app == 0 {
            let tables: u32 =
                tx.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if tables != 0 {
                return Err(BuildError::Invalid);
            }
            tx.execute_batch(
                "CREATE TABLE builds (
                owner TEXT NOT NULL, id TEXT NOT NULL, request TEXT NOT NULL,
                input BLOB NOT NULL, digest TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('queued','building','candidate','failed')),
                claim TEXT, resource TEXT, candidate TEXT, failure TEXT,
                log BLOB NOT NULL DEFAULT X'', truncated INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(owner,id));
                PRAGMA application_id=1397768770; PRAGMA user_version=1;",
            )?;
        } else if version != 1 || app != 1_397_768_770 {
            return Err(BuildError::Invalid);
        }
        tx.commit()?;
        Ok(Self {
            db: Mutex::new(db),
            config,
        })
    }

    /// Atomically retain the exact request and input; replays never dispatch again.
    /// # Errors
    /// Rejects malformed input, conflicting retries, exhausted quotas and unknown storage outcomes.
    pub fn submit(
        &self,
        owner: &str,
        request: &BuildRequest,
        input: &[u8],
    ) -> Result<BuildView, BuildError> {
        if owner.is_empty()
            || owner.len() > 128
            || !request.is_valid()
            || input.is_empty()
            || input.len() > MAX_INPUT_BYTES
        {
            return Err(BuildError::Invalid);
        }
        let digest = format!("{:x}", Sha256::digest(input));
        let encoded = serde_json::to_string(request).map_err(|_| BuildError::Invalid)?;
        let mut db = self.db.lock().map_err(|_| BuildError::Storage)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT request,digest FROM builds WHERE owner=?1 AND id=?2",
                params![owner, request.build_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old, hash)) = existing {
            if old != encoded || hash != digest {
                return Err(BuildError::Conflict);
            }
        } else {
            let (count, bytes, own_count, own_bytes): (u32,i64,u32,i64) = tx.query_row(
                "SELECT count(*),coalesce(sum(length(input)),0),coalesce(sum(owner=?1),0),coalesce(sum(CASE WHEN owner=?1 THEN length(input) ELSE 0 END),0) FROM builds",
                [owner], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            if count >= self.config.max_jobs_global
                || own_count >= self.config.max_jobs_per_tenant
                || u64::try_from(bytes).map_err(|_| BuildError::Storage)? + input.len() as u64
                    > self.config.max_input_bytes_global
                || u64::try_from(own_bytes).map_err(|_| BuildError::Storage)? + input.len() as u64
                    > self.config.max_input_bytes_per_tenant
            {
                return Err(BuildError::Quota);
            }
            tx.execute("INSERT INTO builds(owner,id,request,input,digest,state) VALUES(?1,?2,?3,?4,?5,'queued')",
                params![owner,request.build_id,encoded,input,digest])?;
        }
        let result = read_view(&tx, owner, &request.build_id)?;
        tx.commit()?;
        Ok(result)
    }

    /// # Errors
    /// Unknown and foreign requests both return Missing.
    pub fn get(&self, owner: &str, id: &str) -> Result<BuildView, BuildError> {
        let db = self.db.lock().map_err(|_| BuildError::Storage)?;
        read_view(&db, owner, id)
    }

    /// # Errors
    /// Refuses invalid cursors and never reads another owner's logs.
    pub fn logs(
        &self,
        owner: &str,
        id: &str,
        offset: u64,
        limit: u32,
    ) -> Result<BuildLog, BuildError> {
        if !(1..=65536).contains(&limit) || offset > u64::from(self.config.max_log_bytes) {
            return Err(BuildError::Invalid);
        }
        let db = self.db.lock().map_err(|_| BuildError::Storage)?;
        let (length, bytes, truncated): (i64,Vec<u8>,bool) = db.query_row(
            "SELECT length(log),substr(log,?3+1,?4),truncated FROM builds WHERE owner=?1 AND id=?2",
            params![owner,id,i64::try_from(offset).map_err(|_| BuildError::Invalid)?,limit], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(BuildError::Missing)?;
        if offset > u64::try_from(length).map_err(|_| BuildError::Storage)? {
            return Err(BuildError::Invalid);
        }
        Ok(BuildLog {
            next_offset: offset + bytes.len() as u64,
            bytes,
            truncated,
        })
    }
}

fn read_view(db: &Connection, owner: &str, id: &str) -> Result<BuildView, BuildError> {
    let (state, digest, size, candidate, failure): (String,String,i64,Option<String>,Option<String>) = db.query_row(
        "SELECT state,digest,length(input),candidate,failure FROM builds WHERE owner=?1 AND id=?2", params![owner,id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or(BuildError::Missing)?;
    let state = match state.as_str() {
        "queued" => BuildState::Queued,
        "building" => BuildState::Building,
        "candidate" => BuildState::Candidate,
        "failed" => BuildState::Failed,
        _ => return Err(BuildError::Storage),
    };
    Ok(BuildView {
        build_id: id.into(),
        state,
        input_sha256: digest,
        input_bytes: u64::try_from(size).map_err(|_| BuildError::Storage)?,
        candidate_id: candidate,
        failure,
    })
}
