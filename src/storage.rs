//! Cooperative, local-filesystem transactions. The journal lives outside authored bases.
use crate::Result;
use anyhow::{Context, bail, ensure};
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File, OpenOptions},
    io::Write as IoWrite,
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_FILE: u64 = 64 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Write {
    pub path: String,
    /// None means the path must not exist; Some is the expected SHA256 of its bytes.
    pub expected: Option<String>,
    pub bytes: Option<Vec<u8>>,
}
pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}
pub fn json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut b = serde_json::to_vec_pretty(value)?;
    b.push(b'\n');
    Ok(b)
}

// serde_json normally silently accepts duplicate object keys. All levels reject them here.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element::<Unique>()? {
                    out.push(v.0)
                }
                Ok(Unique(out.into()))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom(format!("duplicate JSON key: {k}")));
                    }
                    out.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn parse_json(bytes: &[u8]) -> Result<Value> {
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "size_limit: JSON exceeds 64 MiB"
    );
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}
pub fn read_json(path: &Path) -> Result<Value> {
    parse_json(&read_bytes(path)?).with_context(|| format!("invalid_json: {}", path.display()))
}
pub fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.file_type().is_file(),
        "unsafe_path: expected a regular file"
    );
    ensure!(m.len() <= MAX_FILE, "size_limit: file exceeds 64 MiB");
    Ok(fs::read(path)?)
}

pub fn relative_path(relative: &str) -> Result<&Path> {
    ensure!(
        !relative.is_empty() && !relative.contains('\\') && !relative.contains('\0'),
        "unsafe_path: invalid relative path"
    );
    let rel = Path::new(relative);
    ensure!(!rel.is_absolute(), "unsafe_path: absolute path");
    ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_))),
        "unsafe_path: dot or parent path component"
    );
    ensure!(
        rel.components()
            .map(|x| x.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
            == relative,
        "unsafe_path: noncanonical relative path"
    );
    Ok(rel)
}

pub fn confined(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = relative_path(relative)?;
    let mut out = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(n) => {
                out.push(n);
                match fs::symlink_metadata(&out) {
                    Ok(m) => ensure!(
                        !m.file_type().is_symlink() && (m.is_dir() || m.is_file()),
                        "unsafe_path: symlink or special file"
                    ),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            _ => bail!("unsafe_path: dot or parent path component"),
        }
    }
    Ok(out)
}
pub fn safe_absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut p = PathBuf::new();
    for c in path.components() {
        match c {
            Component::RootDir => p.push("/"),
            Component::Normal(n) => {
                p.push(n);
                if let Ok(m) = fs::symlink_metadata(&p) {
                    ensure!(!m.file_type().is_symlink(), "unsafe_path: symlinked output")
                }
            }
            _ => bail!("unsafe_path: output must have no dot/parent components"),
        }
    }
    Ok(p)
}
pub fn private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(
            fs::symlink_metadata(path)?.is_dir(),
            "unsafe_path: expected directory"
        );
        return Ok(());
    }
    if let Some(p) = path.parent() {
        private_dir(p)?;
    }
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
            fs::symlink_metadata(path)?.is_dir(),
            "unsafe_path: expected directory"
        ),
        Err(e) => return Err(e.into()),
    };
    Ok(())
}
pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing parent")?;
    private_dir(parent)?;
    if let Ok(m) = fs::symlink_metadata(path) {
        ensure!(
            m.file_type().is_file(),
            "unsafe_path: destination is not regular"
        );
    }
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    stage
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    stage.write_all(bytes)?;
    stage.as_file().sync_all()?;
    stage.persist(path).map_err(|e| e.error)?;
    sync_dir(parent)
}

