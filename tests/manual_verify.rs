#![allow(clippy::field_reassign_with_default)]
use anyhow::Result;
use cfs_io::block_device::FileBlockDevice;
use cfs_io::crypto::{EncryptedBlockDevice, KdfAlgorithm, KdfParams, CryptoHeader};
use cfs_io::volume::{CFSVolume, FormatOptions};
use std::fs;

#[test]
fn test_manual() -> Result<()> {
    let path = "cfs_manual_test.img";
    let _ = fs::remove_file(path);

    let mut opts = FormatOptions::default();
    opts.enable_aead = true;
    let kdf = KdfParams {
        algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
        pbkdf2_iterations: 100_000,
        argon2_memory_kib: 0,
        argon2_time_cost: 0,
        argon2_parallelism: 0,
    };

    let path = std::path::Path::new("cfs_manual_test.img");
    let dev = FileBlockDevice::open(path, Some(4 * 1024 * 1024))?;
    let enc = EncryptedBlockDevice::format_encrypted(
        Box::new(dev),
        b"mysecretpw",
        &kdf,
        opts.block_size,
        true,
    )?;

    println!("Formatted with AEAD enabled!");
    
    let vol = CFSVolume::format_v3(Box::new(enc), &opts)?;
    vol.mkdir("/testdir")?;
    vol.create_file("/testdir/file.txt")?;
    vol.write_file("/testdir/file.txt", 0, b"Hello AEAD!")?;
    vol.sync()?;
    drop(vol);

    println!("Volume created, written and unmounted successfully.");

    // Now test slot commands
    let mut dev2 = FileBlockDevice::open(path, None)?;
    let hdr = CryptoHeader::read_from(&mut dev2, b"mysecretpw", 4096)?;
    
    println!("Read crypto header. Slots:");
    for (i, slot) in hdr.slots.iter().enumerate() {
        println!("  Slot {}: state={}", i, slot.state);
    }
    
    Ok(())
}
