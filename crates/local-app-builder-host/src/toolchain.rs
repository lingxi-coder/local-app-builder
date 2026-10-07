//! The toolchain a build runs: exactly the Node and pnpm the templates were made for.
//!
//! An app's contract names its toolchain (`pnpm@12.5.1/node@26.9.0`), and a build is only the build the contract
//! describes if it ran those tools. Whatever happens to be on a Mac's `PATH` is not that (P1 measured a corepack shim
//! that fails outright, and a different Node that builds but proves nothing), so this module provides its own copy
//! under `<data root>/toolchains/<key>/`, and nothing else is ever run.
//!
//! **What is pinned.** For each platform: the URL, the SHA-256 and the exact size of the Node archive and of pnpm's
//! native-binary package. The pins are in this file, so the network is never trusted for what to expect: a download
//! that is larger than pinned is cut off, and one that does not hash to the pin is thrown away before anything in it is
//! read. Only the two programs are taken out of the archives (`bin/node` and pnpm's binary): no npm, no headers, no
//! scripts, and nothing from an archive is ever written outside its own staging directory.
//!
//! **pnpm is the native package, on purpose.** pnpm 12's main package is a launcher that downloads its binary the first
//! time it runs, from wherever its environment says. Pinning that package would pin nothing: the binary that actually
//! builds would come from the network at build time, inside the sandbox. `@pnpm/exe.<platform>` carries the binary
//! itself.
//!
//! **What installed means.** The receipt records the archive and binary digests and the versions the programs
//! reported. [`Toolchains::verify`] recomputes the digests and runs `--version` on each, so a tree that was altered, or
//! that reports another version than it was installed as, is reported damaged and replaced by the next install. The
//! install runs under a lock, extracts into a staging directory and moves the finished tree into place, so another
//! process never sees half a toolchain.
//!
//! **Offline.** [`Source::Directory`] takes the same archives from a directory instead of the network, and checks them
//! against the same pins.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;

/// The toolchain key the templates' contracts name. A test pins it to the service's own constant.
pub const TOOLCHAIN_KEY: &str = "pnpm@12.5.1/node@26.9.0";
/// What `node --version` prints for the pinned Node.
pub const NODE_VERSION_OUTPUT: &str = "v26.9.0";
/// What `pnpm --version` prints for the pinned pnpm.
pub const PNPM_VERSION_OUTPUT: &str = "12.5.1";

/// The receipt file inside an installed toolchain.
pub const RECEIPT_FILE: &str = "receipt.json";
/// The lock that serialises installs into one `toolchains/` directory.
pub const INSTALL_LOCK_FILE: &str = ".install.lock";
/// How long an install waits for another install of the same directory.
pub const INSTALL_LOCK_TIMEOUT: Duration = Duration::from_secs(600);
/// The most a single extracted program may expand to; a larger one is an archive bomb, not Node.
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
/// How long one `--version` may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// The Macs this can provision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Apple silicon.
    DarwinArm64,
    /// Intel.
    DarwinX64,
}

impl Platform {
    /// The platform this process runs on.
    ///
    /// # Errors
    /// It is not a Mac this has pins for.
    pub fn host() -> Result<Self, String> {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Ok(Self::DarwinArm64)
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Ok(Self::DarwinX64)
        } else {
            Err(format!(
                "no pinned toolchain for {}-{}: only macOS (arm64, x64) is provisioned",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin-arm64",
            Self::DarwinX64 => "darwin-x64",
        }
    }
}

/// One archive: where it comes from and what it must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// Where to download it.
    pub url: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    /// Its exact size in bytes.
    pub size: u64,
    /// The one file to take out of it, as the archive names it.
    pub entry: String,
}

impl Artifact {
    /// The archive's file name, which is how [`Source::Directory`] finds it.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.url.rsplit('/').next().unwrap_or(&self.url)
    }
}

