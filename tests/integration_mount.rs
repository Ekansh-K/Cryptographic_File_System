/// 5L — WinFSP Integration Tests
///
/// These tests spawn the real `cfs-io.exe` binary, mount a CFS image as a
/// Windows drive letter, exercise it through `std::fs`, then unmount.
///
/// **Prerequisites:**
///   - WinFSP runtime installed (`C:\Program Files (x86)\WinFSP\`)
///   - Run as a user with permission to create drive letter mounts (typically
///     Administrator, or a user in the WinFSP allowed-mounts group)
///   - `LIBCLANG_PATH` set if building from source
///
/// Run all 5L tests with:
///   cargo test --test integration_mount -- --ignored --test-threads 1
///
/// `--test-threads 1` is required because each test uses the same drive
/// letter (T:).  Running in parallel would cause mount conflicts.

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Locate the `cfs-io.exe` binary built in the same profile as this test.
///
/// Integration test binary lives at:  `target/<profile>/deps/<name>-<hash>.exe`
/// Main binary lives at:              `target/<profile>/cfs-io.exe`
fn cfs_exe() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("cannot find current exe");
    // deps/ → profile dir (debug or release)
    let profile_dir = exe
        .parent() // deps/
        .unwrap()
        .parent() // debug/ or release/
        .unwrap();
    let bin = profile_dir.join("cfs-io.exe");
    assert!(
        bin.exists(),
        "cfs-io.exe not found at {}. Run `cargo build` first.",
        bin.display()
    );
    bin
}

/// Format a CFS image of the given size (e.g. "10M").
fn format_image(image: &Path, size: &str) {
    let status = Command::new(cfs_exe())
        .args(["format", image.to_str().unwrap(), size])
        .status()
        .expect("failed to spawn cfs-io format");
    assert!(status.success(), "cfs-io format failed");
}

/// Spawn `cfs-io mount <image> -m <drive>` as a background child.
fn mount_image(image: &Path, drive: &str) -> Child {
    Command::new(cfs_exe())
        .args(["mount", image.to_str().unwrap(), "-m", drive])
        .spawn()
        .expect("failed to spawn cfs-io mount")
}

/// Block until `<drive>\` is accessible or panic after 10 s.
fn wait_for_drive(drive: &str) {
    let root = format!("{}\\", drive);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if Path::new(&root).exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("drive {} did not appear within 10 seconds", drive);
}

/// Block until `<drive>\` is gone or panic after 10 s.
fn wait_for_drive_gone(drive: &str) {
    let root = format!("{}\\", drive);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !Path::new(&root).exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("drive {} did not disappear within 10 seconds", drive);
}

/// Kill the mount process and wait for the drive to disappear.
fn unmount(mut child: Child, drive: &str) {
    child.kill().expect("failed to kill mount process");
    let _ = child.wait();
    wait_for_drive_gone(drive);
}

const DRIVE: &str = "T:";

// ---------------------------------------------------------------------------
// 5L-1  Mount / Unmount lifecycle
// ---------------------------------------------------------------------------

/// Format an image, mount it, verify the drive appears, unmount, verify gone.
#[test]
#[ignore]
fn test_mount_unmount_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("lifecycle.img");
    format_image(&image, "10M");

    let child = mount_image(&image, DRIVE);
    wait_for_drive(DRIVE);

    // Drive must be accessible
    let root = format!("{}\\", DRIVE);
    assert!(Path::new(&root).exists(), "root should exist after mount");

    // Root directory should be empty after a fresh format
    let entries: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 0, "root should be empty after format");

    unmount(child, DRIVE);
    assert!(!Path::new(&root).exists(), "drive should be gone after unmount");
}

// ---------------------------------------------------------------------------
// 5L-2  Create / Read / Write via mount
// ---------------------------------------------------------------------------

/// Write files through std::fs, verify size and content are correct.
#[test]
#[ignore]
fn test_create_read_write_via_mount() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("crw.img");
    format_image(&image, "10M");

    let child = mount_image(&image, DRIVE);
    wait_for_drive(DRIVE);

    let file_path = format!("{}\\hello.txt", DRIVE);

    // Write
    std::fs::write(&file_path, b"Hello, CFS!").unwrap();

    // Read back
    let read_back = std::fs::read(&file_path).unwrap();
    assert_eq!(read_back, b"Hello, CFS!", "read-back mismatch");

    // File metadata
    let meta = std::fs::metadata(&file_path).unwrap();
    assert_eq!(meta.len(), 11, "file size mismatch");
    assert!(!meta.is_dir());

    // Overwrite (truncate + write new content)
    std::fs::write(&file_path, b"Updated content for CFS!").unwrap();
    let updated = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(updated, "Updated content for CFS!");

    unmount(child, DRIVE);
}

// ---------------------------------------------------------------------------
// 5L-3  Directory operations via mount
// ---------------------------------------------------------------------------

/// mkdir / list / rmdir exercised through std::fs.
#[test]
#[ignore]
fn test_directory_ops_via_mount() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("dirs.img");
    format_image(&image, "10M");

    let child = mount_image(&image, DRIVE);
    wait_for_drive(DRIVE);

    let subdir = format!("{}\\mydir", DRIVE);
    let nested = format!("{}\\mydir\\inner", DRIVE);
    let file_in_dir = format!("{}\\mydir\\file.txt", DRIVE);

    std::fs::create_dir(&subdir).unwrap();
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(&file_in_dir, b"inside mydir").unwrap();

    // Listing
    let entries: Vec<String> = std::fs::read_dir(format!("{}\\", DRIVE))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    assert!(entries.contains(&"mydir".to_string()), "mydir missing from listing");

    // Verify nested
    let meta = std::fs::metadata(&nested).unwrap();
    assert!(meta.is_dir());

    // Remove directory tree
    std::fs::remove_file(&file_in_dir).unwrap();
    std::fs::remove_dir(&nested).unwrap();
    std::fs::remove_dir(&subdir).unwrap();

    // Root should be empty again
    let root_entries: Vec<_> = std::fs::read_dir(format!("{}\\", DRIVE))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(root_entries.len(), 0, "root should be empty after cleanup");

    unmount(child, DRIVE);
}

