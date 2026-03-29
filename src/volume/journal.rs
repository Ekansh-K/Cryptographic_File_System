//! 10E — Journal subsystem for crash-safe metadata operations.
//!
//! Implements a circular **undo log** (write-ahead journal) for metadata-only
//! journaling. Before modifying a metadata block, the old contents are saved
//! as an undo record. On crash, incomplete transactions are rolled back by
//! restoring those old contents.
//!
//! Journal lifecycle:
//!   format  → `Journal::init()` writes the journal superblock + zeros entries
//!   mount   → `Journal::load()` reads jsb, replays if NEEDS_RECOVERY
//!   runtime → `begin_txn()` / `journal_block()` / `commit_txn()` or `abort_txn()`
//!   sync    → `checkpoint()` reclaims committed space, clears NEEDS_RECOVERY
//!   unmount → `mark_clean()` sets JFLAG_CLEAN

use anyhow::{bail, Result};
use std::collections::HashMap;

use crate::block_device::CFSBlockDevice;

// ---------------------------------------------------------------------------
// Constants & flags
// ---------------------------------------------------------------------------

pub const JOURNAL_MAGIC: [u8; 4] = *b"CJNL";
pub const JOURNAL_VERSION: u32 = 1;

/// Journal needs recovery — set on first write, cleared after clean unmount.
pub const JFLAG_NEEDS_RECOVERY: u32 = 0x01;
/// Journal is clean — set after successful unmount or replay.
pub const JFLAG_CLEAN: u32 = 0x02;

/// Fixed header size per journal entry (bytes before undo data).
pub const ENTRY_HEADER_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// JournalSuperblock
// ---------------------------------------------------------------------------

/// On-disk journal superblock (stored in the first block of the journal region).
#[derive(Debug, Clone)]
pub struct JournalSuperblock {
    pub magic: [u8; 4],
    pub version: u32,
    /// Current transaction sequence number (monotonically increasing).
    pub sequence: u64,
    /// Next write position (entry index, 0-based relative to entry area).
    pub head: u64,
    /// Oldest unfinished/uncheckpointed entry (entry index).
    pub tail: u64,
    /// Total entry slots (journal_blocks - 1).
    pub capacity: u64,
    /// JFLAG_NEEDS_RECOVERY | JFLAG_CLEAN.
    pub flags: u32,
    pub checksum: u32,
}

impl JournalSuperblock {
    pub fn new(capacity: u64) -> Self {
        Self {
            magic: JOURNAL_MAGIC,
            version: JOURNAL_VERSION,
            sequence: 1,
            head: 0,
            tail: 0,
            capacity,
            flags: JFLAG_CLEAN,
            checksum: 0,
        }
    }

    pub fn serialize(&self, block_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; block_size as usize];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..8].copy_from_slice(&self.version.to_le_bytes());
        buf[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        buf[16..24].copy_from_slice(&self.head.to_le_bytes());
        buf[24..32].copy_from_slice(&self.tail.to_le_bytes());
        buf[32..40].copy_from_slice(&self.capacity.to_le_bytes());
        buf[40..44].copy_from_slice(&self.flags.to_le_bytes());
        let checksum = crc32fast::hash(&buf[0..44]);
        buf[44..48].copy_from_slice(&checksum.to_le_bytes());
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < 48 {
            bail!("journal superblock too short");
        }
        let magic: [u8; 4] = buf[0..4].try_into().unwrap();
        if magic != JOURNAL_MAGIC {
            bail!("bad journal magic: expected {:?}, got {:?}", JOURNAL_MAGIC, magic);
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        if version != JOURNAL_VERSION {
            bail!("unsupported journal version: {}", version);
        }
        let sequence = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let head = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let tail = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let capacity = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[40..44].try_into().unwrap());
        let stored_checksum = u32::from_le_bytes(buf[44..48].try_into().unwrap());

        let computed = crc32fast::hash(&buf[0..44]);
        if stored_checksum != computed {
            bail!(
                "journal superblock checksum mismatch: stored=0x{:08x}, computed=0x{:08x}",
                stored_checksum, computed
            );
        }

        Ok(Self { magic, version, sequence, head, tail, capacity, flags, checksum: stored_checksum })
    }

    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn needs_recovery(&self) -> bool {
        self.flags & JFLAG_NEEDS_RECOVERY != 0
    }
}

// ---------------------------------------------------------------------------
// EntryType
// ---------------------------------------------------------------------------

/// Journal entry types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EntryType {
    TxnBegin = 0,
    BlockData = 1,
    TxnCommit = 2,
    TxnAbort = 3,
}

impl EntryType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::TxnBegin),
            1 => Ok(Self::BlockData),
            2 => Ok(Self::TxnCommit),
            3 => Ok(Self::TxnAbort),
            _ => bail!("unknown journal entry type: {}", v),
        }
    }
}

// ---------------------------------------------------------------------------
// JournalEntry
// ---------------------------------------------------------------------------

