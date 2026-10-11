use crate::{ControlState, REDIS_VERSION, Revision, StoreError, WriterLease, codec, scripts};
use redis::{Connection, ConnectionAddr, IntoConnectionInfo, RedisConnectionInfo};
use sandproof_protocol::DeploymentId;
use std::{
    collections::BTreeMap,
    net::SocketAddrV4,
    time::{Duration, Instant},
};

const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;

/// Supplied only by protected center assembly. Remote Redis is not supported by
/// this initial adapter: only a literal IPv4 loopback address plus ACL, without
/// URL/DNS, sentinel or failover. The co-located host is a trusted boundary.
pub struct RedisEndpoint {
    address: SocketAddrV4,
    username: String,
    password: String,
}

impl RedisEndpoint {
    /// # Errors
    /// Rejects non-loopback/zero-port endpoints or missing/oversized credentials.
    pub fn new(
        address: SocketAddrV4,
        username: String,
        password: String,
    ) -> Result<Self, StoreError> {
        if !address.ip().is_loopback()
            || address.port() == 0
            || username.is_empty()
            || username.len() > 128
            || password.is_empty()
            || password.len() > 1024
        {
            return Err(StoreError::Configuration);
        }
        Ok(Self {
            address,
            username,
            password,
        })
    }
}

#[derive(Clone)]
pub struct StoreConfig {
    deployment: DeploymentId,
    max_block_bytes: usize,
    redis_memory_bytes: u64,
    operation_timeout: Duration,
}

impl StoreConfig {
    /// `redis_memory_bytes` must exactly match the dedicated engine's configured
    /// ceiling, not a shared cache. AOF/backup filesystem quotas are a separate
    /// deployment obligation; this component does not infer free disk space.
    /// # Errors
    /// Rejects unusable byte/time budgets or insufficient engine envelope.
    pub fn new(
        deployment: DeploymentId,
        max_block_bytes: usize,
        redis_memory_bytes: u64,
        operation_timeout: Duration,
    ) -> Result<Self, StoreError> {
        let block = u64::try_from(max_block_bytes).map_err(|_| StoreError::Configuration)?;
        if !(2..=MAX_BLOCK_BYTES).contains(&max_block_bytes)
            || redis_memory_bytes < block * 4 + 16 * 1024 * 1024
            || operation_timeout.is_zero()
            || operation_timeout > Duration::from_mins(1)
        {
            return Err(StoreError::Configuration);
        }
        Ok(Self {
            deployment,
            max_block_bytes,
            redis_memory_bytes,
            operation_timeout,
        })
    }

    fn key(&self) -> String {
        format!("sandproof:authority:{}", self.deployment.as_str())
    }
}

/// An observed snapshot cannot be constructed by callers and is not a durable
/// dispatch certificate. Its original bytes are retained for exact CAS checks.
pub struct Observed<T> {
    deployment: DeploymentId,
    revision: Revision,
    state: T,
    encoded: Vec<u8>,
}

impl<T> Observed<T> {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }
    #[must_use]
    pub const fn state(&self) -> &T {
        &self.state
    }
}

/// A successful bounded write plus durability-health acknowledgement. Dispatch
/// still needs the center's domain/identity/intent checks and original bindings.
pub struct Committed<T>(Observed<T>);

impl<T> Committed<T> {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.0.revision
    }
    #[must_use]
    pub const fn state(&self) -> &T {
        &self.0.state
    }
    #[must_use]
    pub fn into_observed(self) -> Observed<T> {
        self.0
    }
}

/// Exactly one synchronous connection, one writer lease and no background
/// reconnection/retry/pool. `&mut self` serializes this adapter only, not remote
/// VM work. The future center decision lock must not span host RPC or streams.
pub struct RedisLedger<T> {
    endpoint: RedisEndpoint,
    config: StoreConfig,
    _lease: WriterLease,
    connection: Option<Connection>,
    write_fenced: bool,
    _state: std::marker::PhantomData<T>,
}

impl<T: ControlState> RedisLedger<T> {
    /// Explicit provisioning only; never called by ordinary startup/recovery.
    /// Does not overwrite ANY existing key, even a wrong-type/corrupt one.
    /// # Errors
    /// Returns validation/refusal, or `OutcomeUnknown` for an uncertain write.
    pub fn initialize(
        endpoint: RedisEndpoint,
        config: StoreConfig,
        lease: WriterLease,
        state: &T,
    ) -> Result<(Self, Committed<T>), StoreError> {
        let mut ledger = Self::connect(endpoint, config, lease)?;
        let deadline = ledger.deadline();
        let encoded = codec::encode(state, ledger.config.max_block_bytes)?;
        ledger.check_health(deadline)?;
        let revision = Revision::try_from(1).map_err(|_| StoreError::Configuration)?;
        let committed = ledger.write(None, encoded, revision, deadline)?;
        Ok((ledger, committed))
    }

