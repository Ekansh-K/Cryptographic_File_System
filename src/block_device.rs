use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};

// Win32 FFI via windows-sys (used by RawPartitionBlockDevice).
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, SetFilePointerEx, WriteFile, FILE_BEGIN,
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY, GET_LENGTH_INFORMATION, IOCTL_DISK_GET_DRIVE_GEOMETRY,
    IOCTL_DISK_GET_LENGTH_INFO,
};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Low-level block I/O interface.
///
/// Every implementation guarantees sector-aligned access.  Offsets and buffer
/// lengths **must** be multiples of `sector_size()`.
pub trait CFSBlockDevice: Send {
    /// Read up to `buf.len()` bytes starting at `offset`.
    /// Returns the number of bytes actually read.
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Write `buf` starting at `offset`.
    /// Returns the number of bytes actually written.
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize>;

    /// Total device/file size in bytes.
    fn size(&self) -> u64;

    /// Sector size in bytes (typically 512).
    fn sector_size(&self) -> u32;

    /// Flush any buffered writes to the underlying storage.
    fn flush(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// FileBlockDevice
// ---------------------------------------------------------------------------

const DEFAULT_SECTOR_SIZE: u32 = 512;

/// A block device backed by a regular file (e.g. `cfs.img`).
pub struct FileBlockDevice {
    file: File,
    size: u64,
    sector_size: u32,
}

impl FileBlockDevice {
    /// Open (or create) a backing file.
    ///
    /// If the file does not exist it is created and expanded to `create_size`
    /// bytes.  If `create_size` is `None` and the file does not exist, an
    /// error is returned.
    pub fn open(path: &Path, create_size: Option<u64>) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create_size.is_some())
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;

        let metadata = file.metadata()?;
        let mut size = metadata.len();

        // Expand a newly-created (empty) file to the requested size.
        if size == 0 {
            if let Some(cs) = create_size {
                file.set_len(cs)?;
                size = cs;
            } else {
                bail!("file {} is empty and no create_size was given", path.display());
            }
        }

        Ok(Self {
            file,
            size,
            sector_size: DEFAULT_SECTOR_SIZE,
        })
    }
}