/// On-disk journal entry header (one per block).
/// For `BlockData` entries, a second block immediately follows containing the
/// raw undo data (the full original block contents). All other entry types
/// occupy a single block.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub txn_id: u64,
    pub entry_type: EntryType,
    pub target_block: u64,
    pub data_len: u32,
    pub checksum: u32,
    /// Full undo data (present only for BlockData entries).
    pub data: Vec<u8>,
}

impl JournalEntry {
    pub fn new_begin(txn_id: u64) -> Self {
        Self { txn_id, entry_type: EntryType::TxnBegin, target_block: 0, data_len: 0, checksum: 0, data: Vec::new() }
    }

    pub fn new_block_data(txn_id: u64, target_block: u64, old_data: Vec<u8>) -> Self {
        let data_len = old_data.len() as u32;
        Self { txn_id, entry_type: EntryType::BlockData, target_block, data_len, checksum: 0, data: old_data }
    }

    pub fn new_commit(txn_id: u64) -> Self {
        Self { txn_id, entry_type: EntryType::TxnCommit, target_block: 0, data_len: 0, checksum: 0, data: Vec::new() }
    }

    pub fn new_abort(txn_id: u64) -> Self {
        Self { txn_id, entry_type: EntryType::TxnAbort, target_block: 0, data_len: 0, checksum: 0, data: Vec::new() }
    }

    /// Number of journal entry slots this entry occupies.
    pub fn slot_count(&self) -> u64 {
        match self.entry_type {
            EntryType::BlockData => 2, // header block + data block
            _ => 1,
        }
    }

    /// Serialize the header block only (for all entry types).
    pub fn serialize_header(&self, block_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; block_size as usize];
        buf[0..8].copy_from_slice(&self.txn_id.to_le_bytes());
        buf[8] = self.entry_type as u8;
        // bytes 9..12 = padding zeros
        buf[12..20].copy_from_slice(&self.target_block.to_le_bytes());
        buf[20..24].copy_from_slice(&self.data_len.to_le_bytes());
        // bytes 24..28 = checksum (filled below)
        // bytes 28..32 = reserved zeros

        // Compute checksum over header[0..24] + data (data is in a separate block
        // for BlockData entries, but still included in the checksum)
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&buf[0..24]);
        if !self.data.is_empty() {
            hasher.update(&self.data);
        }
        let checksum = hasher.finalize();
        buf[24..28].copy_from_slice(&checksum.to_le_bytes());

        buf
    }

    /// Serialize the undo data block (only valid for BlockData entries).
    pub fn serialize_data_block(&self, block_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; block_size as usize];
        let copy_len = self.data.len().min(block_size as usize);
        if copy_len > 0 {
            buf[..copy_len].copy_from_slice(&self.data[..copy_len]);
        }
        buf
    }

    /// Deserialize a header block. For BlockData entries, call `read_data_block`
    /// separately to fill in the `data` field.
    pub fn deserialize_header(buf: &[u8], block_size: u32) -> Result<Self> {
        if (buf.len() as u32) < block_size || buf.len() < ENTRY_HEADER_SIZE {
            bail!("journal entry buffer too short");
        }
        let txn_id = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let entry_type = EntryType::from_u8(buf[8])?;
        let target_block = u64::from_le_bytes(buf[12..20].try_into().unwrap());
        let data_len = u32::from_le_bytes(buf[20..24].try_into().unwrap());
        let stored_checksum = u32::from_le_bytes(buf[24..28].try_into().unwrap());

        Ok(Self {
            txn_id, entry_type, target_block, data_len,
            checksum: stored_checksum,
            data: Vec::new(), // filled later for BlockData
        })
    }

    /// After deserializing the header of a BlockData entry, call this with
    /// the next block's raw bytes to fill in the data and verify the checksum.
    pub fn read_data_block(&mut self, header_buf: &[u8], data_buf: &[u8]) -> Result<()> {
        let data = data_buf[..self.data_len as usize].to_vec();

        // Verify checksum: hash header[0..24] + data
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&header_buf[0..24]);
        hasher.update(&data);
        let computed = hasher.finalize();
        if self.checksum != computed {
            bail!(
                "journal entry checksum mismatch: stored=0x{:08x}, computed=0x{:08x}",
                self.checksum, computed
            );
        }

        self.data = data;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Journal sizing helper
// ---------------------------------------------------------------------------

/// Compute journal size in blocks from total volume blocks and percentage.
pub fn compute_journal_blocks(total_blocks: u64, journal_percent: f32) -> u64 {
    if journal_percent == 0.0 {
        return 0;
    }
    let raw_blocks = (total_blocks as f64 * journal_percent as f64 / 100.0) as u64;
    let min_blocks = 64u64;
    let max_blocks = std::cmp::max((total_blocks as f64 * 0.01) as u64, 131072);
    raw_blocks.clamp(min_blocks, max_blocks)
}

// ---------------------------------------------------------------------------
// Journal (in-memory state)
// ---------------------------------------------------------------------------

/// In-memory journal manager.
pub struct Journal {
    /// On-disk journal superblock (cached in memory).
    pub jsb: JournalSuperblock,
    /// First block of journal region on volume.
    journal_start: u64,
    /// Volume block size.
    block_size: u32,
    /// Active (uncommitted) transaction ID, if any.
    active_txn: Option<u64>,
    /// Undo records for the active transaction (for abort rollback).
    active_undo: Vec<(u64, Vec<u8>)>,
}