/// Everything that identifies one toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The key (`pnpm@…/node@…`); also the directory it installs into.
    pub key: String,
    /// What `node --version` must print.
    pub node_version: String,
    /// What `pnpm --version` must print.
    pub pnpm_version: String,
    /// The Node archive.
    pub node: Artifact,
    /// pnpm's native-binary package.
    pub pnpm: Artifact,
}

impl Spec {
    /// The pinned toolchain for `platform`.
    #[must_use]
    pub fn pinned(platform: Platform) -> Self {
        let node = |sha256: &str, size| Artifact {
            url: format!("https://nodejs.org/dist/v26.9.0/node-v26.9.0-{}.tar.gz", platform.name()),
            sha256: sha256.into(),
            size,
            entry: format!("node-v26.9.0-{}/bin/node", platform.name()),
        };
        let pnpm = |sha256: &str, size| Artifact {
            url: format!(
                "https://registry.npmjs.org/@pnpm/exe.{0}/-/exe.{0}-12.5.1.tgz",
                platform.name()
            ),
            sha256: sha256.into(),
            size,
            entry: "package/pnpm".into(),
        };
        let (node, pnpm) = match platform {
            // Node's digests are from nodejs.org's SHASUMS256.txt for v26.9.0. The arm64 archive was also downloaded
            // and hashed here (P1); the x64 one is pinned from SHASUMS256.txt alone. pnpm's packages were downloaded,
            // hashed here, and their SHA-512 compared with the registry's `dist.integrity`.
            Platform::DarwinArm64 => (
                node("6f3de7ed853ee283b4bf24b6e426618f1d357401ce5815db1866eb85eb4b05d9", 58_074_543),
                pnpm("8e187dd097b1f16de500ebd4b9b32391e6b885f7082a110d5f337b0d42458d41", 19_795_171),
            ),
            Platform::DarwinX64 => (
                node("06b2e742ed9025dc84adc830243b3f731956eac9c321bccd0ede384209af02a8", 59_474_959),
                pnpm("3af6f17fa65c0824e3331aca351fdd673bedc24516cf9a6b3a656693261197bf", 21_865_288),
            ),
        };
        Self {
            key: TOOLCHAIN_KEY.into(),
            node_version: NODE_VERSION_OUTPUT.into(),
            pnpm_version: PNPM_VERSION_OUTPUT.into(),
            node,
            pnpm,
        }
    }
}

/// Where the archives come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Download them.
    Network,
    /// Take them, under their own file names, from this directory.
    Directory(PathBuf),
}

/// What an install recorded about one program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramRecord {
    /// What `--version` printed when it was installed.
    pub version: String,
    /// The digest of the archive it came from.
    pub archive_sha256: String,
    /// The digest of the program itself.
    pub binary_sha256: String,
}

/// What an install recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The toolchain key.
    pub key: String,
    /// Node.
    pub node: ProgramRecord,
    /// pnpm.
    pub pnpm: ProgramRecord,
    /// When it was installed, in seconds since the Unix epoch.
    pub installed_unix: u64,
}

/// Why a toolchain could not be installed or is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainError {
    /// The download could not be made at all.
    Unreachable {
        /// What was being fetched.
        url: String,
        /// Why it could not be reached.
        detail: String,
    },
    /// The server answered, but not with the file.
    BadResponse {
        /// What was being fetched.
        url: String,
        /// What it answered.
        detail: String,
    },
    /// A file is not the one that was pinned.
    Mismatch {
        /// Which file.
        what: String,
        /// How it differs.
        detail: String,
    },
    /// The archive is not what it should contain.
    BadArchive {
        /// Which archive.
        file: String,
        /// What is wrong with it.
        detail: String,
    },
    /// An installed program does not behave as pinned.
    WrongProgram {
        /// `node` or `pnpm`.
        program: String,
        /// What it did instead.
        detail: String,
    },
    /// Anything about the local file system.
    Io(String),
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable { url, detail } => write!(
                f,
                "toolchain_unreachable: cannot download {url}: {detail}. Nothing was installed. To install without \
                 network access, put the two archives in a directory and run `local-app-builder toolchain install --from DIR`; \
                 `local-app-builder toolchain status` names the files."
            ),
            Self::BadResponse { url, detail } => write!(f, "toolchain_download_failed: {url}: {detail}. Nothing was installed."),
            Self::Mismatch { what, detail } => write!(
                f,
                "toolchain_integrity: {what} is not the file that was pinned ({detail}). It was discarded and nothing was installed."
            ),
            Self::BadArchive { file, detail } => write!(f, "toolchain_archive: {file}: {detail}. Nothing was installed."),
            Self::WrongProgram { program, detail } => write!(f, "toolchain_program: {program}: {detail}"),
            Self::Io(detail) => write!(f, "toolchain_io: {detail}"),
        }
    }
}