impl CFSBlockDevice for FileBlockDevice {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let ss = self.sector_size as u64;
        if offset % ss != 0 || (buf.len() as u64) % ss != 0 {
            bail!(
                "unaligned read: offset={offset}, len={} (sector_size={ss})",
                buf.len()
            );
        }
        self.file.seek(SeekFrom::Start(offset))?;
        let n = self.file.read(buf)?;
        Ok(n)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize> {
        let ss = self.sector_size as u64;
        if offset % ss != 0 || (buf.len() as u64) % ss != 0 {
            bail!(
                "unaligned write: offset={offset}, len={} (sector_size={ss})",
                buf.len()
            );
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RawPartitionBlockDevice
// ---------------------------------------------------------------------------

/// Standard Win32 generic-access flags (not exported by windows-sys).
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

/// A block device backed by a raw Windows partition (e.g. `\\.\Z:`).
///
/// All I/O goes through Win32 `ReadFile` / `WriteFile` with
/// `FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH` so the OS does not
/// cache or re-order our sector writes.
pub struct RawPartitionBlockDevice {
    handle: HANDLE,
    size: u64,
    sector_size: u32,
}

// HANDLE can be sent across threads (it is just a pointer-sized value).
// SAFETY: Win32 file handles are safe to send between threads.
unsafe impl Send for RawPartitionBlockDevice {}

impl RawPartitionBlockDevice {
    /// Open a raw partition by its device path (e.g. `\\.\Z:`).
    ///
    /// Queries the disk geometry and partition length via
    /// `DeviceIoControl`.
    pub fn open(device_path: &str) -> Result<Self> {
        // Encode path as null-terminated UTF-16.
        let wide: Vec<u16> = OsStr::new(device_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: calling CreateFileW with valid pointer and known constants.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            bail!(
                "CreateFileW({device_path}) failed (os error {})",
                std::io::Error::last_os_error()
            );
        }

        let sector_size = query_sector_size(handle, device_path)?;
        let size = query_partition_length(handle, device_path)?;

        Ok(Self {
            handle,
            size,
            sector_size,
        })
    }
}

impl Drop for RawPartitionBlockDevice {
    fn drop(&mut self) {
        // SAFETY: handle was returned by a successful CreateFileW.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

impl CFSBlockDevice for RawPartitionBlockDevice {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let ss = self.sector_size as u64;
        if offset % ss != 0 || (buf.len() as u64) % ss != 0 {
            bail!(
                "unaligned read: offset={offset}, len={} (sector_size={ss})",
                buf.len()
            );
        }
        win32_seek(self.handle, offset)?;
        let mut bytes_read: u32 = 0;
        // SAFETY: handle is valid, buf pointer/len are valid, bytes_read is out-param.
        let ok = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut bytes_read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            bail!(
                "ReadFile failed at offset {offset}: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(bytes_read as usize)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize> {
        let ss = self.sector_size as u64;
        if offset % ss != 0 || (buf.len() as u64) % ss != 0 {
            bail!(
                "unaligned write: offset={offset}, len={} (sector_size={ss})",
                buf.len()
            );
        }
        win32_seek(self.handle, offset)?;
        let mut bytes_written: u32 = 0;
        // SAFETY: handle is valid, buf pointer/len are valid.
        let ok = unsafe {
            WriteFile(
                self.handle,
                buf.as_ptr(),
                buf.len() as u32,
                &mut bytes_written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            bail!(
                "WriteFile failed at offset {offset}: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(bytes_written as usize)
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn flush(&mut self) -> Result<()> {
        // SAFETY: handle is valid.
        let ok = unsafe { FlushFileBuffers(self.handle) };
        if ok == 0 {
            bail!("FlushFileBuffers failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Win32 helpers (each isolates one unsafe call)
// ---------------------------------------------------------------------------

/// Seek the file handle to `offset` using `SetFilePointerEx`.
fn win32_seek(handle: HANDLE, offset: u64) -> Result<()> {
    let mut new_pos: i64 = 0;
    // SAFETY: handle is valid, new_pos is an out-param.
    let ok = unsafe { SetFilePointerEx(handle, offset as i64, &mut new_pos, FILE_BEGIN) };
    if ok == 0 {
        bail!(
            "SetFilePointerEx({offset}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

/// Query the physical sector size via `IOCTL_DISK_GET_DRIVE_GEOMETRY`.
fn query_sector_size(handle: HANDLE, label: &str) -> Result<u32> {
    let mut geo: DISK_GEOMETRY = unsafe { std::mem::zeroed() };
    let mut bytes_returned: u32 = 0;
    // SAFETY: handle is valid, geo is a valid out-buffer of known size.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY,
            ptr::null(),
            0,
            &mut geo as *mut _ as *mut _,
            std::mem::size_of::<DISK_GEOMETRY>() as u32,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        bail!(
            "IOCTL_DISK_GET_DRIVE_GEOMETRY({label}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(geo.BytesPerSector)
}

/// Query total partition length via `IOCTL_DISK_GET_LENGTH_INFO`.
fn query_partition_length(handle: HANDLE, label: &str) -> Result<u64> {
    let mut info: GET_LENGTH_INFORMATION = unsafe { std::mem::zeroed() };
    let mut bytes_returned: u32 = 0;
    // SAFETY: handle is valid, info is a valid out-buffer.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            ptr::null(),
            0,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        bail!(
            "IOCTL_DISK_GET_LENGTH_INFO({label}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(info.Length as u64)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn round_trip_one_sector() {
        // Create a temporary file that will be cleaned up automatically.
        let tmp = NamedTempFile::new().expect("failed to create temp file");
        let path = tmp.path().to_path_buf();

        // 1 MiB backing file
        let create_size = 1024 * 1024;
        let mut dev =
            FileBlockDevice::open(&path, Some(create_size)).expect("open failed");

        assert_eq!(dev.size(), create_size);
        assert_eq!(dev.sector_size(), 512);

        // Build a recognisable 512-byte pattern.
        let mut pattern = [0u8; 512];
        for (i, byte) in pattern.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }

        // Write at offset 0.
        let written = dev.write(0, &pattern).expect("write failed");
        assert_eq!(written, 512);
        dev.flush().expect("flush failed");

        // Read back.
        let mut readback = [0u8; 512];
        let n = dev.read(0, &mut readback).expect("read failed");
        assert_eq!(n, 512);
        assert_eq!(readback, pattern);
    }

    #[test]
    fn unaligned_access_is_rejected() {
        let tmp = NamedTempFile::new().expect("failed to create temp file");
        let path = tmp.path().to_path_buf();

        let mut dev =
            FileBlockDevice::open(&path, Some(4096)).expect("open failed");

        // Unaligned offset
        let mut buf = [0u8; 512];
        assert!(dev.read(1, &mut buf).is_err());

        // Unaligned length
        let mut buf = [0u8; 100];
        assert!(dev.read(0, &mut buf).is_err());
    }
}
