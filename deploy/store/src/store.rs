//! The design store: one file per design plus a history of what it replaced.
//!
//! ```text
//! <root>/projects/<name>               the current design (its mtime is the design's "modified")
//! <root>/history/<name>/<modified-ms>  every earlier version (kept: the newest `keep`)
//! ```
//!
//! Saving a design whose bytes changed keeps the old file in the history first; deleting a design
//! moves it there. Nothing the app does through this API destroys a version except the pruning
//! of the oldest ones beyond `keep`.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub const MAX_NAME_BYTES: usize = 200;

#[derive(Debug)]
pub enum StoreError {
    BadName,
    NotFound,
    TooLarge,
    Io(io::Error),
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound { StoreError::NotFound } else { StoreError::Io(e) }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    /// Milliseconds since 1970.
    pub modified: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Version {
    /// The version's "modified" (ms since 1970); also its id.
    pub ts: u64,
    pub size: u64,
}

#[derive(Serialize, Debug)]
pub struct PutOutcome {
    pub modified: u64,
    pub unchanged: bool,
}

pub struct Store {
    projects: PathBuf,
    history: PathBuf,
    max_bytes: usize,
    keep: usize,
    /// Writers take turns: the store is for one person, so this never contends.
    lock: Mutex<()>,
}

/// A name that is one plain file name: no path parts, no control characters, not hidden.
pub fn check_name(name: &str) -> Result<(), StoreError> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '\0'));
    if ok { Ok(()) } else { Err(StoreError::BadName) }
}

fn to_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn modified_ms(path: &Path) -> io::Result<u64> {
    Ok(to_ms(fs::metadata(path)?.modified()?))
}

fn set_mtime(path: &Path, when: SystemTime) -> io::Result<()> {
    fs::OpenOptions::new().write(true).open(path)?.set_modified(when)
}

