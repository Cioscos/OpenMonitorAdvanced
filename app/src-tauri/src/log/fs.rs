//! Filesystem seam of the log writer, so its tests run on an in-memory fake.

use std::io;
use std::path::Path;

/// The few filesystem operations the log writer needs.
pub trait LogFs: Send + Sync + 'static {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;
    /// Creates `path` exclusively: fails with `AlreadyExists` when it is
    /// there, never truncates.
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn LogFile>>;
}

/// An open log file part.
pub trait LogFile: Send {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

/// The real filesystem.
pub struct RealFs;

impl LogFs for RealFs {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn create_new(&self, path: &Path) -> io::Result<Box<dyn LogFile>> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(Box::new(RealFile(file)))
    }
}

/// Unbuffered: the writer keeps its own buffer, so every `write_all` hands
/// the bytes to the operating system (they survive a crash of the app).
struct RealFile(std::fs::File);

impl LogFile for RealFile {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.0, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.0)
    }
}

/// In-memory filesystem with faults injectable per call, for the writer and
/// session tests.
#[cfg(test)]
pub mod fake {
    use std::collections::{BTreeMap, HashMap};
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use super::{LogFile, LogFs};

    /// What the n-th `write_all` call (counted across all files, from 1) does.
    #[derive(Debug, Clone, Copy)]
    pub enum Fault {
        /// Fails with this raw OS error, writing nothing.
        Error(i32),
        /// Writes the first half of the bytes, then fails with this error.
        Partial(i32),
    }

    #[derive(Default)]
    struct State {
        dirs: Vec<PathBuf>,
        files: BTreeMap<PathBuf, FileData>,
        writes: u32,
        flushes: u32,
        write_faults: HashMap<u32, Fault>,
        flush_faults: HashMap<u32, i32>,
    }

    #[derive(Default)]
    struct FileData {
        data: Vec<u8>,
        /// Length of `data` at the last flush.
        flushed: usize,
        flushes: u32,
    }

    #[derive(Default)]
    pub struct MemFs {
        state: Arc<Mutex<State>>,
    }

    fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
        state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl MemFs {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// A file that already exists (counts as flushed).
        pub fn insert(&self, path: impl Into<PathBuf>, bytes: &[u8]) {
            let data = FileData {
                data: bytes.to_vec(),
                flushed: bytes.len(),
                flushes: 0,
            };
            lock(&self.state).files.insert(path.into(), data);
        }

        /// The `call`-th `write_all` (from 1, across all files) fails.
        pub fn fail_write(&self, call: u32, fault: Fault) {
            lock(&self.state).write_faults.insert(call, fault);
        }

        /// The `call`-th `flush` (from 1, across all files) fails with `code`.
        pub fn fail_flush(&self, call: u32, code: i32) {
            lock(&self.state).flush_faults.insert(call, code);
        }

        /// Everything written to `path`.
        pub fn content(&self, path: &Path) -> Option<Vec<u8>> {
            lock(&self.state).files.get(path).map(|f| f.data.clone())
        }

        /// What `path` held at its last flush.
        pub fn flushed(&self, path: &Path) -> Option<Vec<u8>> {
            lock(&self.state)
                .files
                .get(path)
                .map(|f| f.data[..f.flushed].to_vec())
        }

        /// How many times `path` was flushed.
        pub fn flush_count(&self, path: &Path) -> u32 {
            lock(&self.state).files.get(path).map_or(0, |f| f.flushes)
        }

        /// All file paths, sorted.
        pub fn files(&self) -> Vec<PathBuf> {
            lock(&self.state).files.keys().cloned().collect()
        }

        /// Folders passed to `create_dir_all`, in call order.
        pub fn dirs(&self) -> Vec<PathBuf> {
            lock(&self.state).dirs.clone()
        }
    }

    impl LogFs for MemFs {
        fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
            lock(&self.state).dirs.push(dir.to_path_buf());
            Ok(())
        }

        fn create_new(&self, path: &Path) -> io::Result<Box<dyn LogFile>> {
            let mut state = lock(&self.state);
            if state.files.contains_key(path) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            state.files.insert(path.to_path_buf(), FileData::default());
            Ok(Box::new(MemFile {
                state: self.state.clone(),
                path: path.to_path_buf(),
            }))
        }
    }

    struct MemFile {
        state: Arc<Mutex<State>>,
        path: PathBuf,
    }

    impl LogFile for MemFile {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            let mut state = lock(&self.state);
            state.writes += 1;
            let fault = state.write_faults.get(&state.writes).copied();
            let file = state.files.entry(self.path.clone()).or_default();
            match fault {
                None => {
                    file.data.extend_from_slice(bytes);
                    Ok(())
                }
                Some(Fault::Error(code)) => Err(io::Error::from_raw_os_error(code)),
                Some(Fault::Partial(code)) => {
                    file.data.extend_from_slice(&bytes[..bytes.len() / 2]);
                    Err(io::Error::from_raw_os_error(code))
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut state = lock(&self.state);
            state.flushes += 1;
            if let Some(code) = state.flush_faults.get(&state.flushes).copied() {
                return Err(io::Error::from_raw_os_error(code));
            }
            let file = state.files.entry(self.path.clone()).or_default();
            file.flushed = file.data.len();
            file.flushes += 1;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_fs_creates_new_files_exclusively() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oma-logfs-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let nested = dir.join("a").join("b");
        let path = nested.join("oma.csv");
        let fs = RealFs;

        fs.create_dir_all(&nested).unwrap();
        let mut file = fs.create_new(&path).unwrap();
        file.write_all(b"one,").unwrap();
        file.write_all(b"two").unwrap();
        file.flush().unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"one,two");

        // Never truncates an existing file.
        let err = fs.create_new(&path).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"one,two");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
