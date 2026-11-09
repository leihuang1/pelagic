//! Recoverable file-group publication. Not a concurrent-reader snapshot or crash journal.
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(target_vendor = "apple")]
mod batch;
#[cfg(not(target_vendor = "apple"))]
#[path = "directory/unbatched.rs"]
mod batch;
mod syncing;
pub use batch::Batch;
pub use syncing::raise_open_file_limit;

const MANIFEST: &str = "pelagic-manifest.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    implementation: String,
    artifacts: Vec<Artifact>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    size_bytes: u64,
    sha256: String,
}

/// The CLI's three formats. HTTP instead returns one HDF5 file it writes privately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Legacy,
    Hdf5,
    Both,
}

impl Format {
    #[must_use]
    pub fn legacy(self) -> bool {
        self != Self::Hdf5
    }
    #[must_use]
    pub fn hdf5(self) -> bool {
        self != Self::Legacy
    }
}

/// Artifact basename derived from a supplied input, never from mutable acoustic metadata.
/// # Errors
/// Returns an error if the input has no file stem. Unsafe/reserved artifact spelling is normalized.
pub fn stem(input: &Path) -> Result<String, String> {
    let stem = input
        .file_stem()
        .ok_or("input must name a file")?
        .to_string_lossy();
    let mut stem: String = stem
        .chars()
        .map(|c| if c.is_control() || c == '\\' { '_' } else { c })
        .collect();
    if stem.starts_with(".pelagic-") {
        stem.insert_str(0, "case-");
    }
    valid_name(&stem)?;
    Ok(stem)
}

/// SHA-256 of the manifest in a result directory, or `None` without one.
/// # Errors
/// Rejects an invalid or symlinked manifest, and filesystem errors.
pub fn manifest_sha256(output: &Path) -> Result<Option<String>, String> {
    let path = output.join(MANIFEST);
    match read_manifest(&path)? {
        Some(_) => fingerprint(&path, MANIFEST).map(|manifest| Some(manifest.sha256)),
        None => Ok(None),
    }
}

/// [`manifest_sha256`], once every artifact the manifest lists still matches it.
/// # Errors
/// Also rejects a missing or modified artifact.
pub fn verified_manifest_sha256(output: &Path) -> Result<Option<String>, String> {
    let path = output.join(MANIFEST);
    let Some(manifest) = read_manifest(&path)? else {
        return Ok(None);
    };
    for artifact in &manifest.artifacts {
        verify(&output.join(&artifact.name), artifact, false)?;
    }
    fingerprint(&path, MANIFEST).map(|manifest| Some(manifest.sha256))
}

/// Run a writer in exclusively owned scratch, then update only verified owned artifacts.
/// All writer handles must be closed before it returns. Error conversion stays solver-specific.
/// # Errors
/// Rejects conflicts, modified owned files, input aliases, symlinks, quotas and I/O failures.
/// Installation failure rolls back; rollback failure preserves scratch/backup and names them.
/// A [`Batch`] shares Apple drive flushes before installation; no directory durability is promised.
pub fn publish<T, E: From<String>>(
    output: &Path,
    inputs: &[&Path],
    maximum: Option<u64>,
    write: impl FnOnce(&Path) -> Result<T, E>,
) -> Result<T, E> {
    let mut transaction = Transaction::begin(output, inputs).map_err(E::from)?;
    let result = write(&transaction.stage)?;
    transaction.install(maximum).map_err(E::from)?;
    Ok(result)
}

struct Transaction {
    output: PathBuf,
    resolved_output: Option<PathBuf>,
    inputs: Vec<PathBuf>,
    stage: PathBuf,
    backup: PathBuf,
    lock: PathBuf,
    created_output: bool,
    fresh: bool,
    permissions: Option<fs::Permissions>,
    owned: Vec<PathBuf>,
    retain: bool,
    #[cfg(target_vendor = "apple")]
    batch: Option<std::sync::Arc<Batch>>,
    #[cfg(test)]
    before_install: Option<fn(&Self)>,
    #[cfg(test)]
    sync: fn(&File) -> std::io::Result<()>,
    #[cfg(all(test, target_vendor = "apple"))]
    full_sync: fn(&File) -> std::io::Result<()>,
    #[cfg(test)]
    install_file: fn(&Path, &Path) -> std::io::Result<bool>,
    #[cfg(test)]
    after_install: Option<fn(&Self, &str)>,
    #[cfg(test)]
    after_step: Option<fn(&Self, &str)>,
    #[cfg(test)]
    install_directory: fn(&Path, &Path) -> std::io::Result<bool>,
    #[cfg(all(test, target_vendor = "apple"))]
    wait: fn(&Batch, File) -> Result<(), String>,
}

