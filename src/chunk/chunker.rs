//! File chunking with content-based deduplication
//!
//! Uses BLAKE3 for content hashing to enable deduplication.
//! Chunks are identified by their content hash, allowing identical
//! data to be stored only once.

use crate::config::ChunkConfig;
use crate::error::{Error, Result};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Content-based chunk identifier (BLAKE3 hash)
pub type ChunkId = String;

/// Information about a chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    /// Content-based ID
    pub id: ChunkId,
    /// Size in bytes
    pub size: usize,
    /// Offset in original file
    pub offset: u64,
    /// Hash of original (pre-encryption) content
    pub content_hash: String,
}

/// A chunk of data ready for encryption
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Chunk information
    pub info: ChunkInfo,
    /// Raw data (not yet encrypted)
    pub data: Vec<u8>,
}

impl Chunk {
    /// Create a new chunk from data
    pub fn new(data: Vec<u8>, offset: u64) -> Self {
        let content_hash = blake3::hash(&data).to_hex().to_string();
        let id = content_hash.clone(); // ID is based on content hash

        Chunk {
            info: ChunkInfo {
                id,
                size: data.len(),
                offset,
                content_hash,
            },
            data,
        }
    }

    /// Get the chunk ID
    pub fn id(&self) -> &str {
        &self.info.id
    }
}

/// Chunker for splitting files into fixed-size chunks
pub struct Chunker {
    chunk_size: usize,
}

impl Chunker {
    /// Create a new chunker with the given configuration
    pub fn new(config: &ChunkConfig) -> Self {
        Chunker {
            chunk_size: config.chunk_size,
        }
    }

    /// Create a chunker with a specific chunk size
    pub fn with_size(chunk_size: usize) -> Self {
        Chunker { chunk_size }
    }

    /// Get the configured chunk size
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Split data into chunks
    pub fn chunk_data(&self, data: &[u8]) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        for chunk_data in data.chunks(self.chunk_size) {
            chunks.push(Chunk::new(chunk_data.to_vec(), offset));
            offset += chunk_data.len() as u64;
        }

        chunks
    }

    /// Split a reader into chunks
    pub fn chunk_reader<R: Read>(&self, mut reader: R) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut buffer = vec![0u8; self.chunk_size];

        loop {
            let mut total_read = 0;

            // Read until buffer is full or EOF
            while total_read < self.chunk_size {
                match reader.read(&mut buffer[total_read..]) {
                    Ok(0) => break, // EOF
                    Ok(n) => total_read += n,
                    Err(e) => return Err(Error::Io(e)),
                }
            }

            if total_read == 0 {
                break; // No more data
            }

            // Truncate buffer to actual data read
            let chunk_data = buffer[..total_read].to_vec();
            chunks.push(Chunk::new(chunk_data, offset));
            offset += total_read as u64;
        }

        Ok(chunks)
    }

    /// Reassemble chunks into complete data
    pub fn reassemble(&self, chunks: &[Chunk]) -> Vec<u8> {
        let total_size: usize = chunks.iter().map(|c| c.data.len()).sum();
        let mut result = Vec::with_capacity(total_size);

        // Sort by offset and concatenate
        let mut sorted: Vec<_> = chunks.iter().collect();
        sorted.sort_by_key(|c| c.info.offset);

        for chunk in sorted {
            result.extend_from_slice(&chunk.data);
        }

        result
    }

    /// Reassemble chunks into a writer
    pub fn reassemble_to_writer<W: Write>(&self, chunks: &[Chunk], mut writer: W) -> Result<u64> {
        let mut sorted: Vec<_> = chunks.iter().collect();
        sorted.sort_by_key(|c| c.info.offset);

        let mut written = 0u64;
        for chunk in sorted {
            writer.write_all(&chunk.data)?;
            written += chunk.data.len() as u64;
        }

        Ok(written)
    }

    /// Calculate the hash of complete file data
    pub fn file_hash(&self, data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }

    /// Calculate the hash of a reader's contents
    pub fn file_hash_reader<R: Read>(&self, mut reader: R) -> Result<String> {
        let mut hasher = Hasher::new();
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => { hasher.update(&buffer[..n]); },
                Err(e) => return Err(Error::Io(e)),
            }
        }

        Ok(hasher.finalize().to_hex().to_string())
    }
}

/// Deduplication tracker
pub struct DedupTracker {
    /// Known chunk IDs
    known_chunks: std::collections::HashSet<ChunkId>,
}

impl DedupTracker {
    /// Create a new dedup tracker
    pub fn new() -> Self {
        DedupTracker {
            known_chunks: std::collections::HashSet::new(),
        }
    }

    /// Check if a chunk is already known
    pub fn is_known(&self, chunk_id: &str) -> bool {
        self.known_chunks.contains(chunk_id)
    }