// ---------------------------------------------------------------------------
// 5L-4  Large file (>40 KB, exercises indirect block pointers)
// ---------------------------------------------------------------------------

/// Write a 100 KB file through std::fs, read it back, verify byte-for-byte.
#[test]
#[ignore]
fn test_large_file_via_mount() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("large.img");
    format_image(&image, "20M");

    let child = mount_image(&image, DRIVE);
    wait_for_drive(DRIVE);

    let file_path = format!("{}\\large.bin", DRIVE);

    // Build a 100 KB pattern
    let pattern: Vec<u8> = (0u32..102_400).map(|i| (i % 256) as u8).collect();

    std::fs::write(&file_path, &pattern).unwrap();

    // Check reported size
    let meta = std::fs::metadata(&file_path).unwrap();
    assert_eq!(meta.len(), 102_400, "large file size mismatch");

    // Read back and verify
    let read_back = std::fs::read(&file_path).unwrap();
    assert_eq!(read_back.len(), 102_400);
    assert_eq!(
        read_back, pattern,
        "large file content mismatch — indirect block I/O broken"
    );

    unmount(child, DRIVE);
}

// ---------------------------------------------------------------------------
// 5L-5  Rename / move via mount
// ---------------------------------------------------------------------------

/// Rename files and directories through std::fs.
#[test]
#[ignore]
fn test_rename_via_mount() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("rename.img");
    format_image(&image, "10M");

    let child = mount_image(&image, DRIVE);
    wait_for_drive(DRIVE);

    let original = format!("{}\\original.txt", DRIVE);
    let renamed = format!("{}\\renamed.txt", DRIVE);
    let subdir = format!("{}\\subdir", DRIVE);
    let moved = format!("{}\\subdir\\moved.txt", DRIVE);

    // Rename file in same directory
    std::fs::write(&original, b"rename test").unwrap();
    std::fs::rename(&original, &renamed).unwrap();
    assert!(!Path::new(&original).exists(), "original should be gone after rename");
    assert_eq!(std::fs::read(&renamed).unwrap(), b"rename test");

    // Move file to subdirectory
    std::fs::create_dir(&subdir).unwrap();
    std::fs::rename(&renamed, &moved).unwrap();
    assert!(!Path::new(&renamed).exists());
    assert_eq!(std::fs::read(&moved).unwrap(), b"rename test");

    // Rename a directory
    let dir_a = format!("{}\\dira", DRIVE);
    let dir_b = format!("{}\\dirb", DRIVE);
    std::fs::create_dir(&dir_a).unwrap();
    std::fs::rename(&dir_a, &dir_b).unwrap();
    assert!(!Path::new(&dir_a).exists());
    assert!(std::fs::metadata(&dir_b).unwrap().is_dir());

    unmount(child, DRIVE);
}

// ---------------------------------------------------------------------------
// 5L-6  Persistence across remount
// ---------------------------------------------------------------------------

/// Write files, unmount, remount, verify data survived.
#[test]
#[ignore]
fn test_persistence_across_remount() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("persist.img");
    format_image(&image, "10M");

    // First mount — write data
    {
        let child = mount_image(&image, DRIVE);
        wait_for_drive(DRIVE);

        std::fs::write(format!("{}\\text.txt", DRIVE), b"persistent content").unwrap();
        std::fs::create_dir(format!("{}\\keeper", DRIVE)).unwrap();
        std::fs::write(format!("{}\\keeper\\inner.bin", DRIVE), b"binary data \x00\x01\x02\x03").unwrap();

        // Build 8 KB pattern
        let pattern: Vec<u8> = (0u16..8192).map(|i| (i % 256) as u8).collect();
        std::fs::write(format!("{}\\pattern.bin", DRIVE), &pattern).unwrap();

        unmount(child, DRIVE);
    }

    // Second mount — verify data
    {
        let child = mount_image(&image, DRIVE);
        wait_for_drive(DRIVE);

        // Text file
        let txt = std::fs::read(format!("{}\\text.txt", DRIVE)).unwrap();
        assert_eq!(txt, b"persistent content", "text file content mismatch after remount");

        // Nested binary file
        let bin = std::fs::read(format!("{}\\keeper\\inner.bin", DRIVE)).unwrap();
        assert_eq!(bin, b"binary data \x00\x01\x02\x03", "binary file mismatch after remount");

        // Pattern file
        let read_pattern = std::fs::read(format!("{}\\pattern.bin", DRIVE)).unwrap();
        let expected: Vec<u8> = (0u16..8192).map(|i| (i % 256) as u8).collect();
        assert_eq!(read_pattern, expected, "pattern file integrity failure after remount");

        // Directory structure
        let entries: Vec<String> = std::fs::read_dir(format!("{}\\", DRIVE))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        assert!(entries.contains(&"text.txt".to_string()));
        assert!(entries.contains(&"keeper".to_string()));
        assert!(entries.contains(&"pattern.bin".to_string()));

        unmount(child, DRIVE);
    }
}