impl std::error::Error for ToolchainError {}

fn io<E: fmt::Display>(context: impl fmt::Display) -> impl FnOnce(E) -> ToolchainError {
    move |error| ToolchainError::Io(format!("{context}: {error}"))
}

/// The state of a toolchain directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing is installed for the key.
    NotInstalled,
    /// Installed and verified.
    Ready(Receipt),
    /// Something is there and it is not the pinned toolchain.
    Damaged(String),
}

/// An install: the toolchains of one data root.
#[derive(Debug, Clone)]
pub struct Toolchains {
    root: PathBuf,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..n]);
    }
}

impl Toolchains {
    /// The toolchains under `<data_root>/toolchains`.
    #[must_use]
    pub fn in_data_root(data_root: &Path) -> Self {
        Self { root: data_root.join("toolchains") }
    }

    /// The directory a toolchain installs into; it holds `bin/node` and `bin/pnpm`.
    #[must_use]
    pub fn dir(&self, spec: &Spec) -> PathBuf {
        self.root.join(&spec.key)
    }

    /// Whether the toolchain for `spec` is installed and intact.
    pub async fn status(&self, spec: &Spec) -> Status {
        let dir = self.dir(spec);
        if !dir.exists() {
            return Status::NotInstalled;
        }
        match self.verify(spec).await {
            Ok(receipt) => Status::Ready(receipt),
            Err(error) => Status::Damaged(error.to_string()),
        }
    }

    /// Check the installed toolchain against the pins: the receipt names this key and these archives, both programs
    /// hash to what the receipt says, and each reports the pinned version.
    ///
    /// # Errors
    /// The toolchain is missing, was altered, or does not behave as pinned.
    pub async fn verify(&self, spec: &Spec) -> Result<Receipt, ToolchainError> {
        verify_tree(&self.dir(spec), spec).await
    }

    /// Install the toolchain for `spec` unless an intact one is already there.
    ///
    /// # Errors
    /// See [`ToolchainError`]. On any error nothing partial is left in the toolchain directory.
    pub async fn install(&self, spec: &Spec, source: &Source, progress: &(dyn Fn(&str) + Sync)) -> Result<Receipt, ToolchainError> {
        std::fs::create_dir_all(&self.root).map_err(io(format!("create {}", self.root.display())))?;
        let _lock = InstallLock::acquire(&self.root, INSTALL_LOCK_TIMEOUT).await?;
        if let Ok(receipt) = self.verify(spec).await {
            progress("already installed and verified");
            return Ok(receipt);
        }

        let staging = Staging::create(&self.root)?;
        let node_archive = fetch(&spec.node, source, &staging.path, "node.archive", progress).await?;
        let pnpm_archive = fetch(&spec.pnpm, source, &staging.path, "pnpm.archive", progress).await?;

        let tree = staging.path.join("tree");
        let bin = tree.join("bin");
        std::fs::create_dir_all(&bin).map_err(io("create the staging tree"))?;
        progress("extracting");
        let node_sha = extract_entry(&node_archive, &spec.node, &bin.join("node")).await?;
        let pnpm_sha = extract_entry(&pnpm_archive, &spec.pnpm, &bin.join("pnpm")).await?;

        let node_version = program_version(&bin.join("node"), "node", &spec.node_version).await?;
        let pnpm_version = program_version(&bin.join("pnpm"), "pnpm", &spec.pnpm_version).await?;
        let receipt = Receipt {
            key: spec.key.clone(),
            node: ProgramRecord { version: node_version, archive_sha256: spec.node.sha256.clone(), binary_sha256: node_sha },
            pnpm: ProgramRecord { version: pnpm_version, archive_sha256: spec.pnpm.sha256.clone(), binary_sha256: pnpm_sha },
            installed_unix: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        };
        let body = serde_json::to_vec_pretty(&receipt).map_err(io("encode the receipt"))?;
        std::fs::write(tree.join(RECEIPT_FILE), body).map_err(io("write the receipt"))?;

        let target = self.dir(spec);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(io(format!("remove the damaged {}", target.display())))?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io(format!("create {}", parent.display())))?;
        }
        std::fs::rename(&tree, &target).map_err(io(format!("move the toolchain into {}", target.display())))?;
        progress("installed");
        Ok(receipt)
    }
}