    /// Register a chunk as known
    pub fn register(&mut self, chunk_id: ChunkId) {
        self.known_chunks.insert(chunk_id);
    }

    /// Get the number of known chunks
    pub fn len(&self) -> usize {
        self.known_chunks.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.known_chunks.is_empty()
    }

    /// Filter chunks, returning only new ones
    pub fn filter_new(&mut self, chunks: Vec<Chunk>) -> (Vec<Chunk>, Vec<ChunkId>) {
        let mut new_chunks = Vec::new();
        let mut existing_ids = Vec::new();

        for chunk in chunks {
            if self.is_known(&chunk.info.id) {
                existing_ids.push(chunk.info.id.clone());
            } else {
                self.register(chunk.info.id.clone());
                new_chunks.push(chunk);
            }
        }

        (new_chunks, existing_ids)
    }
}

impl Default for DedupTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn test_chunker() -> Chunker {
        Chunker::with_size(1024) // 1KB chunks for testing
    }

    #[test]
    fn test_chunk_creation() {
        let data = b"Hello, World!";
        let chunk = Chunk::new(data.to_vec(), 0);

        assert_eq!(chunk.data, data);
        assert_eq!(chunk.info.size, data.len());
        assert_eq!(chunk.info.offset, 0);
        assert!(!chunk.info.content_hash.is_empty());
    }

    #[test]
    fn test_chunk_id_deterministic() {
        let data = b"Same content";
        let chunk1 = Chunk::new(data.to_vec(), 0);
        let chunk2 = Chunk::new(data.to_vec(), 100); // Different offset

        // Same content = same hash
        assert_eq!(chunk1.info.content_hash, chunk2.info.content_hash);
    }

    #[test]
    fn test_chunker_small_data() {
        let chunker = test_chunker();
        let data = b"Small data";

        let chunks = chunker.chunk_data(data);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].data, data);
    }

    #[test]
    fn test_chunker_large_data() {
        let chunker = test_chunker();
        let data = vec![0x42u8; 3000]; // 3KB = 3 chunks

        let chunks = chunker.chunk_data(&data);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].data.len(), 1024);
        assert_eq!(chunks[1].data.len(), 1024);
        assert_eq!(chunks[2].data.len(), 952);
    }

    #[test]
    fn test_chunker_reader() {
        let chunker = test_chunker();
        let data = vec![0x42u8; 2500];
        let cursor = Cursor::new(&data);

        let chunks = chunker.chunk_reader(cursor).unwrap();

        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn test_reassemble() {
        let chunker = test_chunker();
        let original = vec![0x42u8; 3000];

        let chunks = chunker.chunk_data(&original);
        let reassembled = chunker.reassemble(&chunks);

        assert_eq!(reassembled, original);
    }

    #[test]
    fn test_reassemble_unordered() {
        let chunker = test_chunker();
        let original = vec![0x42u8; 3000];

        let mut chunks = chunker.chunk_data(&original);
        chunks.reverse(); // Reverse order

        let reassembled = chunker.reassemble(&chunks);
        assert_eq!(reassembled, original);
    }

    #[test]
    fn test_dedup_tracker() {
        let mut tracker = DedupTracker::new();

        let chunk1 = Chunk::new(b"data1".to_vec(), 0);
        let chunk2 = Chunk::new(b"data2".to_vec(), 0);
        let chunk3 = Chunk::new(b"data1".to_vec(), 100); // Same content as chunk1

        assert!(!tracker.is_known(&chunk1.info.id));

        tracker.register(chunk1.info.id.clone());
        assert!(tracker.is_known(&chunk1.info.id));
        assert!(!tracker.is_known(&chunk2.info.id));
        assert!(tracker.is_known(&chunk3.info.id)); // Same hash as chunk1
    }

    #[test]
    fn test_filter_new() {
        let mut tracker = DedupTracker::new();

        let existing = Chunk::new(b"existing".to_vec(), 0);
        tracker.register(existing.info.id.clone());

        let new_chunk = Chunk::new(b"new".to_vec(), 0);
        let dup_chunk = Chunk::new(b"existing".to_vec(), 100);

        let chunks = vec![new_chunk.clone(), dup_chunk];
        let (new_ones, existing_ids) = tracker.filter_new(chunks);

        assert_eq!(new_ones.len(), 1);
        assert_eq!(existing_ids.len(), 1);
        assert_eq!(new_ones[0].info.id, new_chunk.info.id);
    }

    #[test]
    fn test_file_hash() {
        let chunker = test_chunker();
        let data = b"Test data for hashing";

        let hash1 = chunker.file_hash(data);
        let hash2 = chunker.file_hash(data);

        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }
}
