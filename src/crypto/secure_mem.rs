//! Secure memory utilities: `LockedBuffer` pins key material in RAM to prevent
//! Windows from paging it to `pagefile.sys`, where it could be forensically
//! recovered even after the volume is unmounted.
//!
//! On non-Windows targets the type still compiles and provides correct
//! zeroize-on-drop semantics, but without the memory-locking guarantee.

use zeroize::Zeroize;

// ---------------------------------------------------------------------------
// LockedBuffer
// ---------------------------------------------------------------------------

/// A heap buffer whose pages are locked in physical RAM via `VirtualLock`.
///
/// Prevents the OS from swapping the buffer contents to disk while the volume
/// is mounted, protecting cryptographic key material from memory-forensics.
///
/// # Behaviour
/// - On **Windows**: `VirtualLock` is called on construction; on drop the
///   buffer is first zeroized, then `VirtualUnlock`ed and freed.
/// - On **other platforms**: falls back to plain `Vec<u8>` with zeroize-on-drop.
///
/// # Limitations
/// Windows limits the total locked-memory quota per process (typically 8 MiB
/// for non-privileged processes). This is sufficient for key material (32–128
/// bytes) but must not be used for large data buffers.
pub struct LockedBuffer {
    data: Vec<u8>,
    /// True only when VirtualLock succeeded on Windows.
    locked: bool,
}

impl LockedBuffer {
    /// Allocate `size` zero-initialised bytes and attempt to lock them in RAM.
    pub fn new(size: usize) -> Self {
        let mut data = vec![0u8; size];

        #[cfg(target_os = "windows")]
        let locked = unsafe {
            // SAFETY: `data` is valid committed heap memory of length `size`.
            // VirtualLock only requires the range to be committed (heap always
            // is) and within the process address space.
            windows_sys::Win32::System::Memory::VirtualLock(
                data.as_mut_ptr() as *mut core::ffi::c_void,
                size,
            ) != 0
        };

        #[cfg(not(target_os = "windows"))]
        let locked = false;

        Self { data, locked }
    }

    /// Borrow the buffer as a byte slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Mutably borrow the buffer as a byte slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Returns `true` if `VirtualLock` succeeded and the memory is pinned.
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Returns the length of the buffer in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the buffer has zero length.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Drop for LockedBuffer {
    fn drop(&mut self) {
        // Always zeroize *before* unlocking / freeing so that if the unlock
        // call races with a page-out (shouldn't happen but be defensive) the
        // contents are already wiped.
        self.data.zeroize();

        #[cfg(target_os = "windows")]
        if self.locked {
            unsafe {
                // SAFETY: we only unlock if we successfully locked; buffer is
                // still valid in drop (drop takes &mut self, no double-drop).
                windows_sys::Win32::System::Memory::VirtualUnlock(
                    self.data.as_ptr() as *const core::ffi::c_void,
                    self.data.len(),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy `key` bytes into a new [`LockedBuffer`] and zeroize the source slice.
///
/// Combines allocation, lock, copy, and source-zeroing into one call so
/// callers cannot accidentally skip the source-zero step.
pub fn lock_key(key: &mut [u8]) -> LockedBuffer {
    let mut buf = LockedBuffer::new(key.len());
    buf.as_mut_slice().copy_from_slice(key);
    key.zeroize();
    buf
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_buffer_initialised_to_zero() {
        let buf = LockedBuffer::new(64);
        assert_eq!(buf.as_slice(), &[0u8; 64]);
        assert_eq!(buf.len(), 64);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_locked_buffer_write_read() {
        let mut buf = LockedBuffer::new(32);
        buf.as_mut_slice().copy_from_slice(&[0xABu8; 32]);
        assert_eq!(buf.as_slice(), &[0xABu8; 32]);
    }

    #[test]
    fn test_lock_key_zeroizes_source() {
        let mut key = vec![0x42u8; 64];
        let buf = lock_key(&mut key);
        assert_eq!(key, vec![0u8; 64], "source slice must be zeroized");
        assert_eq!(buf.as_slice(), &[0x42u8; 64], "buffer must hold the key");
    }

    #[test]
    fn test_locked_buffer_drop_does_not_panic() {
        let mut buf = LockedBuffer::new(128);
        buf.as_mut_slice().fill(0xFF);
        assert_eq!(buf.as_slice()[0], 0xFF);
        drop(buf); // must not panic; zeroizes then unlocks
    }

    #[test]
    fn test_locked_buffer_is_locked_returns_bool() {
        // We can't assert the value since it depends on OS / privileges,
        // but the call must not panic.
        let buf = LockedBuffer::new(64);
        let _ = buf.is_locked();
    }

    #[test]
    fn test_empty_locked_buffer() {
        let buf = LockedBuffer::new(0);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        drop(buf); // must not panic on zero-length unlock
    }
}
