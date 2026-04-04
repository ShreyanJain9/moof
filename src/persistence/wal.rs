/// Write-ahead log for crash recovery.
///
/// Every heap mutation between snapshots is appended here.
/// On startup: load snapshot, replay WAL, resume.
/// On clean exit: save snapshot, clear WAL.

use std::path::Path;
use std::fs::{self, File, OpenOptions};
use std::io::{Write, BufWriter, BufReader, Read};
use serde::{Serialize, Deserialize};

use crate::runtime::value::{Value, HeapObject};

/// A single WAL entry — one mutation to the heap.
#[derive(Serialize, Deserialize, Debug)]
pub enum WalEntry {
    /// A new object was allocated at this id.
    Alloc { id: u32, object: HeapObject },
    /// An existing object was replaced.
    Replace { id: u32, object: HeapObject },
    /// A new symbol was interned.
    InternSymbol { id: u32, name: String },
}

/// Appends WAL entries to disk.
pub struct WalWriter {
    writer: BufWriter<File>,
}

impl WalWriter {
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {}", dir.display(), e))?;
        let path = dir.join("wal.bin");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Cannot open WAL: {}", e))?;
        Ok(WalWriter {
            writer: BufWriter::new(file),
        })
    }

    pub fn append(&mut self, entry: &WalEntry) -> Result<(), String> {
        let data = bincode::serialize(entry)
            .map_err(|e| format!("WAL serialize error: {}", e))?;
        let len = data.len() as u32;
        self.writer.write_all(&len.to_le_bytes())
            .map_err(|e| format!("WAL write error: {}", e))?;
        self.writer.write_all(&data)
            .map_err(|e| format!("WAL write error: {}", e))?;
        self.writer.flush()
            .map_err(|e| format!("WAL flush error: {}", e))?;
        Ok(())
    }
}

/// Read WAL entries from disk for replay.
pub fn replay_wal(dir: &Path) -> Result<Vec<WalEntry>, String> {
    let path = dir.join("wal.bin");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let data = fs::read(&path)
        .map_err(|e| format!("Cannot read WAL: {}", e))?;

    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
        pos += 4;
        if pos + len > data.len() {
            // Truncated entry — WAL was interrupted mid-write. Stop here.
            break;
        }
        match bincode::deserialize(&data[pos..pos+len]) {
            Ok(entry) => entries.push(entry),
            Err(_) => break, // Corrupted entry, stop replay
        }
        pos += len;
    }

    Ok(entries)
}
