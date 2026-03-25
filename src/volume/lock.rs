//! Byte-range file locking for CFS.
//!
//! Provides `FileLockManager` which tracks advisory byte-range locks on files,
//! supporting shared (read) and exclusive (write) lock types. Compatible with
//! POSIX `fcntl(F_SETLK)` and Windows `LockFileEx` semantics.

use anyhow::{bail, Result};
use std::collections::HashMap;

/// Type of byte-range lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    /// Shared (read) lock — multiple readers allowed, no writers.
    Shared,
    /// Exclusive (write) lock — no other readers or writers.
    Exclusive,
}

/// A byte-range lock on a file.
#[derive(Debug, Clone)]
pub struct FileLock {
    /// Inode index of the locked file.
    pub inode: u32,
    /// Start offset of the locked range (inclusive).
    pub offset: u64,
    /// Length of the locked range. 0 means "to end of file".
    pub length: u64,
    /// Lock type (shared or exclusive).
    pub lock_type: LockType,
    /// Opaque handle identifying the lock owner (e.g., thread ID or file handle).
    pub owner: u64,
}

impl FileLock {
    /// Check if two locks' byte ranges overlap.
    pub fn overlaps(&self, other: &FileLock) -> bool {
        if self.inode != other.inode {
            return false;
        }
        let self_end = if self.length == 0 {
            u64::MAX
        } else {
            self.offset.saturating_add(self.length)
        };
        let other_end = if other.length == 0 {
            u64::MAX
        } else {
            other.offset.saturating_add(other.length)
        };
        self.offset < other_end && other.offset < self_end
    }

    /// Check if this lock conflicts with another lock.
    /// Shared-Shared locks don't conflict; all other combinations do.
    pub fn conflicts_with(&self, other: &FileLock) -> bool {
        if !self.overlaps(other) {
            return false;
        }
        // Same owner → no conflict (lock upgrade/downgrade)
        if self.owner == other.owner {
            return false;
        }
        // Shared-Shared → no conflict
        if self.lock_type == LockType::Shared && other.lock_type == LockType::Shared {
            return false;
        }
        true
    }
}

/// Manages byte-range locks for all open files.
pub struct FileLockManager {
    /// Active locks grouped by inode index.
    locks: HashMap<u32, Vec<FileLock>>,
}

impl FileLockManager {
    pub fn new() -> Self {
        Self {
            locks: HashMap::new(),
        }
    }

    /// Attempt to acquire a lock. Returns `Ok(())` if successful,
    /// `Err` with conflict details if blocked.
    pub fn try_lock(&mut self, lock: FileLock) -> Result<()> {
        let inode_locks = self.locks.entry(lock.inode).or_default();

        for existing in inode_locks.iter() {
            if lock.conflicts_with(existing) {
                bail!(
                    "lock conflict on inode {}: requested {:?} [{}, +{}] conflicts with {:?} [{}, +{}] (owner {})",
                    lock.inode,
                    lock.lock_type,
                    lock.offset,
                    lock.length,
                    existing.lock_type,
                    existing.offset,
                    existing.length,
                    existing.owner,
                );
            }
        }

        inode_locks.push(lock);
        Ok(())
    }