async fn verify_tree(dir: &Path, spec: &Spec) -> Result<Receipt, ToolchainError> {
    let body = std::fs::read(dir.join(RECEIPT_FILE)).map_err(io(format!("read {}", dir.join(RECEIPT_FILE).display())))?;
    let receipt: Receipt = serde_json::from_slice(&body).map_err(io("the receipt is not readable"))?;
    if receipt.key != spec.key
        || receipt.node.archive_sha256 != spec.node.sha256
        || receipt.pnpm.archive_sha256 != spec.pnpm.sha256
    {
        return Err(ToolchainError::Mismatch {
            what: "the installed toolchain".into(),
            detail: "its receipt names another key or other archives than the ones pinned now".into(),
        });
    }
    for (name, record, wanted) in [("node", &receipt.node, &spec.node_version), ("pnpm", &receipt.pnpm, &spec.pnpm_version)] {
        let program = dir.join("bin").join(name);
        let path = program.clone();
        let digest = tokio::task::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(io("hash task"))?
            .map_err(io(format!("read {}", program.display())))?;
        if digest != record.binary_sha256 {
            return Err(ToolchainError::Mismatch {
                what: format!("the installed {name}"),
                detail: format!("it hashes to {digest}, the receipt says {}", record.binary_sha256),
            });
        }
        program_version(&program, name, wanted).await?;
    }
    Ok(receipt)
}

/// Run `program --version` with nothing in its environment and require the pinned answer.
async fn program_version(program: &Path, name: &str, wanted: &str) -> Result<String, ToolchainError> {
    let mut command = tokio::process::Command::new(program);
    command
        .arg("--version")
        .env_clear()
        .env("HOME", "/var/empty")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| ToolchainError::WrongProgram { program: name.into(), detail: "`--version` did not finish".into() })?
        .map_err(|error| ToolchainError::WrongProgram { program: name.into(), detail: format!("cannot run {}: {error}", program.display()) })?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || said != wanted {
        return Err(ToolchainError::WrongProgram {
            program: name.into(),
            detail: format!("`--version` printed {said:?} (exit {:?}); the pinned version is {wanted:?}", output.status.code()),
        });
    }
    Ok(said)
}

/// A directory that is removed when dropped, so a failed install leaves nothing behind.
struct Staging {
    path: PathBuf,
}

