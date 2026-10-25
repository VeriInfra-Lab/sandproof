//! Debug-only, original-candidate observation before the activation receipt.
//! No file, selector or command here grants lifecycle or guest authority.
use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
use sandbox_controller_core as core;
use sandbox_runtime::RuntimeProcessIdentity;
use sandproof_protocol::WorkAction;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CONTROL_LIMIT: u64 = 1024;
const OBSERVATION_LIMIT: usize = 16_384;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    run_identity: String,
    sandbox: String,
    generation: u64,
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instruction {
    schema_version: u32,
    run_identity: String,
    observation_sha256: String,
    command: String,
}
#[derive(Serialize)]
struct Ready<'a> {
    schema_version: u32,
    observation_sha256: &'a str,
}
#[derive(Serialize)]
struct Candidate<'a> {
    resource_id: &'a str,
    pid: u32,
    start_time_ticks: u64,
    boot_id: &'a str,
}
#[derive(Serialize)]
struct Namespace<'a> {
    name: &'a str,
    device: u64,
    inode: u64,
}
#[derive(Serialize)]
struct Observation<'a> {
    schema_version: u32,
    run_identity: &'a str,
    action: &'a WorkAction<core::EffectId>,
    candidate: Candidate<'a>,
    namespace: Namespace<'a>,
    token: &'a str,
}
struct Directory {
    path: PathBuf,
    file: File,
    owner: u32,
}
pub(crate) struct RestoreActivationGate {
    directory: Directory,
    config: Config,
    config_bytes: Vec<u8>,
    config_file: File,
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut value, "{byte:02x}").expect("String write");
    }
    value
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identity_matches(directory: &File, name: &str, file: &File) -> Result<()> {
    let named = fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
    let opened = file.metadata()?;
    if (named.st_dev, named.st_ino) != (opened.dev(), opened.ino()) {
        return Err("restore gate original file changed".into());
    }
    Ok(())
}
impl Directory {
    fn from_opened(path: PathBuf, file: File, owner: u32) -> Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o7777 != 0o700 {
            return Err("restore gate requires its original private directory".into());
        }
        let value = Self { path, file, owner };
        value.verify()?;
        Ok(value)
    }
    fn open(path: PathBuf) -> Result<Self> {
        sandbox_runtime::check_trusted_path(&path)?;
        let file = File::from(fs::open(
            &path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        Self::from_opened(path, file, 0)
    }
    fn verify(&self) -> Result<()> {
        let current = std::fs::symlink_metadata(&self.path)?;
        let original = self.file.metadata()?;
        if current.file_type().is_symlink()
            || (current.dev(), current.ino()) != (original.dev(), original.ino())
            || current.uid() != self.owner
            || current.mode() & 0o7777 != 0o700
        {
            return Err("restore gate original directory changed".into());
        }
        Ok(())
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.read_file(name)?.map(|(_, bytes)| bytes))
    }
    fn read_file(&self, name: &str) -> Result<Option<(File, Vec<u8>)>> {
        self.verify()?;
        let file = match fs::openat(
            &self.file,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let before = file.metadata()?;
        if !before.is_file()
            || before.uid() != self.owner
            || before.nlink() != 1
            || before.mode() & 0o7777 != 0o600
            || before.len() > CONTROL_LIMIT
        {
            return Err("restore gate control file type, owner, mode or bound".into());
        }
        let mut bytes = Vec::new();
        (&file).take(CONTROL_LIMIT + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        if bytes.len() as u64 != before.len()
            || (before.len(), before.ctime(), before.ctime_nsec())
                != (after.len(), after.ctime(), after.ctime_nsec())
        {
            return Err("restore gate control file changed while reading".into());
        }
        identity_matches(&self.file, name, &file)?;
        Ok(Some((file, bytes)))
    }
    fn create(&self, name: &str, bytes: &[u8]) -> Result<File> {
        self.verify()?;
        let mut file = File::from(fs::openat(
            &self.file,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        file.write_all(bytes)?;
        file.sync_all()?;
        identity_matches(&self.file, name, &file)?;
        self.file.sync_all()?;
        self.verify()?;
        Ok(file)
    }
    fn publish_ready(&self, digest: &str) -> Result<()> {
        let bytes = serde_json::to_vec(&Ready {
            schema_version: 1,
            observation_sha256: digest,
        })?;
        let file = self.create("ready-stage.json", &bytes)?;
        fs::renameat_with(
            &self.file,
            "ready-stage.json",
            &self.file,
            "ready.json",
            RenameFlags::NOREPLACE,
        )?;
        self.file.sync_all()?;
        identity_matches(&self.file, "ready.json", &file)?;
        Ok(())
    }
}
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}
impl RestoreActivationGate {
    pub(crate) fn configured(action: &WorkAction<core::EffectId>) -> Result<Option<Self>> {
        let Some(path) = std::env::var_os("SANDPROOF_TEST_RESTORE_ACTIVATION_GATE") else {
            return Ok(None);
        };
        Self::select(Directory::open(PathBuf::from(path))?, action)
    }
    fn select(directory: Directory, action: &WorkAction<core::EffectId>) -> Result<Option<Self>> {
        let (config_file, config_bytes) = directory
            .read_file("config.json")?
            .ok_or("restore gate config missing")?;
        let config: Config = decode(&config_bytes)?;
        if config.schema_version != 1
            || !valid_digest(&config.run_identity)
            || !(1000..=120_000).contains(&config.timeout_ms)
            || config.sandbox.is_empty()
            || config.sandbox.len() > 64
            || config.generation == 0
        {
            return Err("restore gate config shape or deadline".into());
        }
        if config.sandbox != action.identity.sandbox.as_str()
            || config.generation != action.identity.generation
        {
            return Ok(None);
        }
        if !matches!(action.command.command(), core::Command::ActivateVm { .. }) {
            return Err("restore gate requires original activation action".into());
        }
        Ok(Some(Self {
            directory,
            config,
            config_bytes,
            config_file,
        }))
    }
    pub(crate) fn wait(
        &self,
        action: &WorkAction<core::EffectId>,
        name: &str,
        process: &RuntimeProcessIdentity,
        namespace: &File,
        token: &str,
    ) -> Result<()> {
        process.validate_fields()?;
        if action.identity.resource.id.as_str() != name || token.len() < 32 || token.len() > 128 {
            return Err("restore gate observation differs from actual activation".into());
        }
        let metadata = namespace.metadata()?;
        let observation = Observation {
            schema_version: 1,
            run_identity: &self.config.run_identity,
            action,
            candidate: Candidate {
                resource_id: name,
                pid: process.pid,
                start_time_ticks: process.start_time_ticks,
                boot_id: &process.boot_id,
            },
            namespace: Namespace {
                name,
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            token,
        };
        let bytes = serde_json::to_vec(&observation)?;
        if bytes.len() > OBSERVATION_LIMIT {
            return Err("restore gate observation exceeds original bound".into());
        }
        let record = self.directory.create("observation.json", &bytes)?;
        let original = record.metadata()?;
        let digest = hex_digest(&bytes);
        self.directory.publish_ready(&digest)?;
        self.wait_until(
            &digest,
            &record,
            &original,
            Instant::now() + Duration::from_millis(self.config.timeout_ms),
        )
    }
    fn wait_until(
        &self,
        digest: &str,
        record: &File,
        original: &std::fs::Metadata,
        deadline: Instant,
    ) -> Result<()> {
        loop {
            self.directory.verify()?;
            identity_matches(&self.directory.file, "observation.json", record)?;
            identity_matches(&self.directory.file, "config.json", &self.config_file)?;
            let current = record.metadata()?;
            if (
                current.len(),
                current.ctime(),
                current.ctime_nsec(),
                current.mode(),
                current.uid(),
                current.nlink(),
            ) != (
                original.len(),
                original.ctime(),
                original.ctime_nsec(),
                original.mode(),
                original.uid(),
                original.nlink(),
            ) {
                return Err("restore gate original observation changed".into());
            }
            if self.directory.read("config.json")?.as_deref() != Some(self.config_bytes.as_slice())
            {
                return Err("restore gate original config changed".into());
            }
            if Instant::now() >= deadline {
                return Err("restore gate deadline exceeded".into());
            }
            if let Some(bytes) = self.directory.read("command.json")? {
                let command: Instruction = decode(&bytes)?;
                if command.schema_version != 1
                    || command.run_identity != self.config.run_identity
                    || command.observation_sha256 != digest
                {
                    return Err("restore gate command differs from original observation".into());
                }
                self.directory.file.sync_all()?;
                return match command.command.as_str() {
                    "continue" => Ok(()),
                    "fail" => Err("restore gate explicitly interrupted".into()),
                    _ => Err("unsupported restore gate command".into()),
                };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests;