impl Journal {
    // -----------------------------------------------------------------------
    // Circular buffer arithmetic
    // -----------------------------------------------------------------------

    /// Calculate the disk block for a given entry index.
    fn entry_disk_block(&self, entry_index: u64) -> u64 {
        self.journal_start + 1 + entry_index
    }

    /// Advance a position in the circular buffer.
    fn advance(&self, pos: u64) -> u64 {
        (pos + 1) % self.jsb.capacity
    }

    /// Number of entries currently in the journal (from tail to head).
    pub(crate) fn used_entries(&self) -> u64 {
        if self.jsb.head >= self.jsb.tail {
            self.jsb.head - self.jsb.tail
        } else {
            self.jsb.capacity - self.jsb.tail + self.jsb.head
        }
    }

    /// Number of free entry slots (reserve 1 to distinguish empty from full).
    fn free_entries(&self) -> u64 {
        self.jsb.capacity.saturating_sub(1).saturating_sub(self.used_entries())
    }

    fn has_space(&self, n: u64) -> bool {
        self.free_entries() >= n
    }

    // -----------------------------------------------------------------------
    // Internal I/O
    // -----------------------------------------------------------------------

    /// Write a journal entry at the current head position and advance head.
    /// For `BlockData` entries, writes 2 blocks (header + data) and advances
    /// head by 2.
    fn write_entry(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        entry: &JournalEntry,
    ) -> Result<()> {
        // Write header block
        let disk_block = self.entry_disk_block(self.jsb.head);
        let disk_offset = disk_block * self.block_size as u64;
        let header_buf = entry.serialize_header(self.block_size);
        dev.write(disk_offset, &header_buf)?;
        self.jsb.head = self.advance(self.jsb.head);

        // For BlockData entries, write a second block with the raw undo data
        if entry.entry_type == EntryType::BlockData {
            let data_block = self.entry_disk_block(self.jsb.head);
            let data_offset = data_block * self.block_size as u64;
            let data_buf = entry.serialize_data_block(self.block_size);
            dev.write(data_offset, &data_buf)?;
            self.jsb.head = self.advance(self.jsb.head);
        }

        Ok(())
    }

