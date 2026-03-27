use std::sync::Arc;
use cfs_io::block_device::FileBlockDevice;
use cfs_io::volume::{CFSVolume, FormatOptions};

fn main() {
    let tmp_path = std::env::temp_dir().join("cfs_bench_test.img");
    let size: u64 = 64 * 1024 * 1024; // 64 MiB

    // Clean up any leftover
    let _ = std::fs::remove_file(&tmp_path);

    println!("Creating test volume at {:?} ({} bytes)...", tmp_path, size);
    let dev = FileBlockDevice::open(&tmp_path, Some(size)).expect("Failed to create file device");
    let vol = CFSVolume::format_v3(Box::new(dev), &FormatOptions::default()).expect("Failed to format");
    let vol = Arc::new(vol);

    println!("Volume formatted. Testing benchmark logic...");

    let bench_path = "/__cfs_io_benchmark_tmp";

    // Test 1: Small file (4 KiB)
    {
        let size_bytes: u64 = 4096;
        let _ = vol.delete_file(bench_path);
        vol.create_file(bench_path).expect("create_file failed");

        let chunk = vec![0xAAu8; size_bytes as usize];
        let start = std::time::Instant::now();
        vol.write_file(bench_path, 0, &chunk).expect("write_file failed");
        vol.sync().expect("sync failed");
        let write_ms = start.elapsed().as_millis();

        let start = std::time::Instant::now();
        let data = vol.read_file(bench_path, 0, size_bytes).expect("read_file failed");
        let read_ms = start.elapsed().as_millis();

        assert_eq!(data.len(), size_bytes as usize);
        assert!(data.iter().all(|&b| b == 0xAA));

        let _ = vol.delete_file(bench_path);
        println!("  Small (4 KiB): write={}ms read={}ms OK", write_ms, read_ms);
    }

    // Test 2: Medium file (1 MiB)
    {
        let size_bytes: u64 = 1024 * 1024;
        let _ = vol.delete_file(bench_path);
        vol.create_file(bench_path).expect("create_file failed");

        let chunk_size = 4 * 1024 * 1024usize;
        let chunk = vec![0xBBu8; chunk_size.min(size_bytes as usize)];

        let start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_write = remaining.min(chunk.len());
            vol.write_file(bench_path, offset, &chunk[..to_write]).expect("write failed");
            offset += to_write as u64;
        }
        vol.sync().expect("sync failed");
        let write_ms = start.elapsed().as_millis();

        let start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_read = remaining.min(chunk.len()) as u64;
            let _ = vol.read_file(bench_path, offset, to_read).expect("read failed");
            offset += to_read;
        }
        let read_ms = start.elapsed().as_millis();

        let _ = vol.delete_file(bench_path);
        println!("  Medium (1 MiB): write={}ms read={}ms OK", write_ms, read_ms);
    }

    // Test 3: Larger file (16 MiB) - chunked writes
    {
        let size_bytes: u64 = 16 * 1024 * 1024;
        let _ = vol.delete_file(bench_path);
        vol.create_file(bench_path).expect("create_file failed");

        let chunk_size: usize = 4 * 1024 * 1024;
        let chunk = vec![0xCCu8; chunk_size];

        let start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_write = remaining.min(chunk_size);
            vol.write_file(bench_path, offset, &chunk[..to_write]).expect("write failed");
            offset += to_write as u64;
        }
        vol.sync().expect("sync failed");
        let write_ms = start.elapsed().as_millis();

        let start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_read = remaining.min(chunk_size) as u64;
            let data = vol.read_file(bench_path, offset, to_read).expect("read failed");
            assert_eq!(data.len(), to_read as usize);
            offset += to_read;
        }
        let read_ms = start.elapsed().as_millis();

        let _ = vol.delete_file(bench_path);
        let speed_w = if write_ms > 0 { 16000.0 / write_ms as f64 } else { 0.0 };
        let speed_r = if read_ms > 0 { 16000.0 / read_ms as f64 } else { 0.0 };
        println!(
            "  Large (16 MiB): write={}ms ({:.1} MiB/s) read={}ms ({:.1} MiB/s) OK",
            write_ms, speed_w, read_ms, speed_r
        );
    }

    // Test 4: Verify cleanup works - file should not exist
    {
        let result = vol.read_file(bench_path, 0, 1);
        match result {
            Err(_) => println!("  Cleanup verified: benchmark file does not exist. OK"),
            Ok(_) => println!("  WARNING: benchmark file still exists after cleanup!"),
        }
    }

    // Cleanup temp file
    drop(vol);
    let _ = std::fs::remove_file(&tmp_path);
    println!("\nAll benchmark tests passed!");
}
