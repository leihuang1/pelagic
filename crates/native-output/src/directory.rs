//! Recoverable file-group publication. Not a concurrent-reader snapshot or crash journal.
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Three CLI formats; HTTP continues using its existing single-file adapter.
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

/// Run a writer in exclusively owned scratch, then update only verified owned artifacts.
/// All writer handles must be closed before it returns. Error conversion stays solver-specific.
/// # Errors
/// Rejects conflicts, modified owned files, input aliases, symlinks, quotas and I/O failures.
/// Installation failure rolls back; rollback failure preserves scratch/backup and names them.
pub fn publish<T, E: From<String>>(
    output: &Path,
    inputs: &[&Path],
    maximum: Option<u64>,
    write: impl FnOnce(&Path) -> Result<T, E>,
) -> Result<T, E> {
    let mut transaction = Transaction::begin(output, inputs).map_err(E::from)?;
    let result = write(&transaction.stage)?;
    transaction.install(inputs, maximum).map_err(E::from)?;
    Ok(result)
}

struct Transaction {
    output: PathBuf,
    stage: PathBuf,
    backup: PathBuf,
    lock: PathBuf,
    created_output: bool,
    owned: Vec<PathBuf>,
    retain: bool,
    #[cfg(test)]
    before_install: Option<fn(&Self)>,
}

impl Transaction {
    fn begin(output: &Path, inputs: &[&Path]) -> Result<Self, String> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(message)?;
        protect(output, inputs)?;
        let created_output = match fs::symlink_metadata(output) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err("output must be a real result directory, not a file or symlink".into());
            }
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(output).map_err(message)?;
                true
            }
            Err(e) => return Err(message(e)),
        };
        for name in [
            MANIFEST,
            ".pelagic-lock",
            ".pelagic-stage",
            ".pelagic-backup",
        ] {
            protect(&output.join(name), inputs)?;
        }
        let mut this = Self {
            output: output.to_path_buf(),
            stage: output.join(".pelagic-stage"),
            backup: output.join(".pelagic-backup"),
            lock: output.join(".pelagic-lock"),
            created_output,
            owned: Vec::new(),
            retain: false,
            #[cfg(test)]
            before_install: None,
        };
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
        fs::create_dir(&this.stage)
            .map_err(|e| format!("unable to reserve scratch {}: {e}", this.stage.display()))?;
        this.owned.push(this.stage.clone());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&this.stage, fs::Permissions::from_mode(0o700)).map_err(message)?;
        }
        Ok(this)
    }

    #[allow(clippy::too_many_lines)]
    fn install(&mut self, inputs: &[&Path], maximum: Option<u64>) -> Result<(), String> {
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
            protect(&self.output.join(&artifact.name), inputs)?;
        }
        for name in &names {
            let target = self.output.join(name);
            protect(&target, inputs)?;
            if fs::symlink_metadata(&target).is_ok()
                && !old_artifacts.iter().any(|a| &a.name == name)
            {
                return Err(format!("unowned artifact conflict: {}", target.display()));
            }
        }
        protect(&old_path, inputs)?;
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
        check_quota(&self.stage, maximum)?;
        for name in names.iter().map(String::as_str).chain([MANIFEST]) {
            // Windows FlushFileBuffers requires write access, as in single-file publication.
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.stage.join(name))
                .and_then(|f| f.sync_all())
                .map_err(message)?;
        }
        // All validation happens before moving any previous product. Recheck at each move.
        fs::create_dir(&self.backup).map_err(message)?;
        self.owned.push(self.backup.clone());
        let mut moved = Vec::new();
        let mut installed = Vec::new();
        let installation: Result<(), String> = (|| {
            for artifact in old_artifacts.iter().chain(old_manifest.iter()) {
                let target = self.output.join(&artifact.name);
                protect(&target, inputs)?;
                if verify(&target, artifact, true)? {
                    fs::rename(&target, self.backup.join(&artifact.name)).map_err(message)?;
                    moved.push(artifact.name.as_str());
                }
            }
            #[cfg(test)]
            if let Some(hook) = self.before_install {
                hook(self);
            }
            for name in names.iter().map(String::as_str).chain([MANIFEST]) {
                protect(&self.output.join(name), inputs)?;
                // Exclusive installation refuses late destinations, never truncates them.
                fs::hard_link(self.stage.join(name), self.output.join(name))
                    .map_err(|e| format!("unable to install {name}: {e}"))?;
                installed.push(name);
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

    fn rollback(&self, installed: &[&str], moved: &[&str]) -> Result<(), String> {
        let mut errors = Vec::new();
        for name in installed.iter().rev() {
            let expected = fingerprint(&self.stage.join(name), name)?;
            match verify(&self.output.join(name), &expected, false)
                .and_then(|_| fs::remove_file(self.output.join(name)).map_err(message))
            {
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

fn protect(target: &Path, inputs: &[&Path]) -> Result<(), String> {
    let existing = fs::canonicalize(target).ok();
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = fs::canonicalize(parent)
        .ok()
        .and_then(|p| target.file_name().map(|name| p.join(name)));
    for input in inputs {
        let input = fs::canonicalize(input).map_err(message)?;
        let mut alias = existing.as_ref() == Some(&input) || destination.as_ref() == Some(&input);
        if fs::metadata(target).is_ok_and(|m| m.is_file()) {
            alias |= same_file::is_same_file(target, &input).map_err(message)?;
        }
        if alias {
            return Err("output must not replace an input file or its alias".into());
        }
    }
    Ok(())
}

fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
#[path = "directory_tests.rs"]
mod tests;
