use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;
use cfs_io::block_device::FileBlockDevice;
use cfs_io::crypto::aead::AeadCipher;
use cfs_io::crypto::key::{benchmark_kdf, KdfAlgorithm, KdfParams};
use cfs_io::crypto::xts::XtsCipher;
use cfs_io::volume::{CFSVolume, FormatOptions};
use rand::RngCore;

fn main() {
    println!("==================================================");
    println!("  CFS Comprehensive Benchmark Suite (Windows)   ");
    println!("==================================================");

    let mut json_out = String::from("{\n");

    // -------------------------------------------------------------------------
    // 1. Filesystem I/O Scaling across File Sizes
    // -------------------------------------------------------------------------
    println!("\n[Section 1] Filesystem I/O Scaling across File Sizes...");
    let io_sizes = [
        (4 * 1024, "4 KiB"),
        (64 * 1024, "64 KiB"),
        (256 * 1024, "256 KiB"),
        (1024 * 1024, "1 MiB"),
        (4 * 1024 * 1024, "4 MiB"),
        (16 * 1024 * 1024, "16 MiB"),
        (32 * 1024 * 1024, "32 MiB"),
        (64 * 1024 * 1024, "64 MiB"),
    ];

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("cfs_bench_all_io.img");
    let _ = std::fs::remove_file(&tmp_path);

    let vol_size: u64 = 160 * 1024 * 1024; // 160 MiB volume
    let dev = FileBlockDevice::open(&tmp_path, Some(vol_size)).expect("Failed to create file device");
    let vol = CFSVolume::format_v3(Box::new(dev), &FormatOptions::default()).expect("Failed to format");
    let vol = Arc::new(vol);

    let bench_path = "/__cfs_bench_io";
    let max_chunk = 4 * 1024 * 1024;
    let chunk = vec![0xAAu8; max_chunk];

    json_out.push_str("  \"io_scaling\": [\n");

    for (idx, &(size_bytes, label)) in io_sizes.iter().enumerate() {
        let _ = vol.delete_file(bench_path);
        vol.create_file(bench_path).expect("create_file failed");

        let chunk_size = (size_bytes as usize).min(max_chunk);

        // Write
        let t0 = Instant::now();
        let mut offset = 0;
        while offset < size_bytes {
            let rem = (size_bytes - offset) as usize;
            let to_write = rem.min(chunk_size);
            vol.write_file(bench_path, offset, &chunk[..to_write]).expect("write failed");
            offset += to_write as u64;
        }
        let write_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // Sync
        let ts = Instant::now();
        vol.sync().expect("sync failed");
        let sync_ms = ts.elapsed().as_secs_f64() * 1000.0;

        // Read
        let t1 = Instant::now();
        let mut offset = 0;
        while offset < size_bytes {
            let rem = (size_bytes - offset) as usize;
            let to_read = rem.min(chunk_size) as u64;
            let _ = vol.read_file(bench_path, offset, to_read).expect("read failed");
            offset += to_read;
        }
        let read_ms = t1.elapsed().as_secs_f64() * 1000.0;

        let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
        let write_speed = if write_ms > 0.0 { size_mb / (write_ms / 1000.0) } else { 0.0 };
        let read_speed = if read_ms > 0.0 { size_mb / (read_ms / 1000.0) } else { 0.0 };

        println!("  {:<8} | Write: {:6.1} MiB/s ({:6.2} ms) | Read: {:6.1} MiB/s ({:6.2} ms) | Sync: {:5.2} ms",
            label, write_speed, write_ms, read_speed, read_ms, sync_ms);

        let comma = if idx + 1 < io_sizes.len() { "," } else { "" };
        json_out.push_str(&format!(
            "    {{\"label\": \"{}\", \"size_bytes\": {}, \"write_mbps\": {:.2}, \"read_mbps\": {:.2}, \"sync_ms\": {:.2}}}{}\n",
            label, size_bytes, write_speed, read_speed, sync_ms, comma
        ));
    }
    json_out.push_str("  ],\n");

    let _ = vol.delete_file(bench_path);
    drop(vol);
    let _ = std::fs::remove_file(&tmp_path);

    // -------------------------------------------------------------------------
    // 2A. I/O Performance across Block Sizes & File Sizes (5-Run Average)
    // -------------------------------------------------------------------------
    println!("\n[Section 2A] I/O Performance Matrix (4 KiB, 16 KiB, 64 KiB Block Sizes — 5-Run Avg)...");
    let block_sizes_kb = [4, 16, 64];
    let matrix_file_sizes = [
        (4 * 1024, "4 KiB"),
        (1024 * 1024, "1 MiB"),
        (16 * 1024 * 1024, "16 MiB"),
        (128 * 1024 * 1024, "128 MiB"),
    ];

    json_out.push_str("  \"io_matrix\": [\n");
    let total_matrix_entries = block_sizes_kb.len() * matrix_file_sizes.len();
    let mut current_entry = 0;

    for &bs_kb in &block_sizes_kb {
        let bs_bytes = bs_kb * 1024;
        let mut opts = FormatOptions::general_purpose();
        opts.block_size = bs_bytes as u32;
        opts.inode_ratio = 4096; // keep plenty of inodes
        opts.journal_percent = 5.0; // max allowed journal size

        let tmp_path_m = tmp_dir.join(format!("cfs_bench_matrix_{}k.img", bs_kb));
        let _ = std::fs::remove_file(&tmp_path_m);
        let dev = FileBlockDevice::open(&tmp_path_m, Some(300 * 1024 * 1024)).expect("open failed");
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).expect("format failed");
        let vol = Arc::new(vol);

        for &(size_bytes, label) in &matrix_file_sizes {
            let bench_path = "/__cfs_bench_matrix";
            let mut write_speeds = Vec::with_capacity(5);
            let mut read_speeds = Vec::with_capacity(5);
            let mut sync_times = Vec::with_capacity(5);

            for _run in 0..5 {
                let _ = vol.delete_file(bench_path);
                let _ = vol.sync();
                vol.create_file(bench_path).expect("create_file failed");

                let chunk_size = (size_bytes as usize).min(4 * 1024 * 1024);
                let chunk = vec![0xCCu8; chunk_size];

                // Write
                let t0 = Instant::now();
                let mut offset = 0;
                while offset < size_bytes {
                    let rem = (size_bytes - offset) as usize;
                    let to_write = rem.min(chunk_size);
                    vol.write_file(bench_path, offset, &chunk[..to_write]).expect("write failed");
                    offset += to_write as u64;
                }
                let write_ms = t0.elapsed().as_secs_f64() * 1000.0;

                // Sync
                let ts = Instant::now();
                vol.sync().expect("sync failed");
                let sync_ms = ts.elapsed().as_secs_f64() * 1000.0;

                // Read
                let t1 = Instant::now();
                let mut offset = 0;
                while offset < size_bytes {
                    let rem = (size_bytes - offset) as usize;
                    let to_read = rem.min(chunk_size) as u64;
                    let _ = vol.read_file(bench_path, offset, to_read).expect("read failed");
                    offset += to_read;
                }
                let read_ms = t1.elapsed().as_secs_f64() * 1000.0;

                let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
                let w_spd = if write_ms > 0.0 { size_mb / (write_ms / 1000.0) } else { 0.0 };
                let r_spd = if read_ms > 0.0 { size_mb / (read_ms / 1000.0) } else { 0.0 };

                write_speeds.push(w_spd);
                read_speeds.push(r_spd);
                sync_times.push(sync_ms);
            }

            let avg_write: f64 = write_speeds.iter().sum::<f64>() / 5.0;
            let avg_read: f64 = read_speeds.iter().sum::<f64>() / 5.0;
            let avg_sync: f64 = sync_times.iter().sum::<f64>() / 5.0;

            println!("  BS: {:2} KiB | File: {:7} | Write: {:6.1} MiB/s | Read: {:6.1} MiB/s | Sync: {:5.2} ms (5-run avg)",
                bs_kb, label, avg_write, avg_read, avg_sync);

            current_entry += 1;
            let comma = if current_entry < total_matrix_entries { "," } else { "" };
            json_out.push_str(&format!(
                "    {{\"block_size_kb\": {}, \"file_size_label\": \"{}\", \"file_size_bytes\": {}, \"write_mbps\": {:.2}, \"read_mbps\": {:.2}, \"sync_ms\": {:.2}}}{}\n",
                bs_kb, label, size_bytes, avg_write, avg_read, avg_sync, comma
            ));
        }

        let _ = vol.delete_file("/__cfs_bench_matrix");
        drop(vol);
        let _ = std::fs::remove_file(&tmp_path_m);
    }
    json_out.push_str("  ],\n");

    // -------------------------------------------------------------------------
    // 2B. Multiple Small Files Benchmark (500 Files across Varying Block Sizes)
    // -------------------------------------------------------------------------
    println!("\n[Section 2B] Multiple Small Files Benchmark (500 Files × 4 KiB — 5-Run Avg)...");
    json_out.push_str("  \"small_files\": [\n");

    let num_files = 500;
    let file_data = vec![0xDDu8; 4096]; // 4 KiB per file

    for (idx, &bs_kb) in block_sizes_kb.iter().enumerate() {
        let bs_bytes = bs_kb * 1024;
        let mut opts = FormatOptions::general_purpose();
        opts.block_size = bs_bytes as u32;
        opts.inode_ratio = 4096; // ensure plenty of inodes for 500 files
        opts.journal_percent = 5.0; // max allowed journal size

        let tmp_path_sf = tmp_dir.join(format!("cfs_bench_sf_{}k.img", bs_kb));
        let _ = std::fs::remove_file(&tmp_path_sf);
        let dev = FileBlockDevice::open(&tmp_path_sf, Some(1000 * 1024 * 1024)).expect("open failed");
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).expect("format failed");
        let vol = Arc::new(vol);

        let mut write_times_ms = Vec::with_capacity(5);
        let mut read_times_ms = Vec::with_capacity(5);

        for _run in 0..5 {
            // Clean up existing files if any
            for f in 0..num_files {
                let _ = vol.delete_file(&format!("/sf_{}", f));
            }
            let _ = vol.sync();

            // Write 500 files
            let t0 = Instant::now();
            for f in 0..num_files {
                let path = format!("/sf_{}", f);
                vol.create_file(&path).expect("create_file sf failed");
                vol.write_file(&path, 0, &file_data).expect("write sf failed");
            }
            write_times_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

            // Read 500 files
            let t1 = Instant::now();
            for f in 0..num_files {
                let path = format!("/sf_{}", f);
                let _ = vol.read_file(&path, 0, 4096).expect("read sf failed");
            }
            read_times_ms.push(t1.elapsed().as_secs_f64() * 1000.0);
        }

        let avg_w_ms: f64 = write_times_ms.iter().sum::<f64>() / 5.0;
        let avg_r_ms: f64 = read_times_ms.iter().sum::<f64>() / 5.0;
        let w_fps = if avg_w_ms > 0.0 { (num_files as f64) / (avg_w_ms / 1000.0) } else { 0.0 };
        let r_fps = if avg_r_ms > 0.0 { (num_files as f64) / (avg_r_ms / 1000.0) } else { 0.0 };

        println!("  BS: {:2} KiB | Write 500 files: {:6.1} ms ({:6.0} files/s) | Read 500 files: {:6.1} ms ({:6.0} files/s)",
            bs_kb, avg_w_ms, w_fps, avg_r_ms, r_fps);

        let comma = if idx + 1 < block_sizes_kb.len() { "," } else { "" };
        json_out.push_str(&format!(
            "    {{\"block_size_kb\": {}, \"write_ms\": {:.2}, \"read_ms\": {:.2}, \"write_fps\": {:.2}, \"read_fps\": {:.2}}}{}\n",
            bs_kb, avg_w_ms, avg_r_ms, w_fps, r_fps, comma
        ));

        for f in 0..num_files {
            let _ = vol.delete_file(&format!("/sf_{}", f));
        }
        drop(vol);
        let _ = std::fs::remove_file(&tmp_path_sf);
    }
    json_out.push_str("  ],\n");

    // -------------------------------------------------------------------------
    // 3. Cryptographic Engine Speed (RAM only, AES-XTS vs AES-XTS+AEAD)
    // -------------------------------------------------------------------------
    println!("\n[Section 3] Cryptographic Engine Throughput (AES-XTS vs AES-XTS+AEAD)...");
    let crypto_sizes = [
        (16, "16 MiB"),
        (32, "32 MiB"),
        (64, "64 MiB"),
        (128, "128 MiB"),
        (256, "256 MiB"),
        (512, "512 MiB"),
    ];

    let mut xts_key = [0u8; 64];
    rand::rngs::OsRng.fill_bytes(&mut xts_key);
    let mut tag_key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut tag_key);

    // Test AEAD at three block-unit sizes to show granularity impact
    let aead_units: &[(usize, &str)] = &[
        (4 * 1024,  "4 KiB EU"),
        (16 * 1024, "16 KiB EU"),
        (64 * 1024, "64 KiB EU"),
    ];

    json_out.push_str("  \"crypto_speed\": [\n");

    for (idx, &(size_mb, label)) in crypto_sizes.iter().enumerate() {
        let size_bytes = size_mb * 1024 * 1024;
        let mut buf = vec![0u8; size_bytes as usize];
        rand::rngs::OsRng.fill_bytes(&mut buf);

        let cipher_4k  = XtsCipher::new(&xts_key, 4 * 1024);
        let _cipher_16k = XtsCipher::new(&xts_key, 16 * 1024);

        // XTS Encrypt (4 KiB EU — matches default)
        let t0 = Instant::now();
        cipher_4k.encrypt_blocks_parallel(0, &mut buf);
        let xts_enc_secs = t0.elapsed().as_secs_f64();

        // XTS Decrypt
        let t1 = Instant::now();
        cipher_4k.decrypt_blocks_parallel(0, &mut buf);
        let xts_dec_secs = t1.elapsed().as_secs_f64();

        // Re-encrypt for AEAD tests
        cipher_4k.encrypt_blocks_parallel(0, &mut buf);

        let mb = size_mb as f64;
        let xts_enc = if xts_enc_secs > 0.0 { mb / xts_enc_secs } else { 0.0 };
        let xts_dec = if xts_dec_secs > 0.0 { mb / xts_dec_secs } else { 0.0 };

        // AEAD parallel tag computation at each block-unit size
        let mut aead_results: Vec<(String, f64, f64)> = Vec::new();
        for &(eu, eu_label) in aead_units {
            let aead_cipher = AeadCipher::with_block_unit(&tag_key, eu);
            let n_blocks = size_bytes as usize / eu;

            // Compute tags in parallel
            let t2 = Instant::now();
            let tags = aead_cipher.compute_tags_parallel(0, &buf);
            let aead_enc_secs = t2.elapsed().as_secs_f64();

            // Verify tags in parallel
            let t3 = Instant::now();
            let _ = aead_cipher.verify_tags_parallel(0, &buf, &tags);
            let aead_dec_secs = t3.elapsed().as_secs_f64();

            let aead_enc = if aead_enc_secs > 0.0 { mb / aead_enc_secs } else { 0.0 };
            let aead_dec = if aead_dec_secs > 0.0 { mb / aead_dec_secs } else { 0.0 };
            let overhead_enc = if xts_enc > 0.0 { (xts_enc - aead_enc) / xts_enc * 100.0 } else { 0.0 };
            let overhead_dec = if xts_dec > 0.0 { (xts_dec - aead_dec) / xts_dec * 100.0 } else { 0.0 };

            println!("  {:<8} | EU={:<8} | n_tags={:<6} | AEAD Enc: {:6.1} MiB/s (-{:.1}%) | AEAD Dec: {:6.1} MiB/s (-{:.1}%)",
                label, eu_label, n_blocks, aead_enc, overhead_enc, aead_dec, overhead_dec);

            aead_results.push((eu_label.to_string(), aead_enc, aead_dec));
        }

        println!("  {:<8} | XTS Enc: {:6.1} MiB/s | XTS Dec: {:6.1} MiB/s",
            label, xts_enc, xts_dec);

        let comma = if idx + 1 < crypto_sizes.len() { "," } else { "" };
        let aead_4k  = &aead_results[0];
        let aead_16k = &aead_results[1];
        let aead_64k = &aead_results[2];
        let overhead_enc_4k  = if xts_enc > 0.0 { (xts_enc - aead_4k.1)  / xts_enc * 100.0 } else { 0.0 };
        let overhead_dec_4k  = if xts_dec > 0.0 { (xts_dec - aead_4k.2)  / xts_dec * 100.0 } else { 0.0 };
        let overhead_enc_16k = if xts_enc > 0.0 { (xts_enc - aead_16k.1) / xts_enc * 100.0 } else { 0.0 };
        let overhead_dec_16k = if xts_dec > 0.0 { (xts_dec - aead_16k.2) / xts_dec * 100.0 } else { 0.0 };
        let overhead_enc_64k = if xts_enc > 0.0 { (xts_enc - aead_64k.1) / xts_enc * 100.0 } else { 0.0 };
        let overhead_dec_64k = if xts_dec > 0.0 { (xts_dec - aead_64k.2) / xts_dec * 100.0 } else { 0.0 };

        json_out.push_str(&format!(
            "    {{\"size_mb\": {}, \"xts_enc_mbps\": {:.2}, \"xts_dec_mbps\": {:.2}, \
             \"aead_enc_4k_mbps\": {:.2}, \"aead_dec_4k_mbps\": {:.2}, \"overhead_enc_4k_pct\": {:.2}, \"overhead_dec_4k_pct\": {:.2}, \
             \"aead_enc_16k_mbps\": {:.2}, \"aead_dec_16k_mbps\": {:.2}, \"overhead_enc_16k_pct\": {:.2}, \"overhead_dec_16k_pct\": {:.2}, \
             \"aead_enc_64k_mbps\": {:.2}, \"aead_dec_64k_mbps\": {:.2}, \"overhead_enc_64k_pct\": {:.2}, \"overhead_dec_64k_pct\": {:.2}}}{}\n",
            size_mb, xts_enc, xts_dec,
            aead_4k.1,  aead_4k.2,  overhead_enc_4k,  overhead_dec_4k,
            aead_16k.1, aead_16k.2, overhead_enc_16k, overhead_dec_16k,
            aead_64k.1, aead_64k.2, overhead_enc_64k, overhead_dec_64k,
            comma
        ));
    }
    json_out.push_str("  ],\n");

    // -------------------------------------------------------------------------
    // 4. KDF Unlock Latency

    // -------------------------------------------------------------------------
    println!("\n[Section 4] KDF Derivation / Volume Unlock Latency...");
    let kdfs = [
        ("Argon2id (16 MiB, t=1, p=1)", KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        }),
        ("Argon2id (32 MiB, t=2, p=2)", KdfParams::default_argon2id()),
        ("Argon2id (64 MiB, t=2, p=4)", KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 64 * 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 4,
        }),
        ("Argon2id (64 MiB, t=2, p=12)", KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 64 * 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 12,
        }),
        ("Argon2id (128 MiB, t=2, p=12)", KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 128 * 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 12,
        }),
        ("Argon2id (256 MiB, t=2, p=12)", KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 256 * 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 12,
        }),
        ("PBKDF2-SHA256 (100k)", KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 100_000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }),
        ("PBKDF2-SHA256 (300k)", KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 300_000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }),
        ("PBKDF2-SHA256 (600k)", KdfParams::default_pbkdf2()),
        ("PBKDF2-SHA512 (100k)", KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha512,
            pbkdf2_iterations: 100_000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }),
        ("PBKDF2-SHA512 (300k)", KdfParams::default_pbkdf2_sha512()),
    ];

    json_out.push_str("  \"kdf_unlock\": [\n");

    for (idx, (label, params)) in kdfs.iter().enumerate() {
        let dur = benchmark_kdf(params).expect("kdf bench failed");
        let ms = dur.as_secs_f64() * 1000.0;
        println!("  {:<28} | Derivation Time: {:6.2} ms ({:.2} s)", label, ms, dur.as_secs_f64());

        let comma = if idx + 1 < kdfs.len() { "," } else { "" };
        json_out.push_str(&format!(
            "    {{\"algo\": \"{}\", \"time_ms\": {:.2}}}{}\n",
            label, ms, comma
        ));
    }
    json_out.push_str("  ]\n");
    json_out.push_str("}\n");

    // Write JSON output to file
    let json_path = "benchmark_results.json";
    let mut file = File::create(json_path).expect("Failed to create JSON file");
    file.write_all(json_out.as_bytes()).expect("Failed to write JSON");
    println!("\nAll benchmarks completed successfully! Results saved to {}", json_path);
}