    /// Persist the journal superblock to disk.
    pub fn save_jsb(&self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let disk_offset = self.journal_start * self.block_size as u64;
        let buf = self.jsb.serialize(self.block_size);
        dev.write(disk_offset, &buf)?;
        dev.flush()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // 10E.3 — Transaction API
    // -----------------------------------------------------------------------

    /// Begin a new transaction. Returns the transaction ID.
    pub fn begin_txn(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<u64> {
        if self.active_txn.is_some() {
            bail!("cannot begin transaction: another transaction is already active");
        }
        // Need space for at least BEGIN(1) + 1 BLOCK_DATA(2) + COMMIT(1) = 4 slots
        if !self.has_space(4) {
            bail!("journal full: cannot begin transaction (need checkpoint)");
        }

        let txn_id = self.jsb.sequence;
        self.jsb.sequence += 1;

        // Mark journal as needing recovery
        self.jsb.flags |= JFLAG_NEEDS_RECOVERY;
        self.jsb.flags &= !JFLAG_CLEAN;

        // Write TXN_BEGIN entry
        let entry = JournalEntry::new_begin(txn_id);
        self.write_entry(dev, &entry)?;

        // Persist journal superblock
        self.save_jsb(dev)?;

        self.active_txn = Some(txn_id);
        self.active_undo.clear();

        Ok(txn_id)
    }

    /// Record the current contents of a volume block as an undo record.
    /// MUST be called BEFORE the block is modified.
    pub fn journal_block(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        txn_id: u64,
        target_block: u64,
    ) -> Result<()> {
        if self.active_txn != Some(txn_id) {
            bail!("no active transaction with ID {}", txn_id);
        }
        // Skip if this block was already journaled in this transaction —
        // the first snapshot (before any modification) is the correct undo state.
        if self.active_undo.iter().any(|(b, _)| *b == target_block) {
            return Ok(());
        }
        // Need space for this BLOCK_DATA(2 slots) + eventual COMMIT(1 slot) = 3
        if !self.has_space(3) {
            bail!("journal full: cannot add more blocks to transaction");
        }

        // Read the CURRENT contents of the target block (before modification)
        let mut old_data = vec![0u8; self.block_size as usize];
        let disk_offset = target_block * self.block_size as u64;
        dev.read(disk_offset, &mut old_data)?;

        // Save for potential abort
        self.active_undo.push((target_block, old_data.clone()));

        // Write BLOCK_DATA entry to journal
        let entry = JournalEntry::new_block_data(txn_id, target_block, old_data);
        self.write_entry(dev, &entry)?;

        Ok(())
    }

    /// Commit the active transaction. Modifications are now permanent.
    pub fn commit_txn(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        txn_id: u64,
    ) -> Result<()> {
        if self.active_txn != Some(txn_id) {
            bail!("no active transaction with ID {}", txn_id);
        }

        // Flush device to ensure all metadata modifications hit disk
        // BEFORE we write the commit record
        dev.flush()?;

        // Write TXN_COMMIT entry
        let entry = JournalEntry::new_commit(txn_id);
        self.write_entry(dev, &entry)?;

        // Flush again to ensure commit record itself is on disk
        dev.flush()?;

        // Persist journal superblock so the updated head covers all entries
        // (BEGIN + undo records + COMMIT). Without this, a crash after commit
        // but before the next sync/checkpoint would leave the on-disk head
        // pointing only past BEGIN, making undo records invisible to replay.
        self.save_jsb(dev)?;

        self.active_txn = None;
        self.active_undo.clear();
        Ok(())
    }

    /// Abort the active transaction. Restores old block contents.
    pub fn abort_txn(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        txn_id: u64,
    ) -> Result<()> {
        if self.active_txn != Some(txn_id) {
            bail!("no active transaction with ID {}", txn_id);
        }

        // Replay undo records in reverse order: restore old block contents.
        // Reverse order ensures dependent blocks (data) are restored before
        // metadata (inodes, indirect tables) that point to them.
        for (target_block, old_data) in self.active_undo.iter().rev() {
            let disk_offset = target_block * self.block_size as u64;
            dev.write(disk_offset, old_data)?;
        }
        dev.flush()?;

        // Write TXN_ABORT entry
        let entry = JournalEntry::new_abort(txn_id);
        self.write_entry(dev, &entry)?;

        self.active_txn = None;
        self.active_undo.clear();
        Ok(())
    }

    /// Returns true if a transaction is currently active.
    pub fn has_active_txn(&self) -> bool {
        self.active_txn.is_some()
    }

    /// Returns the active transaction ID, if any.
    pub fn active_txn_id(&self) -> Option<u64> {
        self.active_txn
    }

    // -----------------------------------------------------------------------
    // Checkpoint (space reclamation)
    // -----------------------------------------------------------------------

    /// Reclaim space by advancing the tail past all completed transactions.
    pub fn checkpoint(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<u64> {
        let mut pos = self.jsb.tail;
        let mut freed = 0u64;
        let mut txn_start = pos;

        while pos != self.jsb.head {
            let disk_block = self.entry_disk_block(pos);
            let disk_offset = disk_block * self.block_size as u64;
            let mut buf = vec![0u8; self.block_size as usize];
            dev.read(disk_offset, &mut buf)?;

            let entry = match JournalEntry::deserialize_header(&buf, self.block_size) {
                Ok(e) => e,
                Err(_) => break, // Corrupt entry — stop
            };

            pos = self.advance(pos);

            match entry.entry_type {
                EntryType::TxnBegin => {
                    txn_start = self.jsb.tail;
                }
                EntryType::BlockData => {
                    // Skip the data block (2nd slot of this entry)
                    pos = self.advance(pos);
                }
                EntryType::TxnCommit | EntryType::TxnAbort => {
                    // Transaction is complete: advance tail past all its slots
                    let entries_to_free = if pos >= txn_start {
                        pos - txn_start
                    } else {
                        self.jsb.capacity - txn_start + pos
                    };
                    self.jsb.tail = pos;
                    freed += entries_to_free;
                }
            }
        }

        if freed > 0 {
            self.save_jsb(dev)?;
        }

        Ok(freed)
    }

    /// Mark the journal as clean (called during sync/unmount).
    pub fn mark_clean(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        self.jsb.flags = JFLAG_CLEAN;
        self.jsb.flags &= !JFLAG_NEEDS_RECOVERY;
        self.save_jsb(dev)
    }

    // -----------------------------------------------------------------------
    // 10E.2 + 10E.5 — Initialization & Recovery
    // -----------------------------------------------------------------------

    /// Initialize a new journal on disk (called during format).
    pub fn init(
        dev: &mut dyn CFSBlockDevice,
        journal_start: u64,
        journal_blocks: u64,
        block_size: u32,
    ) -> Result<()> {
        if journal_blocks < 2 {
            bail!("journal must have at least 2 blocks (1 superblock + 1 entry)");
        }
        let capacity = journal_blocks - 1;
        let jsb = JournalSuperblock::new(capacity);
        let jsb_buf = jsb.serialize(block_size);
        dev.write(journal_start * block_size as u64, &jsb_buf)?;

        // Zero all journal entry blocks
        let zero_buf = vec![0u8; block_size as usize];
        for i in 1..journal_blocks {
            dev.write((journal_start + i) * block_size as u64, &zero_buf)?;
        }
        dev.flush()?;
        Ok(())
    }

    /// Load journal from disk and perform recovery if needed.
    pub fn load(
        dev: &mut dyn CFSBlockDevice,
        journal_start: u64,
        _journal_blocks: u64,
        block_size: u32,
    ) -> Result<Self> {
        let jsb_offset = journal_start * block_size as u64;
        let mut jsb_buf = vec![0u8; block_size as usize];
        dev.read(jsb_offset, &mut jsb_buf)?;
        let jsb = JournalSuperblock::deserialize(&jsb_buf)?;

        let mut journal = Self {
            jsb,
            journal_start,
            block_size,
            active_txn: None,
            active_undo: Vec::new(),
        };

        if journal.jsb.needs_recovery() {
            journal.replay(dev)?;
        }

        Ok(journal)
    }

    /// Replay the journal: undo incomplete transactions.
    fn replay(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        // Phase 1: Scan all entries and group by transaction
        let mut transactions: HashMap<u64, TransactionRecord> = HashMap::new();
        let mut scan_pos = self.jsb.tail;
        let mut scan_order: Vec<u64> = Vec::new();

        while scan_pos != self.jsb.head {
            let disk_block = self.entry_disk_block(scan_pos);
            let disk_offset = disk_block * self.block_size as u64;
            let mut header_buf = vec![0u8; self.block_size as usize];
            dev.read(disk_offset, &mut header_buf)?;

            match JournalEntry::deserialize_header(&header_buf, self.block_size) {
                Ok(mut entry) => {
                    scan_pos = self.advance(scan_pos);

                    // For BlockData entries, read the following data block
                    if entry.entry_type == EntryType::BlockData && scan_pos != self.jsb.head {
                        let data_block = self.entry_disk_block(scan_pos);
                        let data_offset = data_block * self.block_size as u64;
                        let mut data_buf = vec![0u8; self.block_size as usize];
                        dev.read(data_offset, &mut data_buf)?;
                        let data_corrupt = entry.read_data_block(&header_buf, &data_buf).is_err();
                        scan_pos = self.advance(scan_pos);
                        if data_corrupt {
                            // Mark this transaction as having corrupt data so
                            // replay treats it as incomplete, but keep scanning
                            // to avoid losing later committed transactions.
                            let txn = transactions
                                .entry(entry.txn_id)
                                .or_insert_with(|| {
                                    scan_order.push(entry.txn_id);
                                    TransactionRecord::new(entry.txn_id)
                                });
                            txn.has_corrupt_data = true;
                            continue;
                        }
                    }

                    let txn = transactions
                        .entry(entry.txn_id)
                        .or_insert_with(|| {
                            scan_order.push(entry.txn_id);
                            TransactionRecord::new(entry.txn_id)
                        });
                    match entry.entry_type {
                        EntryType::TxnBegin => txn.has_begin = true,
                        EntryType::BlockData => {
                            txn.undo_records.push((entry.target_block, entry.data));
                        }
                        EntryType::TxnCommit => txn.has_commit = true,
                        EntryType::TxnAbort => txn.has_abort = true,
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }

        // Phase 2: Undo incomplete transactions
        for txn_id in &scan_order {
            if let Some(txn) = transactions.get(txn_id) {
                if txn.has_begin && !txn.has_commit && !txn.has_abort
                    || txn.has_corrupt_data
                {
                    // Incomplete: replay undo records in REVERSE order
                    for (target_block, old_data) in txn.undo_records.iter().rev() {
                        let disk_offset = target_block * self.block_size as u64;
                        dev.write(disk_offset, old_data)?;
                    }
                }
            }
        }

        // Phase 3: Flush and reset journal
        dev.flush()?;
        self.jsb.head = 0;
        self.jsb.tail = 0;
        self.jsb.flags = JFLAG_CLEAN;
        self.save_jsb(dev)?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Status / introspection
    // -----------------------------------------------------------------------

    /// Return a displayable status summary.
    pub fn status(&self) -> JournalStatus {
        JournalStatus {
            journal_start: self.journal_start,
            capacity: self.jsb.capacity,
            used: self.used_entries(),
            sequence: self.jsb.sequence,
            head: self.jsb.head,
            tail: self.jsb.tail,
            needs_recovery: self.jsb.needs_recovery(),
            clean: self.jsb.flags & JFLAG_CLEAN != 0,
            block_size: self.block_size,
        }
    }
}

/// Read-only snapshot of journal state for display.
#[derive(Debug)]
pub struct JournalStatus {
    pub journal_start: u64,
    pub capacity: u64,
    pub used: u64,
    pub sequence: u64,
    pub head: u64,
    pub tail: u64,
    pub needs_recovery: bool,
    pub clean: bool,
    pub block_size: u32,
}

impl std::fmt::Display for JournalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total_blocks = self.capacity + 1; // capacity excludes jsb block
        let size_bytes = total_blocks * self.block_size as u64;
        let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
        let pct = if self.capacity > 0 {
            self.used as f64 / self.capacity as f64 * 100.0
        } else {
            0.0
        };
        writeln!(f, "Journal Status:")?;
        writeln!(f, "  Region:         blocks {}..{} ({:.1} MB, {} blocks)",
            self.journal_start, self.journal_start + total_blocks, size_mb, total_blocks)?;
        writeln!(f, "  State:          {}", if self.clean { "Clean" } else { "Dirty" })?;
        writeln!(f, "  Sequence:       {} (next txn ID)", self.sequence)?;
        writeln!(f, "  Head:           {} (next write position)", self.head)?;
        writeln!(f, "  Tail:           {} (oldest entry)", self.tail)?;
        writeln!(f, "  Capacity:       {} entry slots", self.capacity)?;
        writeln!(f, "  Used:           {} entries ({:.1}%)", self.used, pct)?;
        write!(f, "  Needs recovery: {}", if self.needs_recovery { "Yes" } else { "No" })
    }
}

// ---------------------------------------------------------------------------
// Internal: transaction record for replay
// ---------------------------------------------------------------------------

struct TransactionRecord {
    #[allow(dead_code)]
    txn_id: u64,
    has_begin: bool,
    has_commit: bool,
    has_abort: bool,
    has_corrupt_data: bool,
    undo_records: Vec<(u64, Vec<u8>)>,
}

impl TransactionRecord {
    fn new(txn_id: u64) -> Self {
        Self {
            txn_id,
            has_begin: false,
            has_commit: false,
            has_abort: false,
            has_corrupt_data: false,
            undo_records: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use tempfile::NamedTempFile;

    const BS: u32 = 4096;

    fn make_dev(blocks: u64) -> (NamedTempFile, Box<dyn CFSBlockDevice>) {
        let tmp = NamedTempFile::new().unwrap();
        let size = blocks * BS as u64;
        let dev = FileBlockDevice::open(tmp.path(), Some(size)).unwrap();
        (tmp, Box::new(dev))
    }

    // -----------------------------------------------------------------------
    // 10E.2 — JournalSuperblock tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_journal_superblock_roundtrip() {
        let jsb = JournalSuperblock::new(100);
        let buf = jsb.serialize(BS);
        let jsb2 = JournalSuperblock::deserialize(&buf).unwrap();
        assert_eq!(jsb2.magic, JOURNAL_MAGIC);
        assert_eq!(jsb2.version, JOURNAL_VERSION);
        assert_eq!(jsb2.sequence, 1);
        assert_eq!(jsb2.head, 0);
        assert_eq!(jsb2.tail, 0);
        assert_eq!(jsb2.capacity, 100);
        assert!(jsb2.flags & JFLAG_CLEAN != 0);
    }

    #[test]
    fn test_journal_superblock_bad_magic() {
        let mut buf = JournalSuperblock::new(100).serialize(BS);
        buf[0] = 0xFF;
        assert!(JournalSuperblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_journal_superblock_bad_checksum() {
        let mut buf = JournalSuperblock::new(100).serialize(BS);
        buf[10] ^= 0xFF; // corrupt data
        assert!(JournalSuperblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_journal_superblock_empty() {
        let jsb = JournalSuperblock::new(100);
        assert!(jsb.is_empty());
    }

    #[test]
    fn test_journal_superblock_needs_recovery() {
        let mut jsb = JournalSuperblock::new(100);
        assert!(!jsb.needs_recovery());
        jsb.flags |= JFLAG_NEEDS_RECOVERY;
        assert!(jsb.needs_recovery());
    }

    // -----------------------------------------------------------------------
    // 10E.2 — JournalEntry tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_entry_begin_roundtrip() {
        let entry = JournalEntry::new_begin(42);
        let buf = entry.serialize_header(BS);
        let entry2 = JournalEntry::deserialize_header(&buf, BS).unwrap();
        assert_eq!(entry2.txn_id, 42);
        assert_eq!(entry2.entry_type, EntryType::TxnBegin);
    }

    #[test]
    fn test_entry_block_data_roundtrip() {
        let data = vec![0xAB; BS as usize];
        let entry = JournalEntry::new_block_data(7, 500, data.clone());
        let header_buf = entry.serialize_header(BS);
        let data_buf = entry.serialize_data_block(BS);
        let mut entry2 = JournalEntry::deserialize_header(&header_buf, BS).unwrap();
        entry2.read_data_block(&header_buf, &data_buf).unwrap();
        assert_eq!(entry2.txn_id, 7);
        assert_eq!(entry2.entry_type, EntryType::BlockData);
        assert_eq!(entry2.target_block, 500);
        assert_eq!(entry2.data, data);
    }

    #[test]
    fn test_entry_commit_roundtrip() {
        let entry = JournalEntry::new_commit(99);
        let buf = entry.serialize_header(BS);
        let entry2 = JournalEntry::deserialize_header(&buf, BS).unwrap();
        assert_eq!(entry2.txn_id, 99);
        assert_eq!(entry2.entry_type, EntryType::TxnCommit);
    }

    #[test]
    fn test_entry_abort_roundtrip() {
        let entry = JournalEntry::new_abort(55);
        let buf = entry.serialize_header(BS);
        let entry2 = JournalEntry::deserialize_header(&buf, BS).unwrap();
        assert_eq!(entry2.txn_id, 55);
        assert_eq!(entry2.entry_type, EntryType::TxnAbort);
    }

    #[test]
    fn test_entry_bad_checksum() {
        let entry = JournalEntry::new_begin(1);
        let mut buf = entry.serialize_header(BS);
        // Corrupt header data (not the checksum field itself)
        buf[10] ^= 0xFF;
        // For begin entries, we can try deserialize_header which doesn't verify
        // checksum alone — checksum is verified only when we have data.
        // Instead, test with a block_data entry:
        let data = vec![0xAB; BS as usize];
        let entry2 = JournalEntry::new_block_data(1, 10, data);
        let header_buf = entry2.serialize_header(BS);
        let data_buf = entry2.serialize_data_block(BS);
        let mut entry3 = JournalEntry::deserialize_header(&header_buf, BS).unwrap();
        // Corrupt the data buf
        let mut bad_data = data_buf.clone();
        bad_data[0] ^= 0xFF;
        assert!(entry3.read_data_block(&header_buf, &bad_data).is_err());
    }

    #[test]
    fn test_entry_type_from_u8() {
        assert_eq!(EntryType::from_u8(0).unwrap(), EntryType::TxnBegin);
        assert_eq!(EntryType::from_u8(1).unwrap(), EntryType::BlockData);
        assert_eq!(EntryType::from_u8(2).unwrap(), EntryType::TxnCommit);
        assert_eq!(EntryType::from_u8(3).unwrap(), EntryType::TxnAbort);
        assert!(EntryType::from_u8(4).is_err());
    }

    // -----------------------------------------------------------------------
    // Circular buffer arithmetic
    // -----------------------------------------------------------------------

    #[test]
    fn test_circular_advance() {
        let (_tmp, _dev) = make_dev(100);
        let j = Journal {
            jsb: JournalSuperblock { capacity: 10, ..JournalSuperblock::new(10) },
            journal_start: 0, block_size: BS, active_txn: None, active_undo: Vec::new(),
        };
        assert_eq!(j.advance(0), 1);
        assert_eq!(j.advance(9), 0); // wraps around
    }

    #[test]
    fn test_circular_used_entries() {
        let mut jsb = JournalSuperblock::new(10);
        jsb.head = 5;
        jsb.tail = 2;
        let j = Journal {
            jsb: jsb.clone(), journal_start: 0, block_size: BS, active_txn: None, active_undo: Vec::new(),
        };
        assert_eq!(j.used_entries(), 3);

        jsb.head = 2;
        jsb.tail = 5;
        let j2 = Journal {
            jsb, journal_start: 0, block_size: BS, active_txn: None, active_undo: Vec::new(),
        };
        assert_eq!(j2.used_entries(), 7);
    }

    #[test]
    fn test_circular_free_entries() {
        let mut jsb = JournalSuperblock::new(100);
        jsb.head = 30;
        jsb.tail = 0;
        let j = Journal {
            jsb, journal_start: 0, block_size: BS, active_txn: None, active_undo: Vec::new(),
        };
        assert_eq!(j.free_entries(), 69); // 100 - 1 - 30
    }

    #[test]
    fn test_compute_journal_blocks() {
        // 4 GB / 4K = 1048576 blocks, 1% = 10485
        let blocks = compute_journal_blocks(1048576, 1.0);
        assert!(blocks >= 64);
        assert!(blocks <= 131072);

        // Tiny volume → minimum
        assert_eq!(compute_journal_blocks(100, 1.0), 64);

        // Disabled
        assert_eq!(compute_journal_blocks(1000000, 0.0), 0);
    }

    // -----------------------------------------------------------------------
    // 10E.3 — Transaction API tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_begin_txn_returns_id() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn_id = j.begin_txn(&mut *dev).unwrap();
        assert_eq!(txn_id, 1);
        // Commit before next
        j.commit_txn(&mut *dev, txn_id).unwrap();
        let txn_id2 = j.begin_txn(&mut *dev).unwrap();
        assert_eq!(txn_id2, 2);
        j.commit_txn(&mut *dev, txn_id2).unwrap();
    }

    #[test]
    fn test_begin_txn_sets_needs_recovery() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn_id = j.begin_txn(&mut *dev).unwrap();
        assert!(j.jsb.needs_recovery());
        j.commit_txn(&mut *dev, txn_id).unwrap();
    }

    #[test]
    fn test_begin_txn_double_begin_error() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        j.begin_txn(&mut *dev).unwrap();
        assert!(j.begin_txn(&mut *dev).is_err());
    }

    #[test]
    fn test_journal_block_saves_old_data() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();

        // Write known data to a block outside journal area
        let target_block = 150u64;
        let original = vec![0xABu8; BS as usize];
        dev.write(target_block * BS as u64, &original).unwrap();

        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn = j.begin_txn(&mut *dev).unwrap();
        j.journal_block(&mut *dev, txn, target_block).unwrap();

        // Verify undo record contains the original data
        assert_eq!(j.active_undo.len(), 1);
        assert_eq!(j.active_undo[0].0, target_block);
        assert_eq!(j.active_undo[0].1, original);

        j.commit_txn(&mut *dev, txn).unwrap();
    }

    #[test]
    fn test_abort_txn_restores_old_data() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();

        let target_block = 150u64;
        let original = vec![0xABu8; BS as usize];
        dev.write(target_block * BS as u64, &original).unwrap();

        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn = j.begin_txn(&mut *dev).unwrap();
        j.journal_block(&mut *dev, txn, target_block).unwrap();

        // Overwrite the block (simulating the metadata modification)
        let new_data = vec![0xCD; BS as usize];
        dev.write(target_block * BS as u64, &new_data).unwrap();

        // Abort should restore original
        j.abort_txn(&mut *dev, txn).unwrap();

        let mut check = vec![0u8; BS as usize];
        dev.read(target_block * BS as u64, &mut check).unwrap();
        assert_eq!(check, original);
    }

    #[test]
    fn test_full_lifecycle() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();

        let txn = j.begin_txn(&mut *dev).unwrap();
        j.journal_block(&mut *dev, txn, 150).unwrap();
        j.journal_block(&mut *dev, txn, 151).unwrap();
        j.journal_block(&mut *dev, txn, 152).unwrap();
        j.commit_txn(&mut *dev, txn).unwrap();

        // 1 BEGIN + 3 BLOCK_DATA(2 each) + 1 COMMIT = 1 + 6 + 1 = 8 slots used
        assert_eq!(j.used_entries(), 8);
    }

    #[test]
    fn test_checkpoint_frees_committed() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();

        // Create and commit 2 transactions
        for _ in 0..2 {
            let txn = j.begin_txn(&mut *dev).unwrap();
            j.journal_block(&mut *dev, txn, 150).unwrap();
            j.commit_txn(&mut *dev, txn).unwrap();
        }

        let used_before = j.used_entries();
        assert!(used_before > 0);

        let freed = j.checkpoint(&mut *dev).unwrap();
        assert_eq!(freed, used_before);
        assert_eq!(j.used_entries(), 0);
    }

    // -----------------------------------------------------------------------
    // 10E.5 — Journal Replay tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_replay_clean_journal_no_op() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();
        let j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        assert!(!j.jsb.needs_recovery());
        assert!(j.jsb.flags & JFLAG_CLEAN != 0);
    }

    #[test]
    fn test_replay_incomplete_txn_undone() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();

        // Write known data to blocks 150 and 151
        let original_150 = vec![0xAAu8; BS as usize];
        let original_151 = vec![0xBBu8; BS as usize];
        dev.write(150 * BS as u64, &original_150).unwrap();
        dev.write(151 * BS as u64, &original_151).unwrap();

        // Start a transaction, journal both blocks
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn = j.begin_txn(&mut *dev).unwrap();
        j.journal_block(&mut *dev, txn, 150).unwrap();
        j.journal_block(&mut *dev, txn, 151).unwrap();

        // Overwrite the blocks (simulating metadata changes)
        dev.write(150 * BS as u64, &vec![0xFF; BS as usize]).unwrap();
        dev.write(151 * BS as u64, &vec![0xEE; BS as usize]).unwrap();

        // "Crash" — do NOT commit. Save the jsb so the state is on disk.
        j.save_jsb(&mut *dev).unwrap();
        dev.flush().unwrap();

        // Reload journal — should detect NEEDS_RECOVERY and replay
        let j2 = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        assert!(!j2.jsb.needs_recovery());
        assert!(j2.jsb.flags & JFLAG_CLEAN != 0);

        // Verify blocks 150 and 151 are restored
        let mut check = vec![0u8; BS as usize];
        dev.read(150 * BS as u64, &mut check).unwrap();
        assert_eq!(check, original_150);
        dev.read(151 * BS as u64, &mut check).unwrap();
        assert_eq!(check, original_151);
    }