impl Staging {
    fn create(root: &Path) -> Result<Self, ToolchainError> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = root.join(format!(".staging-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(io(format!("create {}", path.display())))?;
        Ok(Self { path })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Serialises installs into one `toolchains/` directory, across processes.
struct InstallLock {
    file: File,
}

impl InstallLock {
    async fn acquire(root: &Path, timeout: Duration) -> Result<Self, ToolchainError> {
        let path = root.join(INSTALL_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io(format!("open {}", path.display())))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(TryLockError::WouldBlock) => {
                    return Err(ToolchainError::Io(format!(
                        "another install has held {} for {} s",
                        path.display(),
                        timeout.as_secs()
                    )))
                }
                Err(TryLockError::Error(error)) => return Err(ToolchainError::Io(format!("lock {}: {error}", path.display()))),
            }
        }
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Put one pinned archive into `staging/<name>`, verified. Nothing is read from it before it has hashed to the pin.
async fn fetch(
    artifact: &Artifact,
    source: &Source,
    staging: &Path,
    name: &str,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<PathBuf, ToolchainError> {
    let destination = staging.join(name);
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut out = tokio::fs::File::create(&destination).await.map_err(io(format!("create {}", destination.display())))?;
    let mut accept = |chunk: &[u8]| -> Result<(), ToolchainError> {
        total += chunk.len() as u64;
        if total > artifact.size {
            return Err(ToolchainError::Mismatch {
                what: artifact.file_name().to_string(),
                detail: format!("it is larger than the pinned {} bytes; the transfer was cut off", artifact.size),
            });
        }
        hasher.update(chunk);
        Ok(())
    };
    match source {
        Source::Network => {
            progress(&format!("downloading {} ({} bytes)", artifact.url, artifact.size));
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(30 * 60))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let same_scheme = attempt.previous().first().is_some_and(|first| first.scheme() == attempt.url().scheme());
                    if attempt.previous().len() < 5 && same_scheme {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .build()
                .map_err(|error| ToolchainError::Io(format!("http client: {error}")))?;
            let mut response = client
                .get(&artifact.url)
                .send()
                .await
                .map_err(|error| ToolchainError::Unreachable { url: artifact.url.clone(), detail: describe(&error) })?;
            if !response.status().is_success() {
                return Err(ToolchainError::BadResponse { url: artifact.url.clone(), detail: format!("HTTP {}", response.status()) });
            }
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| ToolchainError::Unreachable { url: artifact.url.clone(), detail: describe(&error) })?
            {
                accept(&chunk)?;
                out.write_all(&chunk).await.map_err(io("write the download"))?;
            }
        }
        Source::Directory(dir) => {
            let path = dir.join(artifact.file_name());
            progress(&format!("reading {}", path.display()));
            let mut input = tokio::fs::File::open(&path).await.map_err(|error| ToolchainError::Io(format!(
                "cannot read {}: {error} (the archive for {} must be in that directory under that name)",
                path.display(),
                artifact.url
            )))?;
            let mut buffer = vec![0_u8; 256 * 1024];
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut input, &mut buffer).await.map_err(io(format!("read {}", path.display())))?;
                if n == 0 {
                    break;
                }
                accept(&buffer[..n])?;
                out.write_all(&buffer[..n]).await.map_err(io("copy the archive"))?;
            }
        }
    }
    out.flush().await.map_err(io("flush the download"))?;
    drop(out);
    let digest = hex(&hasher.finalize());
    if total != artifact.size || digest != artifact.sha256 {
        return Err(ToolchainError::Mismatch {
            what: artifact.file_name().to_string(),
            detail: format!("{total} bytes with sha256 {digest}; pinned: {} bytes with sha256 {}", artifact.size, artifact.sha256),
        });
    }
    Ok(destination)
}

fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Write the one regular file `artifact.entry` of the gzip-compressed tar at `archive` to `destination` (mode 0755)
/// and return its SHA-256. Every other entry is skipped unread.
async fn extract_entry(archive: &Path, artifact: &Artifact, destination: &Path) -> Result<String, ToolchainError> {
    let (archive, entry, destination, file) = (archive.to_path_buf(), artifact.entry.clone(), destination.to_path_buf(), artifact.file_name().to_string());
    tokio::task::spawn_blocking(move || -> Result<String, ToolchainError> {
        let bad = |detail: String| ToolchainError::BadArchive { file: file.clone(), detail };
        let reader = File::open(&archive).map_err(io(format!("open {}", archive.display())))?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(reader));
        let entries = tar.entries().map_err(|error| bad(format!("not a readable archive: {error}")))?;
        for item in entries {
            let mut item = item.map_err(|error| bad(format!("unreadable entry: {error}")))?;
            let path = item.path().map_err(|error| bad(format!("unreadable entry name: {error}")))?;
            if path.to_str() != Some(entry.as_str()) {
                continue;
            }
            if !item.header().entry_type().is_file() {
                return Err(bad(format!("{entry} is not a regular file (a link or a directory in its place is not accepted)")));
            }
            if item.header().size().map_err(|e| bad(format!("{entry}: {e}")))? > MAX_BINARY_BYTES {
                return Err(bad(format!("{entry} claims more than {MAX_BINARY_BYTES} bytes")));
            }
            let mut out = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o755)
                .open(&destination)
                .map_err(io(format!("create {}", destination.display())))?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0_u8; 256 * 1024];
            let mut written: u64 = 0;
            loop {
                let n = item.read(&mut buffer).map_err(|error| bad(format!("{entry}: {error}")))?;
                if n == 0 {
                    break;
                }
                written += n as u64;
                if written > MAX_BINARY_BYTES {
                    return Err(bad(format!("{entry} expands past {MAX_BINARY_BYTES} bytes")));
                }
                hasher.update(&buffer[..n]);
                out.write_all(&buffer[..n]).map_err(io("write the program"))?;
            }
            out.flush().map_err(io("flush the program"))?;
            return Ok(hex(&hasher.finalize()));
        }
        Err(bad(format!("it does not contain {entry}")))
    })
    .await
    .map_err(io("extract task"))?
}