impl Store {
    pub fn open(root: &Path, max_bytes: usize, keep: usize) -> io::Result<Store> {
        let (projects, history) = (root.join("projects"), root.join("history"));
        fs::create_dir_all(&projects)?;
        fs::create_dir_all(&history)?;
        Ok(Store { projects, history, max_bytes, keep: keep.max(1), lock: Mutex::new(()) })
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        // A panic while holding the lock leaves nothing half-written (writes go through a temp
        // file and a rename), so a poisoned lock is safe to take.
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The stored designs, by name.
    pub fn list(&self) -> Result<Vec<FileInfo>, StoreError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.projects)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                out.push(FileInfo { name, size: meta.len(), modified: meta.modified().map(to_ms).unwrap_or(0) });
            }
        }
        out.sort_by_key(|f| f.name.to_lowercase());
        Ok(out)
    }

    /// Every name with a current design or a history (deleted designs have only a history).
    pub fn all_names(&self) -> Result<Vec<String>, StoreError> {
        let mut names: Vec<String> = self.list()?.into_iter().map(|f| f.name).collect();
        if let Ok(dir) = fs::read_dir(&self.history) {
            for entry in dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && check_name(&name).is_ok() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, StoreError> {
        check_name(name)?;
        Ok(fs::read(self.projects.join(name))?)
    }

    pub fn read_version(&self, name: &str, ts: u64) -> Result<Vec<u8>, StoreError> {
        check_name(name)?;
        Ok(fs::read(self.history.join(name).join(ts.to_string()))?)
    }

    pub fn versions(&self, name: &str) -> Result<Vec<Version>, StoreError> {
        check_name(name)?;
        let mut out = Vec::new();
        let Ok(dir) = fs::read_dir(self.history.join(name)) else { return Ok(out) };
        for entry in dir.flatten() {
            let Ok(ts) = entry.file_name().to_string_lossy().parse::<u64>() else { continue };
            if let Ok(meta) = entry.metadata() {
                out.push(Version { ts, size: meta.len() });
            }
        }
        out.sort_by_key(|v| std::cmp::Reverse(v.ts));
        Ok(out)
    }

    /// Store `bytes` as the design `name`, keeping what it replaces. `modified` (ms since 1970) is
    /// the design's save time on the client; the server's clock is used when it is missing.
    pub fn put(&self, name: &str, bytes: &[u8], modified: Option<u64>) -> Result<PutOutcome, StoreError> {
        check_name(name)?;
        if bytes.len() > self.max_bytes {
            return Err(StoreError::TooLarge);
        }
        let now = SystemTime::now();
        // A client clock far ahead would make its design win every comparison from then on.
        let latest = now + Duration::from_secs(24 * 3600);
        let when = modified
            .filter(|m| *m > 0)
            .and_then(|m| UNIX_EPOCH.checked_add(Duration::from_millis(m)))
            .unwrap_or(now)
            .min(latest);

        let _g = self.guard();
        let cur = self.projects.join(name);
        if fs::metadata(&cur).is_ok_and(|m| m.len() == bytes.len() as u64) && fs::read(&cur).is_ok_and(|old| old == bytes) {
            set_mtime(&cur, when)?;
            return Ok(PutOutcome { modified: to_ms(when), unchanged: true });
        }
        let nanos = now.duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let tmp = self.projects.join(format!(".tmp-{}-{nanos}", std::process::id()));
        let result = (|| -> Result<(), StoreError> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.set_modified(when)?;
            f.sync_all()?;
            drop(f);
            if cur.exists() {
                self.keep_version(name, &cur, false)?;
            }
            fs::rename(&tmp, &cur)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        self.prune(name);
        Ok(PutOutcome { modified: to_ms(when), unchanged: false })
    }

    /// Delete the current design; its versions stay in the history.
    pub fn delete(&self, name: &str) -> Result<(), StoreError> {
        check_name(name)?;
        let _g = self.guard();
        let cur = self.projects.join(name);
        if !cur.is_file() {
            return Err(StoreError::NotFound);
        }
        self.keep_version(name, &cur, true)?;
        self.prune(name);
        Ok(())
    }

    /// Put `cur` in the history under its modified time (a free id: two versions never share one).
    /// `consume`: the current file goes (a delete); otherwise it is linked or copied (a replace).
    fn keep_version(&self, name: &str, cur: &Path, consume: bool) -> Result<(), StoreError> {
        let dir = self.history.join(name);
        fs::create_dir_all(&dir)?;
        let mut ts = modified_ms(cur)?;
        let mut dest = dir.join(ts.to_string());
        for _ in 0..1000 {
            if !dest.exists() {
                break;
            }
            ts += 1;
            dest = dir.join(ts.to_string());
        }
        if dest.exists() {
            return Err(StoreError::Io(io::Error::other("no free version id")));
        }
        if consume {
            if fs::rename(cur, &dest).is_err() {
                fs::copy(cur, &dest)?;
                fs::remove_file(cur)?;
            }
        } else if fs::hard_link(cur, &dest).is_err() {
            fs::copy(cur, &dest)?;
        }
        Ok(())
    }

    /// Drop the oldest versions beyond `keep`. Best effort: a failure here never fails a save.
    fn prune(&self, name: &str) {
        let Ok(versions) = self.versions(name) else { return };
        for v in versions.iter().skip(self.keep) {
            let _ = fs::remove_file(self.history.join(name).join(v.ts.to_string()));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn store(keep: usize) -> (Store, PathBuf) {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("sc-store-test-{n}-{keep}"));
        (Store::open(&root, 1 << 20, keep).unwrap(), root)
    }

    #[test]
    fn names_are_one_plain_file_name() {
        let long = "x".repeat(201);
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "..\\x", "c:evil", "tab\there", long.as_str()] {
            assert!(check_name(bad).is_err(), "{bad:?} must be refused");
        }
        for good in ["bracket.solidcraft", "My part (2).solidcraft", "Teil Gr\u{f6}\u{df}e.solidcraft"] {
            assert!(check_name(good).is_ok(), "{good:?} must be accepted");
        }
    }

    #[test]
    fn a_save_keeps_the_version_it_replaces() {
        let (s, root) = store(10);
        let first = s.put("a.solidcraft", b"one", Some(1_000)).unwrap();
        assert!(!first.unchanged && first.modified == 1_000);
        assert!(s.versions("a.solidcraft").unwrap().is_empty());
        s.put("a.solidcraft", b"two", Some(2_000)).unwrap();
        assert_eq!(s.read("a.solidcraft").unwrap(), b"two");
        let v = s.versions("a.solidcraft").unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].ts, 1_000);
        assert_eq!(s.read_version("a.solidcraft", 1_000).unwrap(), b"one");
        assert_eq!(s.list().unwrap()[0].modified, 2_000);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn saving_the_same_bytes_adds_no_version() {
        let (s, root) = store(10);
        s.put("a.solidcraft", b"same", Some(1_000)).unwrap();
        let again = s.put("a.solidcraft", b"same", Some(5_000)).unwrap();
        assert!(again.unchanged);
        assert!(s.versions("a.solidcraft").unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_moves_the_design_to_the_history() {
        let (s, root) = store(10);
        s.put("a.solidcraft", b"one", Some(1_000)).unwrap();
        s.delete("a.solidcraft").unwrap();
        assert!(matches!(s.read("a.solidcraft"), Err(StoreError::NotFound)));
        assert!(matches!(s.delete("a.solidcraft"), Err(StoreError::NotFound)));
        assert_eq!(s.read_version("a.solidcraft", 1_000).unwrap(), b"one");
        assert_eq!(s.all_names().unwrap(), vec!["a.solidcraft".to_string()]);
        assert!(s.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_newest_versions_are_kept() {
        let (s, root) = store(3);
        for i in 1..=8u64 {
            s.put("a.solidcraft", format!("v{i}").as_bytes(), Some(i * 1_000)).unwrap();
        }
        let ts: Vec<u64> = s.versions("a.solidcraft").unwrap().iter().map(|v| v.ts).collect();
        assert_eq!(ts, vec![7_000, 6_000, 5_000]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_and_bad_names_are_refused_and_a_far_future_clock_is_clamped() {
        let (s, root) = store(3);
        assert!(matches!(s.put("a", &vec![0u8; (1 << 20) + 1], None), Err(StoreError::TooLarge)));
        assert!(matches!(s.put("../x", b"x", None), Err(StoreError::BadName)));
        let out = s.put("a", b"x", Some(u64::MAX / 4)).unwrap();
        assert!(out.modified < to_ms(SystemTime::now()) + 25 * 3600 * 1000);
        let _ = fs::remove_dir_all(root);
    }
}