    /// Normal startup must find and validate the original authority block.
    /// A read is not dispatch approval; reaffirm before resuming original work.
    /// # Errors
    /// Rejects missing, corrupt, unsafe, foreign or unsupported deployments.
    pub fn open(
        endpoint: RedisEndpoint,
        config: StoreConfig,
        lease: WriterLease,
    ) -> Result<(Self, Observed<T>), StoreError> {
        let mut ledger = Self::connect(endpoint, config, lease)?;
        let observed = ledger.read()?;
        Ok((ledger, observed))
    }

    fn connect(
        endpoint: RedisEndpoint,
        config: StoreConfig,
        lease: WriterLease,
    ) -> Result<Self, StoreError> {
        if T::FORMAT_VERSION == 0 || lease.deployment != config.deployment {
            return Err(StoreError::Configuration);
        }
        let mut ledger = Self {
            endpoint,
            config,
            _lease: lease,
            connection: None,
            write_fenced: false,
            _state: std::marker::PhantomData,
        };
        ledger.reconnect()?;
        Ok(ledger)
    }

    /// Explicit reconnect; does not clear an uncertain-write fence or dispatch.
    /// # Errors
    /// Rejects unavailable or unhealthy storage without automatic retry.
    pub fn reconnect(&mut self) -> Result<(), StoreError> {
        self.connection = None;
        let deadline = self.deadline();
        // redis-rs applies connect_timeout to TCP, but NOT to UnixStream::connect.
        // Literal loopback therefore avoids an unbounded local socket connect.
        let info = ConnectionAddr::Tcp(
            self.endpoint.address.ip().to_string(),
            self.endpoint.address.port(),
        )
        .into_connection_info()
        .map_err(|_| StoreError::Configuration)?
        .set_redis_settings(
            RedisConnectionInfo::default()
                .set_username(&self.endpoint.username)
                .set_password(&self.endpoint.password)
                .set_skip_set_lib_name(),
        );
        let client = redis::Client::open(info).map_err(|_| StoreError::Configuration)?;
        self.connection = Some(
            client
                .get_connection_with_timeout(self.config.operation_timeout)
                .map_err(|_| StoreError::Unavailable)?,
        );
        if let Err(error) = self.check_health(deadline) {
            self.connection = None;
            return Err(error);
        }
        Ok(())
    }

    #[must_use]
    pub const fn writes_fenced(&self) -> bool {
        self.write_fenced
    }

    /// # Errors
    /// Rejects an unhealthy/missing/oversized/foreign/unvalidated control block.
    pub fn read(&mut self) -> Result<Observed<T>, StoreError> {
        let deadline = self.deadline();
        self.check_health(deadline)?;
        let response: Vec<redis::Value> = self.query(
            redis::cmd("EVAL")
                .arg(scripts::READ)
                .arg(1)
                .arg(self.config.key())
                .arg(self.config.deployment.as_str())
                .arg(T::FORMAT_VERSION)
                .arg(self.config.max_block_bytes),
            deadline,
        )?;
        let length = response.len();
        let mut response = response.into_iter();
        let Some(first) = response.next() else {
            return Err(StoreError::InvalidRecord);
        };
        let code: i64 = redis::from_redis_value(first).map_err(|_| StoreError::InvalidRecord)?;
        match code {
            1 if length == 1 => return Err(StoreError::Missing),
            3 if length == 1 => return Err(StoreError::InvalidRecord),
            0 if length == 3 => {}
            _ => return Err(StoreError::InvalidRecord),
        }
        let raw: String =
            redis::from_redis_value(response.next().ok_or(StoreError::InvalidRecord)?)
                .map_err(|_| StoreError::InvalidRecord)?;
        let number: u64 = raw.parse().map_err(|_| StoreError::InvalidRecord)?;
        if number.to_string() != raw {
            return Err(StoreError::InvalidRecord);
        }
        let revision = Revision::try_from(number).map_err(|_| StoreError::InvalidRecord)?;
        let encoded: Vec<u8> =
            redis::from_redis_value(response.next().ok_or(StoreError::InvalidRecord)?)
                .map_err(|_| StoreError::InvalidRecord)?;
        let state = codec::decode(&encoded, self.config.max_block_bytes)?;
        Ok(Observed {
            deployment: self.config.deployment.clone(),
            revision,
            state,
            encoded,
        })
    }

    /// # Errors
    /// Refuses while write-fenced; CAS conflicts do not mutate the record.
    /// Any write transport/error/late health failure is `OutcomeUnknown`.
    pub fn commit(&mut self, expected: &Observed<T>, next: &T) -> Result<Committed<T>, StoreError> {
        if self.write_fenced {
            return Err(StoreError::WriteFenced);
        }
        let deadline = self.deadline();
        self.replace(
            expected,
            codec::encode(next, self.config.max_block_bytes)?,
            deadline,
        )
    }

    /// Recovery must validate the current record and durably recommit its EXACT
    /// original payload with a fresh revision before dispatch. Reads/reconnects
    /// never clear the fence. This is not allocating a new sandbox/operation.
    /// # Errors
    /// CAS/health/capacity/identity/exhaustion or an unknown recommit outcome.
    pub fn reaffirm(&mut self, expected: &Observed<T>) -> Result<Committed<T>, StoreError> {
        self.replace(expected, expected.encoded.clone(), self.deadline())
    }