    /// Release a specific lock.
    pub fn unlock(&mut self, inode: u32, offset: u64, length: u64, owner: u64) -> Result<()> {
        let inode_locks = self
            .locks
            .get_mut(&inode)
            .ok_or_else(|| anyhow::anyhow!("no locks on inode {}", inode))?;

        let pos = inode_locks
            .iter()
            .position(|l| l.offset == offset && l.length == length && l.owner == owner)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "lock not found: inode={}, offset={}, length={}, owner={}",
                    inode,
                    offset,
                    length,
                    owner,
                )
            })?;

        inode_locks.remove(pos);

        if inode_locks.is_empty() {
            self.locks.remove(&inode);
        }

        Ok(())
    }

    /// Release ALL locks held by a specific owner on a specific inode.
    /// Called when a file handle is closed.
    pub fn release_all(&mut self, inode: u32, owner: u64) {
        if let Some(inode_locks) = self.locks.get_mut(&inode) {
            inode_locks.retain(|l| l.owner != owner);
            if inode_locks.is_empty() {
                self.locks.remove(&inode);
            }
        }
    }

    /// Release ALL locks held by a specific owner across all inodes.
    /// Called when a process/thread terminates or the last handle is closed.
    pub fn release_all_for_owner(&mut self, owner: u64) {
        self.locks.retain(|_, locks| {
            locks.retain(|l| l.owner != owner);
            !locks.is_empty()
        });
    }

    /// Check if a lock would conflict without actually acquiring it.
    pub fn would_conflict(&self, lock: &FileLock) -> Option<&FileLock> {
        if let Some(inode_locks) = self.locks.get(&lock.inode) {
            for existing in inode_locks {
                if lock.conflicts_with(existing) {
                    return Some(existing);
                }
            }
        }
        None
    }

    /// Get all active locks for an inode (diagnostic/debug).
    pub fn active_locks(&self, inode: u32) -> &[FileLock] {
        self.locks.get(&inode).map_or(&[], |v| v.as_slice())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_shared_shared_no_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Shared,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Shared,
            owner: 2,
        })
        .unwrap();
    }

    #[test]
    fn test_lock_shared_exclusive_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Shared,
            owner: 1,
        })
        .unwrap();
        assert!(mgr
            .try_lock(FileLock {
                inode: 1,
                offset: 0,
                length: 100,
                lock_type: LockType::Exclusive,
                owner: 2,
            })
            .is_err());
    }

    #[test]
    fn test_lock_exclusive_shared_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        assert!(mgr
            .try_lock(FileLock {
                inode: 1,
                offset: 0,
                length: 100,
                lock_type: LockType::Shared,
                owner: 2,
            })
            .is_err());
    }

    #[test]
    fn test_lock_exclusive_exclusive_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        assert!(mgr
            .try_lock(FileLock {
                inode: 1,
                offset: 0,
                length: 100,
                lock_type: LockType::Exclusive,
                owner: 2,
            })
            .is_err());
    }

    #[test]
    fn test_lock_non_overlapping_ok() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 100,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 2,
        })
        .unwrap();
    }

    #[test]
    fn test_lock_partial_overlap_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        assert!(mgr
            .try_lock(FileLock {
                inode: 1,
                offset: 50,
                length: 100,
                lock_type: LockType::Exclusive,
                owner: 2,
            })
            .is_err());
    }

    #[test]
    fn test_lock_full_file_range() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 0, // whole file
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        assert!(mgr
            .try_lock(FileLock {
                inode: 1,
                offset: 500,
                length: 10,
                lock_type: LockType::Shared,
                owner: 2,
            })
            .is_err());
    }

    #[test]
    fn test_lock_same_owner_no_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        // Same owner, overlapping → no conflict
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 50,
            length: 100,
            lock_type: LockType::Shared,
            owner: 1,
        })
        .unwrap();
    }

    #[test]
    fn test_lock_unlock() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.unlock(1, 0, 100, 1).unwrap();
        // Now another owner can lock the same range
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 2,
        })
        .unwrap();
    }

    #[test]
    fn test_lock_release_all_inode() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 200,
            length: 50,
            lock_type: LockType::Shared,
            owner: 1,
        })
        .unwrap();
        mgr.release_all(1, 1);
        assert!(mgr.active_locks(1).is_empty());
    }

    #[test]
    fn test_lock_release_all_for_owner() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 2,
            offset: 0,
            length: 50,
            lock_type: LockType::Shared,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 200,
            length: 10,
            lock_type: LockType::Shared,
            owner: 2,
        })
        .unwrap();
        mgr.release_all_for_owner(1);
        assert!(mgr.active_locks(1).len() == 1); // owner 2's lock remains
        assert!(mgr.active_locks(2).is_empty());
    }

    #[test]
    fn test_lock_would_conflict() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        let query = FileLock {
            inode: 1,
            offset: 50,
            length: 10,
            lock_type: LockType::Shared,
            owner: 2,
        };
        assert!(mgr.would_conflict(&query).is_some());
    }

    #[test]
    fn test_lock_different_inodes() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 2,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 2,
        })
        .unwrap();
    }

    #[test]
    fn test_lock_mixed_ranges() {
        let mut mgr = FileLockManager::new();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 0,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 1,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 100,
            length: 100,
            lock_type: LockType::Exclusive,
            owner: 2,
        })
        .unwrap();
        mgr.try_lock(FileLock {
            inode: 1,
            offset: 200,
            length: 50,
            lock_type: LockType::Shared,
            owner: 3,
        })
        .unwrap();
        assert_eq!(mgr.active_locks(1).len(), 3);
    }
}
