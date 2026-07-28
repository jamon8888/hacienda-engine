//! Durable destinations for audit entries.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::audit::chain::AuditChain;
use crate::audit::entry::AuditEntry;
use crate::audit::error::AuditError;

pub trait AuditSink {
    /// Persist a single entry.
    fn write(&mut self, entry: &AuditEntry) -> Result<(), AuditError>;
    /// Flush any buffered entries to the underlying destination.
    fn flush(&mut self) -> Result<(), AuditError>;
    /// Start a new segment, retiring the current one.
    fn rotate(&mut self) -> Result<(), AuditError>;
}

/// Appends JSON-lines to a file, rotating once the file exceeds `max_size` bytes.
///
/// Rotation keeps at most `max_files` segments: `path`, `path.1`, ... `path.{max_files-1}`.
pub struct FileSink {
    writer: BufWriter<File>,
    path: PathBuf,
    current_size: u64,
    max_size: u64,
    max_files: u32,
    chain: AuditChain,
}

impl FileSink {
    /// Open (or create) the log at `path` and resume its chain under `config_hash`.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Io`] if the file cannot be opened or its size read.
    pub fn new(
        path: impl AsRef<Path>,
        max_size: u64,
        max_files: u32,
        config_hash: impl Into<String>,
    ) -> Result<Self, AuditError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| AuditError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let current_size = file
            .metadata()
            .map_err(|source| AuditError::Io {
                path: path.display().to_string(),
                source,
            })?
            .len();

        Ok(Self {
            writer: BufWriter::new(file),
            path,
            current_size,
            max_size,
            max_files,
            chain: AuditChain::new(config_hash),
        })
    }

    pub fn chain(&self) -> &AuditChain {
        &self.chain
    }

    pub fn chain_mut(&mut self) -> &mut AuditChain {
        &mut self.chain
    }

    fn io_err(&self, source: std::io::Error) -> AuditError {
        AuditError::Io {
            path: self.path.display().to_string(),
            source,
        }
    }
}

impl AuditSink for FileSink {
    fn write(&mut self, entry: &AuditEntry) -> Result<(), AuditError> {
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        let written = line.len() as u64;
        self.writer.write_all(&line).map_err(|e| AuditError::Io {
            path: self.path.display().to_string(),
            source: e,
        })?;
        self.current_size += written;
        if self.current_size > self.max_size {
            self.rotate()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), AuditError> {
        self.writer.flush().map_err(|e| AuditError::Io {
            path: self.path.display().to_string(),
            source: e,
        })
    }

    fn rotate(&mut self) -> Result<(), AuditError> {
        self.flush()?;

        // Shift existing segments up: path.{n} -> path.{n+1}, dropping the oldest.
        for i in (1..self.max_files).rev() {
            let from = PathBuf::from(format!("{}.{}", self.path.display(), i));
            if !from.exists() {
                continue;
            }
            if i + 1 >= self.max_files {
                fs::remove_file(&from).map_err(|e| self.io_err(e))?;
            } else {
                let to = PathBuf::from(format!("{}.{}", self.path.display(), i + 1));
                fs::rename(&from, &to).map_err(|e| self.io_err(e))?;
            }
        }

        if self.path.exists() {
            let rotated = PathBuf::from(format!("{}.1", self.path.display()));
            fs::rename(&self.path, &rotated).map_err(|e| self.io_err(e))?;
        }

        let file = File::create(&self.path).map_err(|e| self.io_err(e))?;
        self.writer = BufWriter::new(file);
        self.current_size = 0;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{AuditEntryInput, EntitySource, RedactionAction};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hacienda-audit-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(chain: &mut AuditChain, id: &str) -> AuditEntry {
        chain
            .push(AuditEntryInput {
                id: id.into(),
                category: "Email".into(),
                action: RedactionAction::Mask,
                span_hash: "abc".into(),
                span_length: 10,
                confidence: Some(1.0),
                source: EntitySource::Regex,
                pipeline_version: "1.0".into(),
                config_hash: String::new(),
            })
            .clone()
    }

    #[test]
    fn should_append_one_json_line_per_entry() {
        let dir = TempDir::new("append");
        let path = dir.join("audit.log");
        let mut sink = FileSink::new(&path, 1_000_000, 3, "cfg").unwrap();
        let mut chain = AuditChain::new("cfg");

        for i in 0..3 {
            let e = entry(&mut chain, &format!("id-{i}"));
            sink.write(&e).unwrap();
        }
        sink.flush().unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 3);
    }

    #[test]
    fn should_rotate_once_the_file_exceeds_max_size() {
        let dir = TempDir::new("rotate");
        let path = dir.join("audit.log");
        // max_size of 1 byte forces a rotation after every write.
        let mut sink = FileSink::new(&path, 1, 3, "cfg").unwrap();
        let mut chain = AuditChain::new("cfg");

        let e = entry(&mut chain, "id-0");
        sink.write(&e).unwrap();
        sink.flush().unwrap();

        assert!(dir.join("audit.log.1").exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn should_keep_at_most_max_files_segments() {
        let dir = TempDir::new("retain");
        let path = dir.join("audit.log");
        let mut sink = FileSink::new(&path, 1, 3, "cfg").unwrap();
        let mut chain = AuditChain::new("cfg");

        for i in 0..5 {
            let e = entry(&mut chain, &format!("id-{i}"));
            sink.write(&e).unwrap();
        }
        sink.flush().unwrap();

        assert!(dir.join("audit.log.1").exists());
        assert!(dir.join("audit.log.2").exists());
        assert!(!dir.join("audit.log.3").exists());
    }

    #[test]
    fn should_resume_appending_to_an_existing_log() {
        let dir = TempDir::new("resume");
        let path = dir.join("audit.log");
        let mut chain = AuditChain::new("cfg");

        let mut sink = FileSink::new(&path, 1_000_000, 3, "cfg").unwrap();
        let first = entry(&mut chain, "id-0");
        sink.write(&first).unwrap();
        sink.flush().unwrap();
        drop(sink);

        let mut sink = FileSink::new(&path, 1_000_000, 3, "cfg").unwrap();
        let second = entry(&mut chain, "id-1");
        sink.write(&second).unwrap();
        sink.flush().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
    }
}