    fn replace(
        &mut self,
        expected: &Observed<T>,
        encoded: Vec<u8>,
        deadline: Instant,
    ) -> Result<Committed<T>, StoreError> {
        if expected.deployment != self.config.deployment {
            return Err(StoreError::InvalidRecord);
        }
        let next = expected
            .revision
            .checked_next()
            .map_err(|_| StoreError::RevisionExhausted)?;
        self.check_health(deadline)?;
        self.write(Some(expected), encoded, next, deadline)
    }

    fn write(
        &mut self,
        expected: Option<&Observed<T>>,
        encoded: Vec<u8>,
        revision: Revision,
        deadline: Instant,
    ) -> Result<Committed<T>, StoreError> {
        let state = codec::decode(&encoded, self.config.max_block_bytes)?;
        let prior = expected.map_or_else(String::new, |value| value.revision.get().to_string());
        let mut command = redis::cmd("EVAL");
        command
            .arg(scripts::CAS)
            .arg(1)
            .arg(self.config.key())
            .arg(self.config.deployment.as_str())
            .arg(T::FORMAT_VERSION)
            .arg(prior)
            .arg(revision.get())
            .arg(&encoded)
            .arg(self.config.max_block_bytes)
            .arg(expected.map_or(&[][..], |value| value.encoded.as_slice()));
        let result: i64 = match self.query(&mut command, deadline) {
            Ok(value) => value,
            Err(_) => return self.fence_unknown(),
        };
        match result {
            0 => {}
            1 => return Err(StoreError::Missing),
            2 => return Err(StoreError::AlreadyInitialized),
            3 => return Err(StoreError::InvalidRecord),
            4 => return Err(StoreError::Conflict),
            _ => return self.fence_unknown(),
        }
        if self.check_health(deadline).is_err() {
            return self.fence_unknown();
        }
        self.write_fenced = false;
        Ok(Committed(Observed {
            deployment: self.config.deployment.clone(),
            revision,
            state,
            encoded,
        }))
    }

    fn fence_unknown<R>(&mut self) -> Result<R, StoreError> {
        self.write_fenced = true;
        self.connection = None;
        Err(StoreError::OutcomeUnknown)
    }

    fn deadline(&self) -> Instant {
        Instant::now() + self.config.operation_timeout
    }

    fn query<R: redis::FromRedisValue>(
        &mut self,
        command: &mut redis::Cmd,
        deadline: Instant,
    ) -> Result<R, StoreError> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or(StoreError::Unavailable)?;
        let result = (|| {
            let connection = self.connection.as_mut().ok_or(StoreError::Unavailable)?;
            connection
                .set_read_timeout(Some(remaining))
                .map_err(|_| StoreError::Unavailable)?;
            connection
                .set_write_timeout(Some(remaining))
                .map_err(|_| StoreError::Unavailable)?;
            command
                .query(connection)
                .map_err(|_| StoreError::Unavailable)
        })();
        // Never reuse a timed-out protocol stream with a possibly pending reply.
        if result.is_err() {
            self.connection = None;
        }
        result
    }

    fn check_health(&mut self, deadline: Instant) -> Result<(), StoreError> {
        let info: String = self.query(redis::cmd("INFO").arg("server"), deadline)?;
        if info_field(&info, "redis_version") != Some(REDIS_VERSION) {
            return Err(StoreError::Configuration);
        }
        let settings: BTreeMap<String, String> = self.query(
            redis::cmd("CONFIG")
                .arg("GET")
                .arg("appendonly")
                .arg("appendfsync")
                .arg("maxmemory-policy")
                .arg("no-appendfsync-on-rewrite")
                .arg("aof-load-truncated")
                .arg("maxmemory"),
            deadline,
        )?;
        let memory = self.config.redis_memory_bytes.to_string();
        for (key, required) in [
            ("appendonly", "yes"),
            ("appendfsync", "always"),
            ("maxmemory-policy", "noeviction"),
            ("no-appendfsync-on-rewrite", "no"),
            ("aof-load-truncated", "no"),
            ("maxmemory", memory.as_str()),
        ] {
            if settings.get(key).map(String::as_str) != Some(required) {
                return Err(StoreError::Configuration);
            }
        }
        let persistence: String = self.query(redis::cmd("INFO").arg("persistence"), deadline)?;
        for (key, required) in [
            ("loading", "0"),
            ("aof_enabled", "1"),
            ("aof_last_write_status", "ok"),
            ("aof_last_bgrewrite_status", "ok"),
        ] {
            if info_field(&persistence, key) != Some(required) {
                return Err(StoreError::Unavailable);
            }
        }
        Ok(())
    }
}

fn info_field<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    let mut matching = info
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| *name == key)
        .map(|(_, value)| value.trim_end_matches('\r'));
    let value = matching.next()?;
    if matching.next().is_some() {
        None
    } else {
        Some(value)
    }
}
