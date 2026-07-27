use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use crate::audit::{AuditChain, AuditEntry, AuditSink};

pub struct FileSink {
    writer: Mutex<BufWriter<File>>,
    chain: Mutex<AuditChain>,
    max_size: u64,
    max_files: u32,
    current_size: Mutex<u64>,
    base_path: std::path::PathBuf,
    file_count: Mutex<u32>,
}

impl FileSink {
    pub fn new(
        path: impl AsRef<std::path::Path>,
        max_size: u64,
        max_files: u32,
        config_hash: String,
    ) -> Result<Self, String> {
        let base_path = path.as_ref().to_path_buf();
        let file = File::create(&base_path)
            .map_err(|e| format!("Failed to create audit log: {}", e))?;
        
        let chain = AuditChain::new("default".into());
        
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            chain: Mutex::new(AuditChain::new("default".into())),
            max_size,
            max_files,
            current_size: Mutex::new(0),
            base_path,
            file_count: Mutex::new(1),
        })
    }

    pub fn chain(&self) -> &std::sync::Mutex<AuditChain> {
        &self.chain
    }
}

impl AuditSink for FileSink {
    fn write(&self, entry: &AuditEntry) -> Result<(), String> {
        // Write to chain
        self.chain.lock().unwrap().append(entry.clone())
            .map_err(|e| format!("Chain append failed: {}", e))?;

        // Write to file
        let mut writer = self.writer.lock().unwrap();
        let entry_json = serde_json::to_string(entry)
            .map_err(|e| format!("Serialization failed: {}", e))?;
        
        writer.write_all(entry_json.as_bytes())
            .map_err(|e| format!("Write failed: {}", e))?;
        writer.write_all(b"\n")
            .map_err(|e| format!("Newline write failed: {}", e))?;
        writer.flush()
            .map_err(|e| format!("Flush failed: {}", e))?;

        // Check rotation
        let current_size = *self.current_size.lock().unwrap() + entry_json.len() as u64 + 1;
        *self.current_size.lock().unwrap() = current_size;

        if current_size > self.max_size {
            self.rotate()?;
        }

        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        self.writer.lock().unwrap().flush()
            .map_err(|e| format!("Flush failed: {}", e))
    }

    fn rotate(&self) -> Result<(), String> {
        let file_count = *self.file_count.lock().unwrap();
        if file_count >= self.max_files {
            return Err("Max files reached".into());
        }

        let new_file_count = file_count + 1;
        let new_path = self.base_path.with_extension(format!("log.{}", new_file_count));
        
        let new_file = File::create(&new_path)
            .map_err(|e| format!("Failed to create rotated file: {}", e))?;
        
        *self.writer.lock().unwrap() = BufWriter::new(new_file);
        *self.current_size.lock().unwrap() = 0;
        *self.file_count.lock().unwrap() = new_file_count;

        Ok(())
    }
}