    #[test]
    fn test_replay_committed_txn_no_undo() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 0, 100, BS).unwrap();

        let original = vec![0xAAu8; BS as usize];
        dev.write(150 * BS as u64, &original).unwrap();

        // Commit a transaction
        let mut j = Journal::load(&mut *dev, 0, 100, BS).unwrap();
        let txn = j.begin_txn(&mut *dev).unwrap();
        j.journal_block(&mut *dev, txn, 150).unwrap();

        // Write new data
        let new_data = vec![0xFF; BS as usize];
        dev.write(150 * BS as u64, &new_data).unwrap();
        j.commit_txn(&mut *dev, txn).unwrap();

        // Force NEEDS_RECOVERY to simulate state (even though committed)
        j.jsb.flags |= JFLAG_NEEDS_RECOVERY;
        j.jsb.flags &= !JFLAG_CLEAN;
        j.save_jsb(&mut *dev).unwrap();

        // Reload — committed txn should NOT be undone
        let _j2 = Journal::load(&mut *dev, 0, 100, BS).unwrap();

        // Block 150 should still have the NEW data (commit was successful)
        let mut check = vec![0u8; BS as usize];
        dev.read(150 * BS as u64, &mut check).unwrap();
        assert_eq!(check, new_data);
    }

    #[test]
    fn test_journal_init_and_load() {
        let (_tmp, mut dev) = make_dev(200);
        Journal::init(&mut *dev, 10, 50, BS).unwrap();
        let j = Journal::load(&mut *dev, 10, 50, BS).unwrap();
        assert_eq!(j.jsb.capacity, 49);
        assert_eq!(j.jsb.sequence, 1);
        assert!(j.jsb.is_empty());
    }
}