#[cfg(test)]
mod pins {
    use super::*;

    #[test]
    fn the_pinned_key_is_the_one_the_service_builds_against() {
        assert_eq!(TOOLCHAIN_KEY, local_app_builder_service::runtime_profiles::RUNTIME_PROFILE_TOOLCHAIN_KEY);
        assert!(TOOLCHAIN_KEY.contains(&format!("node@{}", NODE_VERSION_OUTPUT.trim_start_matches('v'))));
        assert!(TOOLCHAIN_KEY.contains(&format!("pnpm@{PNPM_VERSION_OUTPUT}")));
    }

    #[test]
    fn every_pin_is_complete_and_comes_from_its_own_publisher_over_https() {
        for platform in [Platform::DarwinArm64, Platform::DarwinX64] {
            let spec = Spec::pinned(platform);
            for (artifact, host) in [(&spec.node, "https://nodejs.org/dist/v26.9.0/"), (&spec.pnpm, "https://registry.npmjs.org/@pnpm/exe.")] {
                assert!(artifact.url.starts_with(host), "{}", artifact.url);
                assert!(artifact.url.contains(platform.name()), "{}", artifact.url);
                assert_eq!(artifact.sha256.len(), 64);
                assert!(artifact.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", artifact.sha256);
                assert!(artifact.size > 1_000_000, "{}", artifact.url);
            }
            assert_eq!(spec.node.entry, format!("node-v26.9.0-{}/bin/node", platform.name()));
            assert_eq!(spec.pnpm.entry, "package/pnpm");
        }
        let (a, b) = (Spec::pinned(Platform::DarwinArm64), Spec::pinned(Platform::DarwinX64));
        assert_ne!(a.node.sha256, b.node.sha256);
        assert_ne!(a.pnpm.sha256, b.pnpm.sha256);
    }

    #[test]
    fn the_node_pin_for_apple_silicon_is_the_one_p1_downloaded_and_checked() {
        // 6f3de7ed… is what the P1 run hashed the downloaded archive to, and what SHASUMS256.txt said.
        assert!(Spec::pinned(Platform::DarwinArm64).node.sha256.starts_with("6f3de7ed853ee283"));
    }
}