pub struct Lock(File);
impl Lock {
    pub fn acquire(library: &Path, name: &str, exclusive: bool) -> Result<Self> {
        ensure!(
            !name.contains('/') && !name.contains('\\'),
            "invalid lock name"
        );
        let dir = confined(library, ".runtime/locks")?;
        private_dir(&dir)?;
        let p = confined(library, &format!(".runtime/locks/{name}.lock"))?;
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(p)?;
        let lock = Self(f);
        lock.set(exclusive)?;
        Ok(lock)
    }
    pub fn set(&self, exclusive: bool) -> Result<()> {
        let start = Instant::now();
        loop {
            let flags = if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            };
            // SAFETY: fd stays alive throughout flock, which does not borrow memory.
            if unsafe { libc::flock(self.0.as_raw_fd(), flags | libc::LOCK_NB) } == 0 {
                return Ok(());
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::WouldBlock {
                return Err(e.into());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "busy: another writer holds the library lock"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn unlock(&self) -> Result<()> {
        // SAFETY: fd remains valid.
        if unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.unlock();
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: String,
    old: Option<String>,
    new: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    phase: String,
    entries: Vec<Entry>,
    created_dirs: Vec<String>,
}
fn current(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(read_bytes(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn apply_bytes(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    match bytes {
        Some(b) => atomic(path, b),
        None => {
            if path.exists() {
                fs::remove_file(path)?;
                sync_dir(path.parent().unwrap())?;
            }
            Ok(())
        }
    }
}
fn decode(value: &Option<String>) -> Result<Option<Vec<u8>>> {
    value
        .as_ref()
        .map(|s| hex::decode(s).map_err(Into::into))
        .transpose()
}
pub fn recover(root: &Path, journal_path: &Path) -> Result<bool> {
    if !journal_path.exists() {
        ensure!(
            !root.join(".tars-kb-pending").exists(),
            "recovery_required: pending marker exists but transaction journal is missing"
        );
        return Ok(false);
    }
    let j: Journal = serde_json::from_value(read_json(journal_path)?)?;
    ensure!(
        j.version == 1 && ["prepared", "committed"].contains(&j.phase.as_str()),
        "recovery_required: unsupported journal"
    );
    for e in &j.entries {
        let p = confined(root, &e.path)?;
        let bytes = current(&p)?;
        ensure!(
            bytes == decode(&e.old)? || bytes == decode(&e.new)?,
            "recovery_conflict: external edits in {}",
            e.path
        );
    }
    for e in &j.entries {
        let p = confined(root, &e.path)?;
        let b = decode(if j.phase == "committed" {
            &e.new
        } else {
            &e.old
        })?;
        apply_bytes(&p, b.as_deref())?;
    }
    if j.phase == "prepared" {
        for d in j.created_dirs.iter().rev() {
            let p = confined(root, d)?;
            if p.exists() {
                match fs::remove_dir(p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    let marker = root.join(".tars-kb-pending");
    if marker.exists() {
        fs::remove_file(marker)?;
        sync_dir(root)?;
    }
    fs::remove_file(journal_path)?;
    sync_dir(journal_path.parent().unwrap())?;
    Ok(true)
}
pub fn transact(root: &Path, journal_path: &Path, writes: &[Write]) -> Result<String> {
    transact_inner(root, journal_path, writes, None)
}
fn transact_inner(
    root: &Path,
    journal_path: &Path,
    writes: &[Write],
    fail_after: Option<usize>,
) -> Result<String> {
    ensure!(
        !journal_path.exists(),
        "recovery_required: pending transaction"
    );
    ensure!(!writes.is_empty(), "empty_transaction");
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut dirs = BTreeSet::new();
    for w in writes {
        ensure!(
            seen.insert(w.path.clone()),
            "invalid_transaction: duplicate path"
        );
        let p = confined(root, &w.path)?;
        let old = current(&p)?;
        ensure!(
            old.as_ref().map(|b| hash(b)) == w.expected,
            "conflict: revision changed for {}",
            w.path
        );
        if let Some(b) = &w.bytes {
            ensure!(
                b.len() as u64 <= MAX_FILE,
                "size_limit: write exceeds 64 MiB"
            );
        }
        let mut parent = p.parent();
        while let Some(d) = parent {
            if d == root || d.exists() {
                break;
            }
            dirs.insert(d.strip_prefix(root)?.to_string_lossy().to_string());
            parent = d.parent();
        }
        entries.push(Entry {
            path: w.path.clone(),
            old: old.as_ref().map(hex::encode),
            new: w.bytes.as_ref().map(hex::encode),
        });
    }
    let mut journal = Journal {
        version: 1,
        phase: "prepared".into(),
        entries,
        created_dirs: dirs.into_iter().collect(),
    };
    let raw = serde_json::to_vec(&journal)?;
    ensure!(
        raw.len() as u64 <= MAX_FILE,
        "size_limit: transaction journal exceeds 64 MiB"
    );
    let transaction = hash(&raw);
    atomic(journal_path, &raw)?;
    atomic(&root.join(".tars-kb-pending"), transaction.as_bytes())?;
    for (n, e) in journal.entries.iter().enumerate() {
        let p = confined(root, &e.path)?;
        let bytes = decode(&e.new)?;
        apply_bytes(&p, bytes.as_deref())?;
        if fail_after == Some(n) {
            bail!("injected_failure")
        }
        #[cfg(feature = "fault-injection")]
        if std::env::var("TARS_KB_CRASH_AFTER").ok().as_deref() == Some(&n.to_string()) {
            std::process::exit(86);
        }
    }
    journal.phase = "committed".into();
    atomic(journal_path, &serde_json::to_vec(&journal)?)?;
    #[cfg(feature = "fault-injection")]
    if std::env::var("TARS_KB_CRASH_AFTER").ok().as_deref() == Some("commit") {
        std::process::exit(86);
    }
    fs::remove_file(root.join(".tars-kb-pending"))?;
    sync_dir(root)?;
    fs::remove_file(journal_path)?;
    sync_dir(journal_path.parent().unwrap())?;
    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_nested_keys_rejected() {
        assert!(parse_json(br#"{"a":{"x":1,"x":2}}"#).is_err());
    }
    #[test]
    fn rejects_symlinks_and_traversal() {
        let t = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", t.path().join("link")).unwrap();
        assert!(confined(t.path(), "link/passwd").is_err());
        assert!(confined(t.path(), "a/../b").is_err());
    }
    #[test]
    fn failed_multifile_transaction_rolls_back() {
        for n in 0..2 {
            let t = tempfile::tempdir().unwrap();
            let root = t.path().join("base");
            private_dir(&root).unwrap();
            fs::write(root.join("a"), b"before").unwrap();
            let j = t.path().join("journal");
            let writes = vec![
                Write {
                    path: "a".into(),
                    expected: Some(hash(b"before")),
                    bytes: Some(b"after".to_vec()),
                },
                Write {
                    path: "new/b".into(),
                    expected: None,
                    bytes: Some(b"created".to_vec()),
                },
            ];
            assert!(transact_inner(&root, &j, &writes, Some(n)).is_err());
            assert!(recover(&root, &j).unwrap());
            assert_eq!(fs::read(root.join("a")).unwrap(), b"before");
            assert!(!root.join("new/b").exists());
        }
    }
    #[test]
    fn conflict_never_overwrites() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a"), b"new").unwrap();
        assert!(
            transact(
                t.path(),
                &t.path().join("journal"),
                &[Write {
                    path: "a".into(),
                    expected: Some(hash(b"old")),
                    bytes: Some(b"oops".to_vec())
                }]
            )
            .is_err()
        );
        assert_eq!(fs::read(t.path().join("a")).unwrap(), b"new");
    }
    #[test]
    fn recovery_refuses_external_changes() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a"), b"old").unwrap();
        let j = t.path().join("j");
        let w = Write {
            path: "a".into(),
            expected: Some(hash(b"old")),
            bytes: Some(b"new".to_vec()),
        };
        assert!(transact_inner(t.path(), &j, &[w], Some(0)).is_err());
        fs::write(t.path().join("a"), b"external").unwrap();
        assert!(recover(t.path(), &j).is_err());
        assert_eq!(fs::read(t.path().join("a")).unwrap(), b"external");
    }
}