impl Transaction {
    #[allow(clippy::too_many_lines)] // Keep both reservations under the same cleanup guard.
    fn begin(output: &Path, inputs: &[&Path]) -> Result<Self, String> {
        let parent = parent(output);
        fs::create_dir_all(parent).map_err(message)?;
        let inputs = inputs
            .iter()
            .map(|input| fs::canonicalize(input).map_err(message))
            .collect::<Result<Vec<_>, _>>()?;
        protect(output, &inputs, fs::canonicalize(parent).ok().as_deref())?;
        let sibling = sibling_stage(output);
        let mut fresh = false;
        let created_output = match fs::symlink_metadata(output) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err("output must be a real result directory, not a file or symlink".into());
            }
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if sibling.is_some() {
                    fresh = true;
                    false
                } else {
                    fs::create_dir(output).map_err(message)?;
                    true
                }
            }
            Err(e) => return Err(message(e)),
        };
        if !fresh
            && sibling
                .as_ref()
                .is_some_and(|path| fs::symlink_metadata(path).is_ok())
        {
            return Err(format!(
                "unable to reserve result directory ({}): scratch exists",
                output.join(".pelagic-lock").display()
            ));
        }
        let resolved_output = fs::canonicalize(output).ok().or_else(|| {
            fs::canonicalize(parent)
                .ok()
                .and_then(|p| output.file_name().map(|name| p.join(name)))
        });
        for name in [
            MANIFEST,
            ".pelagic-lock",
            ".pelagic-stage",
            ".pelagic-backup",
        ] {
            protect(&output.join(name), &inputs, resolved_output.as_deref())?;
        }
        let mut this = Self {
            output: output.to_path_buf(),
            resolved_output,
            inputs,
            stage: if fresh {
                sibling.expect("fresh publication has a sibling")
            } else {
                output.join(".pelagic-stage")
            },
            backup: output.join(".pelagic-backup"),
            lock: output.join(".pelagic-lock"),
            created_output,
            fresh,
            permissions: None,
            owned: Vec::new(),
            retain: false,
            #[cfg(target_vendor = "apple")]
            batch: Batch::current(),
            #[cfg(test)]
            before_install: None,
            #[cfg(test)]
            sync: sync_file,
            #[cfg(all(test, target_vendor = "apple"))]
            full_sync,
            #[cfg(test)]
            install_file,
            #[cfg(test)]
            after_install: None,
            #[cfg(test)]
            after_step: None,
            #[cfg(test)]
            install_directory,
            #[cfg(all(test, target_vendor = "apple"))]
            wait: Batch::wait,
        };
        if !fresh {
            fs::create_dir(&this.lock).map_err(|e| {
                format!(
                    "unable to reserve result directory ({}): {e}",
                    this.lock.display()
                )
            })?;
            this.owned.push(this.lock.clone());
            if fs::symlink_metadata(&this.backup).is_ok() {
                return Err(format!(
                    "unowned or recoverable backup exists: {}",
                    this.backup.display()
                ));
            }
        }
        fs::create_dir(&this.stage)
            .map_err(|e| format!("unable to reserve scratch {}: {e}", this.stage.display()))?;
        this.owned.push(this.stage.clone());
        if fresh {
            this.permissions = Some(fs::metadata(&this.stage).map_err(message)?.permissions());
            // The sibling is the reservation until rename; the inner lock travels with it.
            if fs::symlink_metadata(output).is_ok() {
                return Err(format!(
                    "unable to reserve result directory ({}): destination appeared",
                    this.lock.display()
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&this.stage, fs::Permissions::from_mode(0o700)).map_err(message)?;
        }
        Ok(this)
    }

    #[allow(clippy::too_many_lines)]
    fn install(&mut self, maximum: Option<u64>) -> Result<(), String> {
        #[cfg(test)]
        self.step("staged");
        let mut names = BTreeSet::new();
        for entry in fs::read_dir(&self.stage).map_err(message)? {
            let entry = entry.map_err(message)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "artifact name must be UTF-8")?;
            valid_name(&name)?;
            if name == MANIFEST || !entry.file_type().map_err(message)?.is_file() {
                return Err("writer produced a reserved name or non-regular artifact".into());
            }
            names.insert(name);
        }
        let artifacts = names
            .iter()
            .map(|name| fingerprint(&self.stage.join(name), name))
            .collect::<Result<Vec<_>, _>>()?;
        let manifest = Manifest {
            schema_version: 1,
            implementation: "Pelagic".into(),
            artifacts,
        };
        let old_path = self.output.join(MANIFEST);
        let old = read_manifest(&old_path)?;
        let old_manifest = old
            .as_ref()
            .map(|_| fingerprint(&old_path, MANIFEST))
            .transpose()?;
        let old_artifacts = old.as_ref().map_or(&[][..], |old| old.artifacts.as_slice());
        for artifact in old_artifacts {
            verify(&self.output.join(&artifact.name), artifact, true)?;
            self.protect(&self.output.join(&artifact.name))?;
        }
        for name in &names {
            let target = self.output.join(name);
            self.protect(&target)?;
            if fs::symlink_metadata(&target).is_ok()
                && !old_artifacts.iter().any(|a| &a.name == name)
            {
                return Err(format!("unowned artifact conflict: {}", target.display()));
            }
        }
        self.protect(&old_path)?;
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(message)?;
        if bytes.len() > 1024 * 1024 {
            return Err("artifact manifest exceeds 1 MiB".into());
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.stage.join(MANIFEST))
            .and_then(|mut f| f.write_all(&bytes))
            .map_err(message)?;
        let new_manifest = Artifact {
            name: MANIFEST.into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        check_quota(&self.stage, maximum)?;
        names.insert(MANIFEST.into());
        let paths: Vec<_> = names.iter().map(|name| self.stage.join(name)).collect();
        self.sync_files(&paths)?;
        #[cfg(target_vendor = "apple")]
        {
            let handle = open_for_sync(&self.stage.join(MANIFEST)).map_err(message)?;
            if let Some(batch) = &self.batch {
                #[cfg(test)]
                let wait = self.wait;
                #[cfg(not(test))]
                let wait = Batch::wait;
                wait(batch, handle)?;
            } else {
                #[cfg(test)]
                let full_sync = self.full_sync;
                full_sync(&handle).map_err(message)?;
            }
        }
        #[cfg(test)]
        {
            self.step("barrier");
            self.step("synced");
        }
        if self.fresh {
            #[cfg(test)]
            if let Some(hook) = self.before_install {
                hook(self);
            }
            self.protect(&self.output)?;
            let lock = self.stage.join(".pelagic-lock");
            fs::create_dir(&lock).map_err(message)?;
            self.owned.push(lock);
            if let Some(permissions) = self.permissions.take() {
                fs::set_permissions(&self.stage, permissions).map_err(message)?;
            }
            #[cfg(test)]
            let install_directory = self.install_directory;
            match install_directory(&self.stage, &self.output) {
                Ok(true) => {
                    self.owned.clear();
                    self.owned.push(self.lock.clone());
                    self.stage.clone_from(&self.output);
                    #[cfg(test)]
                    self.step("renamed");
                    #[cfg(test)]
                    self.step("installed");
                    return Ok(());
                }
                Ok(false) => {
                    // No exclusive directory rename on this filesystem: reserve the destination
                    // before falling back to the original exclusive per-file installation.
                    fs::create_dir(&self.output).map_err(message)?;
                    self.created_output = true;
                    fs::create_dir(&self.lock).map_err(message)?;
                    self.owned.push(self.lock.clone());
                    fs::remove_dir(self.stage.join(".pelagic-lock")).map_err(message)?;
                    self.fresh = false;
                }
                Err(e) => {
                    return Err(format!(
                        "unable to install result directory {}: {e}",
                        self.output.display()
                    ));
                }
            }
        }
        // All validation happens before moving any previous product. Recheck at each move.
        fs::create_dir(&self.backup).map_err(message)?;
        self.owned.push(self.backup.clone());
        let mut moved = Vec::new();
        let mut installed = Vec::new();
        let installation: Result<(), String> = (|| {
            for artifact in old_artifacts.iter().chain(old_manifest.iter()) {
                let target = self.output.join(&artifact.name);
                self.protect(&target)?;
                if verify(&target, artifact, true)? {
                    fs::rename(&target, self.backup.join(&artifact.name)).map_err(message)?;
                    moved.push(artifact.name.as_str());
                }
            }
            #[cfg(test)]
            if let Some(hook) = self.before_install {
                hook(self);
            }
            #[cfg(test)]
            let install = self.install_file;
            #[cfg(not(test))]
            let install = install_file;
            for artifact in manifest.artifacts.iter().chain([&new_manifest]) {
                let name = &artifact.name;
                self.protect(&self.output.join(name))?;
                // Exclusive installation refuses late destinations, never truncates them.
                let renamed = install(&self.stage.join(name), &self.output.join(name))
                    .map_err(|e| format!("unable to install {name}: {e}"))?;
                installed.push((artifact, renamed));
                #[cfg(test)]
                if let Some(hook) = self.after_install {
                    hook(self, name);
                }
            }
            Ok(())
        })();
        if let Err(error) = installation {
            let rollback = self.rollback(&installed, &moved);
            if let Err(rollback) = rollback {
                self.retain = true;
                return Err(format!(
                    "{error}; rollback failed: {rollback}; recover old results from {}; new data retained at {}; lock retained at {}",
                    self.backup.display(),
                    self.stage.display(),
                    self.lock.display()
                ));
            }
            return Err(format!("{error}; previous results restored"));
        }
        #[cfg(test)]
        self.step("installed");
        // The manifest is the commit marker, not a promise of atomic visibility to readers.
        if let Err(e) = fs::remove_dir_all(&self.backup) {
            self.retain = true;
            return Err(format!(
                "results installed; backup cleanup failed: {e}; recoverable data retained at {}",
                self.backup.display()
            ));
        }
        self.owned.pop();
        Ok(())
    }

    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn sync_files(&self, paths: &[PathBuf]) -> Result<(), String> {
        #[cfg(test)]
        let sync = self.sync;
        #[cfg(not(test))]
        let sync = sync_file;
        syncing::sync_paths(paths, |path| sync_path(path, sync))
            .into_iter()
            .collect::<Result<(), _>>()
    }

    #[cfg(test)]
    fn step(&self, name: &str) {
        if let Some(hook) = self.after_step {
            hook(self, name);
        }
    }

    fn protect(&self, target: &Path) -> Result<(), String> {
        // Inputs and the real parent are fixed under our lock; recheck each destination and its
        // current file identity at every move, so even late symlink/hard-link aliases are refused.
        protect(target, &self.inputs, self.resolved_output.as_deref())
    }

    fn rollback(&self, installed: &[(&Artifact, bool)], moved: &[&str]) -> Result<(), String> {
        let mut errors = Vec::new();
        for (expected, renamed) in installed.iter().rev() {
            let target = self.output.join(&expected.name);
            // Rename consumed the stage copy. Verify against its pre-install fingerprint by
            // reading the destination, then put it back exclusively before restoring old files.
            match verify(&target, expected, false).and_then(|_| {
                if *renamed
                    && install_file(&target, &self.stage.join(&expected.name)).map_err(message)?
                {
                    return Ok(());
                }
                fs::remove_file(&target).map_err(message)
            }) {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
        }
        for name in moved.iter().rev() {
            if let Err(e) = fs::hard_link(self.backup.join(name), self.output.join(name)) {
                errors.push(format!("unable to restore {name}: {e}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.retain {
            return;
        }
        for path in self.owned.iter().rev() {
            let removal = if path == &self.lock {
                fs::remove_dir(path)
            } else {
                fs::remove_dir_all(path)
            };
            if let Err(e) = removal
                && e.kind() != std::io::ErrorKind::NotFound
            {
                crate::diagnostic::cleanup(path, &e);
            }
        }
        if self.created_output {
            let _ = fs::remove_dir(&self.output);
        }
    }
}

// Long/non-component names and other platforms retain the original path and its refusals.
fn sibling_stage(output: &Path) -> Option<PathBuf> {
    if !cfg!(any(target_vendor = "apple", target_os = "linux")) {
        return None;
    }
    let name = output.file_name()?;
    if name.as_encoded_bytes().len() > 240 {
        return None;
    }
    let mut sibling = std::ffi::OsString::from(".pelagic-");
    sibling.push(name);
    sibling.push("-stage");
    Some(parent(output).join(sibling))
}

/// False only if the filesystem does not support exclusive directory rename.
// Other targets have no exclusive rename, so there it cannot fail.
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux")),
    allow(clippy::unnecessary_wraps)
)]
fn install_directory(source: &Path, target: &Path) -> std::io::Result<bool> {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        match renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE) {
            Ok(()) => Ok(true),
            Err(e) if unsupported_rename(e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    {
        let _ = (source, target);
        Ok(false)
    }
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
fn unsupported_rename(error: rustix::io::Errno) -> bool {
    use rustix::io::Errno;
    [
        Errno::INVAL,
        Errno::NOSYS,
        Errno::NOTSUP,
        Errno::OPNOTSUPP,
        Errno::XDEV,
    ]
    .contains(&error)
}

/// True when rename consumed the source; false when the hard-link fallback retained it.
fn install_file(source: &Path, target: &Path) -> std::io::Result<bool> {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    {
        install_file_with(source, target, |source, target| {
            use rustix::fs::{CWD, RenameFlags, renameat_with};
            // renameatx_np(RENAME_EXCL) on Apple, renameat2(RENAME_NOREPLACE) on Linux.
            renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE)
        })
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    fs::hard_link(source, target).map(|()| false)
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
fn install_file_with(
    source: &Path,
    target: &Path,
    rename: impl FnOnce(&Path, &Path) -> rustix::io::Result<()>,
) -> std::io::Result<bool> {
    match rename(source, target) {
        Ok(()) => Ok(true),
        // Unsupported syscall/flag/filesystem: retain the original exclusive hard-link path.
        // Do not retry conflicts, permissions or I/O errors as a different operation.
        Err(e) if unsupported_rename(e) => fs::hard_link(source, target).map(|()| false),
        Err(e) => Err(e.into()),
    }
}

fn open_for_sync(path: &Path) -> std::io::Result<File> {
    // Windows FlushFileBuffers requires write access.
    fs::OpenOptions::new().read(true).write(true).open(path)
}

fn sync_path(path: &Path, sync: fn(&File) -> std::io::Result<()>) -> Result<(), syncing::Failure> {
    sync(&open_for_sync(path).map_err(syncing::Failure::open)?).map_err(syncing::Failure::Other)
}

#[cfg(target_vendor = "apple")]
fn sync_file(file: &File) -> std::io::Result<()> {
    // Apple sync_all includes a drive flush; defer that until every ordinary fsync is done.
    rustix::io::retry_on_intr(|| rustix::fs::fsync(file)).map_err(Into::into)
}

#[cfg(target_vendor = "apple")]
fn full_sync(file: &File) -> std::io::Result<()> {
    rustix::io::retry_on_intr(|| rustix::fs::fcntl_fullfsync(file)).map_err(Into::into)
}

#[cfg(not(target_vendor = "apple"))]
fn sync_file(file: &File) -> std::io::Result<()> {
    file.sync_all()
}

fn valid_name(name: &str) -> Result<(), String> {
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || name.starts_with(".pelagic-")
        || name.contains(['\\', '\n', '\r'])
    {
        return Err(format!("invalid artifact name: {name:?}"));
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<Option<Manifest>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() || m.len() > 1024 * 1024 => {
            return Err("invalid or symlinked Pelagic manifest".into());
        }
        Err(e) => return Err(message(e)),
        _ => {}
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(message)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(message)?;
    if bytes.len() > 1024 * 1024 {
        return Err("Pelagic manifest exceeds 1 MiB".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid Pelagic manifest: {e}"))?;
    if manifest.schema_version != 1 || manifest.implementation != "Pelagic" {
        return Err("unrecognized Pelagic manifest".into());
    }
    let mut names = BTreeSet::new();
    for artifact in &manifest.artifacts {
        valid_name(&artifact.name)?;
        if artifact.name == MANIFEST
            || !names.insert(&artifact.name)
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid Pelagic manifest artifact".into());
        }
    }
    Ok(Some(manifest))
}

fn fingerprint(path: &Path, name: &str) -> Result<Artifact, String> {
    let metadata = fs::symlink_metadata(path).map_err(message)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "artifact must be a regular file, not a symlink: {}",
            path.display()
        ));
    }
    let mut file = File::open(path).map_err(message)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(message)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(Artifact {
        name: name.into(),
        size_bytes: metadata.len(),
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn verify(path: &Path, expected: &Artifact, missing_ok: bool) -> Result<bool, String> {
    if missing_ok
        && let Err(e) = fs::symlink_metadata(path)
        && e.kind() == std::io::ErrorKind::NotFound
    {
        return Ok(false);
    }
    let actual = fingerprint(path, &expected.name)?;
    if actual.sha256 != expected.sha256 || actual.size_bytes != expected.size_bytes {
        return Err(format!(
            "owned artifact was modified; refusing replacement: {}",
            path.display()
        ));
    }
    Ok(true)
}

/// Count physical artifact bytes and enforce an optional file-group quota.
/// # Errors
/// Returns filesystem, overflow or quota errors.
pub fn check_quota(path: &Path, maximum: Option<u64>) -> Result<u64, String> {
    let total = fs::read_dir(path)
        .map_err(message)?
        .try_fold(0_u64, |total, entry| {
            let size = entry.map_err(message)?.metadata().map_err(message)?.len();
            total
                .checked_add(size)
                .ok_or_else(|| "output size overflow".to_owned())
        })?;
    if let Some(maximum) = maximum.filter(|m| total > *m) {
        return Err(format!("result directory exceeds {maximum} bytes"));
    }
    Ok(total)
}

/// Physical sizes in exclusively owned writer scratch. Closed artifacts need no further stats;
/// writers refresh only files they grow. Publication still scans the whole directory at install.
pub struct Quota {
    sizes: HashMap<PathBuf, u64>,
    total: u64,
    maximum: u64,
}

impl Quota {
    /// Inventory existing files without enforcing the limit ahead of the writer's check points.
    /// # Errors
    /// Returns filesystem or size-overflow errors.
    pub fn new(root: &Path, maximum: u64) -> Result<Self, String> {
        let mut quota = Self {
            sizes: HashMap::new(),
            total: 0,
            maximum,
        };
        for entry in fs::read_dir(root).map_err(message)? {
            quota.refresh(&entry.map_err(message)?.path())?;
        }
        Ok(quota)
    }

    /// Account for a newly closed artifact or the current physical size of a growing file.
    /// This does not flush buffers or check the limit: their ordering belongs to the writer.
    /// # Errors
    /// Returns filesystem or size-overflow errors.
    pub fn refresh(&mut self, path: &Path) -> Result<(), String> {
        let size = fs::metadata(path).map_err(message)?.len();
        let previous = self.sizes.get(path).copied().unwrap_or(0);
        self.total = (self.total - previous)
            .checked_add(size)
            .ok_or("output size overflow")?;
        self.sizes.insert(path.to_path_buf(), size);
        Ok(())
    }

    /// Enforce the directory limit at the same point as a full quota scan.
    /// # Errors
    /// Returns a quota error when the accounted physical total exceeds the limit.
    pub fn remaining(&self) -> Result<u64, String> {
        self.maximum
            .checked_sub(self.total)
            .ok_or_else(|| format!("result directory exceeds {} bytes", self.maximum))
    }
}

fn protect(target: &Path, inputs: &[PathBuf], parent: Option<&Path>) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let existing = fs::canonicalize(target).ok();
    let destination = parent.and_then(|p| target.file_name().map(|name| p.join(name)));
    let is_file = fs::metadata(target).is_ok_and(|m| m.is_file());
    for input in inputs {
        let mut alias = existing.as_ref() == Some(input) || destination.as_ref() == Some(input);
        if is_file {
            alias |= same_file::is_same_file(target, input).map_err(message)?;
        }
        if alias {
            return Err("output must not replace an input file or its alias".into());
        }
    }
    Ok(())
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
#[path = "directory_tests.rs"]
mod tests;
