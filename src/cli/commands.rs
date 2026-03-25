use anyhow::{bail, Result};
use std::io::Write;

use crate::crypto::{self, EncryptedBlockDevice, CryptoHeader, KdfAlgorithm, KdfParams};
use crate::volume::{CFSVolume, FormatOptions, ErrorBehavior, CFS_VERSION_V3, INODE_DIR, INODE_FILE, INODE_SYMLINK};
use crate::volume::superblock::*;

use super::{format_mode, format_size_human, format_timestamp, format_timestamp_ns, format_permissions, is_raw_device_path, open_device, open_volume, auto_open_volume, parse_size};

// ---------------------------------------------------------------------------
// 4B Ã¢â‚¬â€ cfs format
// ---------------------------------------------------------------------------

pub fn cmd_format(image: &str, size_str: &str, encrypted: bool, kdf_params: &KdfParams, format_opts: &FormatOptions) -> Result<()> {
    let block_size = format_opts.block_size;
    let dev = if is_raw_device_path(image) {
        open_device(image, None)?
    } else {
        let size = parse_size(size_str)?;
        open_device(image, Some(size))?
    };
    let size = dev.size();

    let kdf_name = match kdf_params.algorithm {
        KdfAlgorithm::Pbkdf2HmacSha256 => "PBKDF2-HMAC-SHA256",
        KdfAlgorithm::Argon2id => "Argon2id",
    };

    let vol = if encrypted {
        let password = super::prompt_password("New password: ")?;
        let confirm = super::prompt_password("Confirm password: ")?;
        if password != confirm {
            bail!("passwords do not match");
        }
        let enc = EncryptedBlockDevice::format_encrypted(
            dev,
            password.as_bytes(),
            kdf_params,
            block_size,
        )?;
        CFSVolume::format_v3(Box::new(enc), format_opts)?
    } else {
        CFSVolume::format_v3(dev, format_opts)?
    };

    let sb = vol.superblock();
    println!("Formatted {image}{}", if encrypted { " (encrypted)" } else { "" });
    if encrypted {
        println!("  KDF:              {kdf_name}");
    }
    println!("  Version:          {}", sb.version);
    println!("  Size:             {size} bytes ({})", format_size_human(size));
    println!("  Block size:       {block_size}");
    println!("  Inode size:       {} bytes", sb.inode_size);
    println!("  Total blocks:     {}", sb.total_blocks);
    println!("  Free blocks:      {}", sb.free_blocks);
    println!("  Inode count:      {}", sb.inode_count);
    if sb.version >= CFS_VERSION_V3 && sb.group_count > 0 {
        println!("  Block groups:     {}", sb.group_count);
        println!("  Blocks/group:     {}", sb.blocks_per_group);
        println!("  Inodes/group:     {}", sb.inodes_per_group);
    }
    if sb.journal_blocks > 0 {
        let journal_size = sb.journal_blocks * sb.block_size as u64;
        println!("  Journal blocks:   {} ({})", sb.journal_blocks, format_size_human(journal_size));
    }
    let label = sb.label();
    if !label.is_empty() {
        println!("  Label:            {label}");
    }

    Ok(())
}

/// Format with explicit password for testing (no interactive prompt).
pub fn cmd_format_with_password(
    image: &str,
    size_str: &str,
    password: &str,
    kdf_params: &KdfParams,
    format_opts: &FormatOptions,
) -> Result<()> {
    let block_size = format_opts.block_size;
    let dev = if is_raw_device_path(image) {
        open_device(image, None)?
    } else {
        let size = parse_size(size_str)?;
        open_device(image, Some(size))?
    };
    let enc = EncryptedBlockDevice::format_encrypted(
        dev,
        password.as_bytes(),
        kdf_params,
        block_size,
    )?;
    let _vol = CFSVolume::format_v3(Box::new(enc), format_opts)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 4C Ã¢â‚¬â€ cfs info
// ---------------------------------------------------------------------------

pub fn cmd_info(image: &str, block_size: u32) -> Result<()> {
    // Check for encrypted header and display KDF info before opening volume
    let mut dev = open_device(image, None)?;
    let is_encrypted = crypto::is_encrypted_device(&mut *dev)?;
    let mut kdf_info: Option<(String, String)> = None;
    if is_encrypted {
        if let Ok(hdr) = CryptoHeader::read_from(&mut *dev, block_size) {
            let params = hdr.kdf_params();
            let algo = match params.algorithm {
                KdfAlgorithm::Pbkdf2HmacSha256 => "PBKDF2-HMAC-SHA256".to_string(),
                KdfAlgorithm::Argon2id => "Argon2id".to_string(),
            };
            let details = match params.algorithm {
                KdfAlgorithm::Pbkdf2HmacSha256 => {
                    format!("iterations={}", params.pbkdf2_iterations)
                }
                KdfAlgorithm::Argon2id => {
                    format!(
                        "memory={} MiB, time={}, parallelism={}",
                        params.argon2_memory_kib / 1024,
                        params.argon2_time_cost,
                        params.argon2_parallelism
                    )
                }
            };
            kdf_info = Some((algo, details));
        }
    }
    drop(dev);

    let vol = auto_open_volume(image, block_size)?;
    let sb = vol.superblock();

    // Derive total/free sizes
    let total_mib = sb.total_blocks * sb.block_size as u64 / (1024 * 1024);
    let free_mib  = sb.free_blocks  * sb.block_size as u64 / (1024 * 1024);

    println!("CFS Volume: {image}");
    println!("  Magic:                 CFS1");
    println!("  Version:               {}", sb.version);
    println!("  Block size:            {} bytes", sb.block_size);
    println!("  Inode size:            {} bytes", sb.inode_size);
    println!("  Total blocks:          {} ({} MiB)", sb.total_blocks, total_mib);
    println!("  Free blocks:           {} ({} MiB)", sb.free_blocks, free_mib);
    println!("  Used blocks:           {}", sb.total_blocks.saturating_sub(sb.free_blocks));
    println!("  Inode count:           {}", sb.inode_count);
    println!("  Free inodes:           {}", sb.free_inodes);
    println!("  Root inode:            {}", sb.root_inode);
    println!("  Inode table start:     block {}", sb.inode_table_start);
    if sb.has_inode_bitmap() {
        println!("  Inode bitmap start:    block {}", sb.inode_bitmap_start);
    }
    println!("  Data bitmap start:     block {}", sb.bitmap_start);
    println!("  Data start:            block {}", sb.data_start);
    if sb.has_backup_sb() {
        println!("  Backup SB block:       block {}", sb.backup_sb_block);
    }
    println!("  UUID:                  {}", sb.uuid_str());
    let label = sb.label();
    if !label.is_empty() {
        println!("  Volume label:          {label}");
    }
    println!("  Mount count:           {}", sb.mount_count);
    if sb.last_mount_time > 0 {
        println!("  Last mounted:          {}", format_timestamp(sb.last_mount_time));
    }

    // v3-specific fields
    let features = format_feature_flags(sb.features_flags);
    println!("  Features:              {features}");

    if sb.version >= CFS_VERSION_V3 && sb.group_count > 0 {
        println!("  Block groups:          {}", sb.group_count);
        println!("  Blocks/group:          {}", sb.blocks_per_group);
        println!("  Inodes/group:          {}", sb.inodes_per_group);
        println!("  GDT blocks:            {}", sb.gdt_blocks);
    }
    if sb.journal_blocks > 0 {
        let journal_size = sb.journal_blocks * sb.block_size as u64;
        println!("  Journal blocks:        {} ({})", sb.journal_blocks, format_size_human(journal_size));
    }
    if sb.version >= CFS_VERSION_V3 {
        let eb = if sb.error_behavior == 1 { "Read-Only" } else { "Continue" };
        println!("  Error behavior:        {eb}");
        println!("  Default perms:         {:04o}", sb.default_permissions);
    }

    if is_encrypted {
        println!("  Encrypted:             yes");
        if let Some((algo, details)) = kdf_info {
            println!("  KDF algorithm:         {algo}");
            println!("  KDF parameters:        {details}");
        }
    } else {
        println!("  Encrypted:             no");
    }

    Ok(())
}

/// Format feature flags as a human-readable string.
fn format_feature_flags(flags: u32) -> String {
    if flags == 0 {
        return "none".to_string();
    }
    let names: &[(u32, &str)] = &[
        (FEATURE_HAS_INODE_BITMAP, "INODE_BITMAP"),
        (FEATURE_HAS_BACKUP_SB, "BACKUP_SB"),
        (FEATURE_256B_INODES, "256B_INODES"),
        (FEATURE_JOURNAL, "JOURNAL"),
        (FEATURE_SECURE_DELETE, "SECURE_DELETE"),
        (FEATURE_XATTR, "XATTR"),
        (FEATURE_SYMLINKS, "SYMLINKS"),
        (FEATURE_METADATA_HMAC, "METADATA_HMAC"),
        (FEATURE_EXTENTS, "EXTENTS"),
        (FEATURE_BLOCK_GROUPS, "BLOCK_GROUPS"),
        (FEATURE_HTREE, "HTREE"),
        (FEATURE_FLEX_BG, "FLEX_BG"),
        (FEATURE_DELAYED_ALLOC, "DELAYED_ALLOC"),
    ];
    let active: Vec<&str> = names.iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if active.is_empty() {
        format!("0x{flags:08X}")
    } else {
        active.join(" ")
    }
}

// ---------------------------------------------------------------------------
// 4D Ã¢â‚¬â€ cfs ls
// ---------------------------------------------------------------------------

pub fn cmd_ls(image: &str, block_size: u32, path: &str, all: bool, long: bool) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    let entries = vol.list_dir(path)?;

    let mut count = 0u32;
    for e in &entries {
        let name = e.name_str();
        if !all && (name == "." || name == "..") {
            continue;
        }
        count += 1;
        if long {
            let type_char = if e.file_type == INODE_DIR as u8 { 'd' } else { '-' };
            // Read inode for size/timestamp info

            // For . and .., use the entry's inode directly
            let inode = vol.read_inode(e.inode_index)?;
            println!(
                "{}{} {:>2} {:>5}:{:<5} {:>10}  {}  {}",
                type_char,
                format_permissions(inode.permissions),
                inode.nlinks,
                inode.owner_id,
                inode.group_id,
                inode.size,
                format_timestamp_ns(inode.modified),
                name
            );
        } else {
            println!("{name}");
        }
    }
    println!("{count} entries");

    Ok(())
}

// ---------------------------------------------------------------------------
// 4E Ã¢â‚¬â€ cfs mkdir
// ---------------------------------------------------------------------------

pub fn cmd_mkdir(image: &str, block_size: u32, path: &str, parents: bool) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;

    if parents {
        mkdir_parents(&vol, path)?;
    } else {
        vol.mkdir(path)?;
    }

    vol.sync()?;
    println!("Created directory: {path}");
    Ok(())
}

fn mkdir_parents(vol: &CFSVolume, path: &str) -> Result<()> {
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let mut current = String::new();
    for comp in components {
        current.push('/');
        current.push_str(comp);
        if !vol.exists(&current)? {
            vol.mkdir(&current)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 4F Ã¢â‚¬â€ cfs cat
// ---------------------------------------------------------------------------

pub fn cmd_cat(image: &str, block_size: u32, path: &str, out: &mut dyn Write) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    let inode = vol.stat(path)?;
    if inode.mode != INODE_FILE {
        bail!("not a file: {path}");
    }
    let data = vol.read_file(path, 0, inode.size)?;
    out.write_all(&data)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 4G Ã¢â‚¬â€ cfs write
// ---------------------------------------------------------------------------

pub fn cmd_write(
    image: &str,
    block_size: u32,
    path: &str,
    data: Option<&str>,
    from_file: Option<&str>,
) -> Result<()> {
    let bytes = if let Some(text) = data {
        text.as_bytes().to_vec()
    } else if let Some(host_path) = from_file {
        std::fs::read(host_path)
            .map_err(|e| anyhow::anyhow!("cannot read host file '{}': {}", host_path, e))?
    } else {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf.into_bytes()
    };

    let vol = auto_open_volume(image, block_size)?;

    if vol.exists(path)? {
        vol.truncate(path, 0)?;
        vol.write_file(path, 0, &bytes)?;
    } else {
        vol.create_file(path)?;
        vol.write_file(path, 0, &bytes)?;
    }

    vol.sync()?;
    println!("Wrote {} bytes to {path}", bytes.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// 4H Ã¢â‚¬â€ cfs rm
// ---------------------------------------------------------------------------

pub fn cmd_rm(image: &str, block_size: u32, path: &str) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    vol.delete_file(path)?;
    vol.sync()?;
    println!("Removed: {path}");
    Ok(())
}

// ---------------------------------------------------------------------------
// 4I Ã¢â‚¬â€ cfs rmdir
// ---------------------------------------------------------------------------

pub fn cmd_rmdir(image: &str, block_size: u32, path: &str) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    vol.rmdir(path)?;
    vol.sync()?;
    println!("Removed directory: {path}");
    Ok(())
}

// ---------------------------------------------------------------------------
// 4J Ã¢â‚¬â€ cfs stat
// ---------------------------------------------------------------------------

pub fn cmd_stat(image: &str, block_size: u32, path: &str) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    let inode = vol.stat(path)?;

    println!("Path:        {path}");
    println!("Type:        {}", format_mode(inode.mode));
    println!("Size:        {} bytes", inode.size);
    println!("Blocks:      {}", inode.block_count);
    println!("Links:       {}", inode.nlinks);
    println!("Permissions: {} (0o{:o})", format_permissions(inode.permissions), inode.permissions);
    println!("Owner:       {}", inode.owner_id);
    println!("Group:       {}", inode.group_id);
    println!("Created:     {}", format_timestamp_ns(inode.created));
    println!("Modified:    {}", format_timestamp_ns(inode.modified));
    println!("Accessed:    {}", format_timestamp_ns(inode.accessed_ns));
    println!("Changed:     {}", format_timestamp_ns(inode.changed_ns));

    Ok(())
}

// ---------------------------------------------------------------------------
// 4K Ã¢â‚¬â€ cfs mv
// ---------------------------------------------------------------------------

pub fn cmd_mv(image: &str, block_size: u32, old_path: &str, new_path: &str) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    vol.rename(old_path, new_path)?;
    vol.sync()?;
    println!("Moved: {old_path} -> {new_path}");
    Ok(())
}

// ---------------------------------------------------------------------------
// 4L Ã¢â‚¬â€ cfs cp
// ---------------------------------------------------------------------------

pub fn cmd_cp(image: &str, block_size: u32, src: &str, dest: &str) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    let inode = vol.stat(src)?;
    if inode.mode != INODE_FILE {
        bail!("can only copy files: {src}");
    }
    let data = vol.read_file(src, 0, inode.size)?;

    if vol.exists(dest)? {
        vol.truncate(dest, 0)?;
        vol.write_file(dest, 0, &data)?;
    } else {
        vol.create_file(dest)?;
        vol.write_file(dest, 0, &data)?;
    }

    vol.sync()?;
    println!("Copied: {src} -> {dest} ({} bytes)", data.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// 4M Ã¢â‚¬â€ cfs import / cfs export
// ---------------------------------------------------------------------------

pub fn cmd_import(
    image: &str,
    block_size: u32,
    host_file: &str,
    cfs_path: &str,
) -> Result<()> {
    let data = std::fs::read(host_file)
        .map_err(|e| anyhow::anyhow!("cannot read host file '{}': {}", host_file, e))?;

    let vol = auto_open_volume(image, block_size)?;

    if vol.exists(cfs_path)? {
        vol.truncate(cfs_path, 0)?;
        vol.write_file(cfs_path, 0, &data)?;
    } else {
        vol.create_file(cfs_path)?;
        vol.write_file(cfs_path, 0, &data)?;
    }

    vol.sync()?;
    println!("Imported: {host_file} -> {cfs_path} ({} bytes)", data.len());
    Ok(())
}

pub fn cmd_export(
    image: &str,
    block_size: u32,
    cfs_path: &str,
    host_file: &str,
) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    let inode = vol.stat(cfs_path)?;
    if inode.mode != INODE_FILE {
        bail!("not a file: {cfs_path}");
    }
    let data = vol.read_file(cfs_path, 0, inode.size)?;
    std::fs::write(host_file, &data)
        .map_err(|e| anyhow::anyhow!("cannot write host file '{}': {}", host_file, e))?;
    println!("Exported: {cfs_path} -> {host_file} ({} bytes)", data.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// 4N Ã¢â‚¬â€ cfs tree
// ---------------------------------------------------------------------------

pub fn cmd_tree(image: &str, block_size: u32, path: &str, out: &mut dyn Write) -> Result<()> {
    let vol = auto_open_volume(image, block_size)?;
    writeln!(out, "{path}")?;

    let mut dir_count = 0usize;
    let mut file_count = 0usize;
    print_tree(&vol, path, "", out, &mut dir_count, &mut file_count)?;

    writeln!(out, "\n{dir_count} directories, {file_count} files")?;
    Ok(())
}

fn print_tree(
    vol: &CFSVolume,
    path: &str,
    prefix: &str,
    out: &mut dyn Write,
    dir_count: &mut usize,
    file_count: &mut usize,
) -> Result<()> {
    let entries = vol.list_dir(path)?;
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            let n = e.name_str();
            n != "." && n != ".."
        })
        .collect();

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;
        let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
        let child_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}\u{2502}   ")
        };

        let name = entry.name_str();
        if entry.file_type == INODE_DIR as u8 {
            writeln!(out, "{prefix}{connector}{name}/")?;
            *dir_count += 1;
            let child_path = if path == "/" {
                format!("/{name}")
            } else {
                format!("{path}/{name}")
            };
            print_tree(vol, &child_path, &child_prefix, out, dir_count, file_count)?;
        } else {
            let inode = vol.read_inode(entry.inode_index)?;
            writeln!(out, "{prefix}{connector}{name} ({} bytes)", inode.size)?;
            *file_count += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 6E Ã¢â‚¬â€ cfs passwd
// ---------------------------------------------------------------------------

pub fn cmd_passwd(image: &str, block_size: u32, new_kdf: Option<KdfParams>) -> Result<()> {
    let mut dev = open_device(image, None)?;

    if !crypto::is_encrypted_device(&mut *dev)? {
        bail!("volume is not encrypted");
    }

    let old_pw = super::prompt_password("Current password: ")?;
    let new_pw = super::prompt_password("New password: ")?;
    let confirm = super::prompt_password("Confirm new password: ")?;
    if new_pw != confirm {
        bail!("passwords do not match");
    }

    crypto::change_password(&mut *dev, old_pw.as_bytes(), new_pw.as_bytes(), new_kdf, block_size)?;
    println!("Password changed successfully.");
    Ok(())
}

// ---------------------------------------------------------------------------
// 7F.4 Ã¢â‚¬â€ cfs bench-kdf
// ---------------------------------------------------------------------------

pub fn cmd_bench_kdf(kdf_params: &KdfParams) -> Result<()> {
    let algo_name = match kdf_params.algorithm {
        KdfAlgorithm::Pbkdf2HmacSha256 => "PBKDF2-HMAC-SHA256",
        KdfAlgorithm::Argon2id => "Argon2id",
    };
    let detail = match kdf_params.algorithm {
        KdfAlgorithm::Pbkdf2HmacSha256 => {
            format!("iterations={}", kdf_params.pbkdf2_iterations)
        }
        KdfAlgorithm::Argon2id => {
            format!(
                "memory={} MiB, time={}, parallelism={}",
                kdf_params.argon2_memory_kib / 1024,
                kdf_params.argon2_time_cost,
                kdf_params.argon2_parallelism
            )
        }
    };
    println!("Benchmarking {algo_name} ({detail})...");

    let duration = crypto::benchmark_kdf(kdf_params)?;
    let ms = duration.as_millis();
    let secs = duration.as_secs_f64();

    if secs >= 1.0 {
        println!("Estimated unlock time: {secs:.2}s");
    } else {
        println!("Estimated unlock time: {ms}ms");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// cfs presets
// ---------------------------------------------------------------------------

pub fn cmd_presets() -> Result<()> {
    println!("Available format presets:");
    println!();
    println!("  {:<14} Block: 4K   Inode: 256B  Ratio: 16K  Journal: 1.0%  SecDel: yes", "general");
    println!("  {:<14} Block: 16K  Inode: 256B  Ratio: 64K  Journal: 0.5%  SecDel: yes", "large-files");
    println!("  {:<14} Block: 4K   Inode: 256B  Ratio: 4K   Journal: 2.0%  SecDel: yes", "small-files");
    println!("  {:<14} Block: 4K   Inode: 256B  Ratio: 16K  Journal: 2.0%  SecDel: yes  Error: read-only", "max-security");
    println!("  {:<14} Block: 4K   Inode: 128B  Ratio: 16K  Journal: off   SecDel: no", "minimal");
    Ok(())
}

// ---------------------------------------------------------------------------
// cfs chmod / chown
// ---------------------------------------------------------------------------

pub fn cmd_chmod(image: &str, block_size: u32, path: &str, mode: &str) -> Result<()> {
    let perms = super::parse_permissions(mode)?;
    let vol = super::auto_open_volume(image, block_size)?;
    vol.chmod(path, perms)?;
    vol.sync()?;
    println!("permissions set to 0o{:o} on {}", perms, path);
    Ok(())
}

pub fn cmd_chown(
    image: &str,
    block_size: u32,
    path: &str,
    owner: Option<u32>,
    group: Option<u32>,
) -> Result<()> {
    if owner.is_none() && group.is_none() {
        bail!("at least one of --owner or --group must be specified");
    }
    let vol = super::auto_open_volume(image, block_size)?;
    vol.chown(path, owner, group)?;
    vol.sync()?;
    if let (Some(o), Some(g)) = (owner, group) {
        println!("owner set to {}:{} on {}", o, g, path);
    } else if let Some(o) = owner {
        println!("owner set to {} on {}", o, path);
    } else if let Some(g) = group {
        println!("group set to {} on {}", g, path);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// cfs journal-status
// ---------------------------------------------------------------------------

pub fn cmd_journal_status(image: &str, block_size: u32) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;
    match vol.journal_status() {
        Some(status) => print!("{status}"),
        None => println!("Journal is not enabled on this volume."),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 10G Ã¢â‚¬â€ cfs ln
// ---------------------------------------------------------------------------

pub fn cmd_ln(image: &str, block_size: u32, target: &str, link_path: &str, symbolic: bool) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;

    if symbolic {
        vol.symlink(target, link_path)?;
        println!("Created symbolic link: {link_path} -> {target}");
    } else {
        vol.link(target, link_path)?;
        println!("Created hard link: {link_path} -> {target}");
    }

    vol.sync()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 10G Ã¢â‚¬â€ cfs readlink
// ---------------------------------------------------------------------------

pub fn cmd_readlink(image: &str, block_size: u32, path: &str) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;
    let target = vol.readlink(path)?;
    println!("{target}");
    Ok(())
}

// ---------------------------------------------------------------------------
// 10G Ã¢â‚¬â€ cfs xattr
// ---------------------------------------------------------------------------

pub fn cmd_xattr(
    image: &str,
    block_size: u32,
    op: &str,
    path: &str,
    key: Option<&str>,
    value: Option<&str>,
) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;

    match op {
        "get" => {
            let key = key.ok_or_else(|| anyhow::anyhow!("key is required for 'get' operation"))?;
            match vol.get_xattr(path, key)? {
                Some(val) => {
                    // Try to print as UTF-8, fall back to hex
                    match std::str::from_utf8(&val) {
                        Ok(s) => println!("{s}"),
                        Err(_) => {
                            for b in &val {
                                print!("{b:02x}");
                            }
                            println!();
                        }
                    }
                }
                None => println!("(not set)"),
            }
        }
        "set" => {
            let key = key.ok_or_else(|| anyhow::anyhow!("key is required for 'set' operation"))?;
            let value = value.ok_or_else(|| anyhow::anyhow!("value is required for 'set' operation"))?;
            vol.set_xattr(path, key, value.as_bytes())?;
            vol.sync()?;
            println!("Set {key} on {path}");
        }
        "list" => {
            let keys = vol.list_xattr(path)?;
            if keys.is_empty() {
                println!("(no extended attributes)");
            } else {
                for k in &keys {
                    println!("{k}");
                }
            }
        }
        "rm" => {
            let key = key.ok_or_else(|| anyhow::anyhow!("key is required for 'rm' operation"))?;
            vol.remove_xattr(path, key)?;
            vol.sync()?;
            println!("Removed {key} from {path}");
        }
        other => bail!("unknown xattr operation '{}' (expected: get, set, list, rm)", other),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 10I — Fallocate, PunchHole, Defrag, FragStat
// ---------------------------------------------------------------------------

pub fn cmd_fallocate(image: &str, block_size: u32, path: &str, offset: u64, length: u64) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;
    vol.fallocate(path, offset, length)?;
    vol.sync()?;

    let bs = block_size as u64;
    let blocks = (length + bs - 1) / bs;
    println!("Preallocated {} blocks ({}) for {}", blocks, format_bytes(length), path);
    println!("  Offset: {}", offset);
    println!("  Length: {} bytes", length);
    println!("  Blocks: {} (uninitialized)", blocks);
    Ok(())
}

pub fn cmd_punch_hole(image: &str, block_size: u32, path: &str, offset: u64, length: u64) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;
    let freed = vol.punch_hole(path, offset, length)?;
    vol.sync()?;

    println!("Punched hole in {}", path);
    println!("  Offset: {} ({})", offset, format_bytes(offset));
    println!("  Length: {} ({})", length, format_bytes(length));
    println!("  Blocks freed: {}", freed);
    println!("  File size: unchanged");
    Ok(())
}

pub fn cmd_defrag(
    image: &str,
    block_size: u32,
    path: Option<&str>,
    volume_wide: bool,
    dry_run: bool,
) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;

    if dry_run {
        let p = path.unwrap_or("/");
        let report = vol.fragmentation_report(p)?;
        print_frag_report(&report);
        return Ok(());
    }

    if volume_wide {
        let stats = vol.defragment_volume()?;
        vol.sync()?;
        println!("Volume defragmentation complete");
        println!("  Files checked: {}", stats.files_checked);
        println!("  Files defragmented: {}", stats.files_defragmented);
        println!("  Already contiguous: {}", stats.already_contiguous);
        println!("  Extents: {} → {}", stats.extents_before, stats.extents_after);
        println!("  Blocks moved: {}", stats.blocks_moved);
        println!("  Errors (skipped): {}", stats.skipped_errors);
    } else {
        let p = path.ok_or_else(|| anyhow::anyhow!("specify a file path or use --volume"))?;
        let stats = vol.defragment_file(p)?;
        vol.sync()?;
        if stats.already_contiguous > 0 {
            println!("{} is already contiguous (1 extent)", p);
        } else {
            println!("Defragmented {}", p);
            println!("  Extents: {} → {}", stats.extents_before, stats.extents_after);
            println!("  Blocks moved: {}", stats.blocks_moved);
        }
    }

    Ok(())
}

pub fn cmd_fragstat(image: &str, block_size: u32, path: &str) -> Result<()> {
    let vol = super::auto_open_volume(image, block_size)?;
    let report = vol.fragmentation_report(path)?;
    print_frag_report(&report);
    Ok(())
}

fn print_frag_report(report: &crate::volume::FragReport) {
    println!("Fragmentation report for {}", report.path);
    println!("  File size: {}", format_bytes(report.file_size));
    println!("  Total blocks: {}", report.total_blocks);
    println!("  Extents: {}", report.extent_count);
    println!("  Largest extent: {} blocks", report.largest_extent);
    println!("  Smallest extent: {} blocks", report.smallest_extent);
    println!("  Contiguity score: {:.2}", report.contiguity_score);
    if report.needs_defrag {
        println!("  Recommendation: defragmentation recommended");
    } else {
        println!("  Recommendation: no defragmentation needed");
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{auto_open_volume_with_password, open_volume};
    use crate::crypto::{KdfAlgorithm, KdfParams};
    use crate::volume::DEFAULT_BLOCK_SIZE;
    use tempfile::NamedTempFile;

    fn temp_image_path() -> NamedTempFile {
        NamedTempFile::new().unwrap()
    }

    /// Dummy KdfParams for non-encrypted format (value is unused).
    fn dummy_kdf() -> KdfParams {
        KdfParams::default_argon2id()
    }

    /// Low-iteration PBKDF2 params for fast encrypted tests.
    fn fast_pbkdf2() -> KdfParams {
        KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 600_000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }
    }

    fn format_temp(size_str: &str) -> (NamedTempFile, String) {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        let opts = FormatOptions::default();
        cmd_format(&path, size_str, false, &dummy_kdf(), &opts).unwrap();
        (tmp, path)
    }

    // --- 4B format tests ---

    #[test]
    fn test_cmd_format_creates_volume() {
        let (_tmp, path) = format_temp("2M");
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert_eq!(vol.superblock().block_size, DEFAULT_BLOCK_SIZE);
        assert!(vol.superblock().total_blocks > 0);
        assert!(vol.superblock().free_blocks > 0);
    }

    #[test]
    fn test_cmd_format_custom_block_size() {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        let mut opts = FormatOptions::default();
        opts.block_size = 512;
        opts.blocks_per_group = 512 * 8; // Must match block_size
        cmd_format(&path, "2M", false, &dummy_kdf(), &opts).unwrap();
        let vol = open_volume(&path, 512).unwrap();
        assert_eq!(vol.superblock().block_size, 512);
    }

    #[test]
    fn test_cmd_format_zero_size_error() {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        let opts = FormatOptions::default();
        assert!(cmd_format(&path, "0", false, &dummy_kdf(), &opts).is_err());
    }

    // --- 4C info tests ---

    #[test]
    fn test_cmd_info_displays_fields() {
        let (_tmp, path) = format_temp("2M");
        // Just check it doesn't error Ã¢â‚¬â€ output goes to stdout
        cmd_info(&path, DEFAULT_BLOCK_SIZE).unwrap();
    }

    // --- 4D ls tests ---

    #[test]
    fn test_cmd_ls_root_after_format() {
        let (_tmp, path) = format_temp("2M");
        // Should not error listing root with -a
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/", true, false).unwrap();
    }

    #[test]
    fn test_cmd_ls_with_files() {
        let (_tmp, path) = format_temp("2M");
        {
            let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
            vol.mkdir("/docs").unwrap();
            vol.create_file("/readme.txt").unwrap();
            vol.write_file("/readme.txt", 0, b"hello").unwrap();
            vol.sync().unwrap();
        }
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/", false, false).unwrap();
    }

    #[test]
    fn test_cmd_ls_hides_dot_entries() {
        // Functional test: list_dir returns . and .., but ls without -a should skip them
        let (_tmp, path) = format_temp("2M");
        // No error expected
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/", false, false).unwrap();
    }

    #[test]
    fn test_cmd_ls_long_format() {
        let (_tmp, path) = format_temp("2M");
        {
            let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
            vol.create_file("/test.txt").unwrap();
            vol.write_file("/test.txt", 0, b"data").unwrap();
            vol.sync().unwrap();
        }
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/", false, true).unwrap();
    }

    // --- 4E mkdir tests ---

    #[test]
    fn test_cmd_mkdir_creates_dir() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/docs", false).unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(vol.exists("/docs").unwrap());
    }

    #[test]
    fn test_cmd_mkdir_nested() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/a", false).unwrap();
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/a/b", false).unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(vol.exists("/a/b").unwrap());
    }

    #[test]
    fn test_cmd_mkdir_parents() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/a/b/c", true).unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(vol.exists("/a").unwrap());
        assert!(vol.exists("/a/b").unwrap());
        assert!(vol.exists("/a/b/c").unwrap());
    }

    // --- 4F cat tests ---

    #[test]
    fn test_cmd_cat_reads_file() {
        let (_tmp, path) = format_temp("2M");
        {
            let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
            vol.create_file("/hello.txt").unwrap();
            vol.write_file("/hello.txt", 0, b"Hello CFS").unwrap();
            vol.sync().unwrap();
        }
        let mut out = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/hello.txt", &mut out).unwrap();
        assert_eq!(out, b"Hello CFS");
    }

    #[test]
    fn test_cmd_cat_nonexistent() {
        let (_tmp, path) = format_temp("2M");
        let mut out = Vec::new();
        assert!(cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/nope.txt", &mut out).is_err());
    }

    // --- 4G write tests ---

    #[test]
    fn test_cmd_write_data_flag() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/test.txt", Some("hello"), None).unwrap();
        let mut out = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/test.txt", &mut out).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn test_cmd_write_from_file() {
        let (_tmp, path) = format_temp("2M");
        let host_tmp = NamedTempFile::new().unwrap();
        std::fs::write(host_tmp.path(), b"from host").unwrap();
        cmd_write(
            &path,
            DEFAULT_BLOCK_SIZE,
            "/imported.txt",
            None,
            Some(host_tmp.path().to_str().unwrap()),
        )
        .unwrap();
        let mut out = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/imported.txt", &mut out).unwrap();
        assert_eq!(out, b"from host");
    }

    #[test]
    fn test_cmd_write_creates_if_missing() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/new.txt", Some("created"), None).unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(vol.exists("/new.txt").unwrap());
    }

    // --- 4H rm tests ---

    #[test]
    fn test_cmd_rm_removes_file() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/f.txt", Some("x"), None).unwrap();
        cmd_rm(&path, DEFAULT_BLOCK_SIZE, "/f.txt").unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(!vol.exists("/f.txt").unwrap());
    }

    #[test]
    fn test_cmd_rm_dir_error() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/mydir", false).unwrap();
        assert!(cmd_rm(&path, DEFAULT_BLOCK_SIZE, "/mydir").is_err());
    }

    // --- 4I rmdir tests ---

    #[test]
    fn test_cmd_rmdir_empty() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/empty", false).unwrap();
        cmd_rmdir(&path, DEFAULT_BLOCK_SIZE, "/empty").unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(!vol.exists("/empty").unwrap());
    }

    #[test]
    fn test_cmd_rmdir_nonempty_err() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/mydir", false).unwrap();
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/mydir/f.txt", Some("x"), None).unwrap();
        assert!(cmd_rmdir(&path, DEFAULT_BLOCK_SIZE, "/mydir").is_err());
    }

    // --- 4J stat tests ---

    #[test]
    fn test_cmd_stat_file() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/test.txt", Some("12345"), None).unwrap();
        // Just verify no error; output goes to stdout
        cmd_stat(&path, DEFAULT_BLOCK_SIZE, "/test.txt").unwrap();
    }

    #[test]
    fn test_cmd_stat_dir() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/mydir", false).unwrap();
        cmd_stat(&path, DEFAULT_BLOCK_SIZE, "/mydir").unwrap();
    }

    // --- 4K mv tests ---

    #[test]
    fn test_cmd_mv_rename() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/a.txt", Some("data"), None).unwrap();
        cmd_mv(&path, DEFAULT_BLOCK_SIZE, "/a.txt", "/b.txt").unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(!vol.exists("/a.txt").unwrap());
        assert!(vol.exists("/b.txt").unwrap());
        let data = vol.read_file("/b.txt", 0, 100).unwrap();
        assert_eq!(&data, b"data");
    }

    #[test]
    fn test_cmd_mv_across_dirs() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/src", false).unwrap();
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/dst", false).unwrap();
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/src/f.txt", Some("moved"), None).unwrap();
        cmd_mv(&path, DEFAULT_BLOCK_SIZE, "/src/f.txt", "/dst/f.txt").unwrap();
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(!vol.exists("/src/f.txt").unwrap());
        let data = vol.read_file("/dst/f.txt", 0, 100).unwrap();
        assert_eq!(&data, b"moved");
    }

    // --- 4L cp tests ---

    #[test]
    fn test_cmd_cp_file() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/orig.txt", Some("copy me"), None).unwrap();
        cmd_cp(&path, DEFAULT_BLOCK_SIZE, "/orig.txt", "/copy.txt").unwrap();
        let mut out1 = Vec::new();
        let mut out2 = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/orig.txt", &mut out1).unwrap();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/copy.txt", &mut out2).unwrap();
        assert_eq!(out1, out2);
        assert_eq!(out1, b"copy me");
    }

    #[test]
    fn test_cmd_cp_nonexistent() {
        let (_tmp, path) = format_temp("2M");
        assert!(cmd_cp(&path, DEFAULT_BLOCK_SIZE, "/nope.txt", "/dest.txt").is_err());
    }

    // --- 4M import/export tests ---

    #[test]
    fn test_cmd_import_file() {
        let (_tmp, path) = format_temp("2M");
        let host_tmp = NamedTempFile::new().unwrap();
        std::fs::write(host_tmp.path(), b"host data").unwrap();
        cmd_import(
            &path,
            DEFAULT_BLOCK_SIZE,
            host_tmp.path().to_str().unwrap(),
            "/imported.txt",
        )
        .unwrap();
        let mut out = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/imported.txt", &mut out).unwrap();
        assert_eq!(out, b"host data");
    }

    #[test]
    fn test_cmd_export_file() {
        let (_tmp, path) = format_temp("2M");
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/data.txt", Some("export me"), None).unwrap();
        let host_tmp = NamedTempFile::new().unwrap();
        cmd_export(
            &path,
            DEFAULT_BLOCK_SIZE,
            "/data.txt",
            host_tmp.path().to_str().unwrap(),
        )
        .unwrap();
        let data = std::fs::read(host_tmp.path()).unwrap();
        assert_eq!(data, b"export me");
    }

    #[test]
    fn test_cmd_import_large() {
        let (_tmp, path) = format_temp("2M");
        let host_tmp = NamedTempFile::new().unwrap();
        // Create a 10 KB file with a known pattern
        let big_data: Vec<u8> = (0..10240).map(|i| (i % 251) as u8).collect();
        std::fs::write(host_tmp.path(), &big_data).unwrap();

        cmd_import(
            &path,
            DEFAULT_BLOCK_SIZE,
            host_tmp.path().to_str().unwrap(),
            "/big.bin",
        )
        .unwrap();

        // Export and compare
        let out_tmp = NamedTempFile::new().unwrap();
        cmd_export(
            &path,
            DEFAULT_BLOCK_SIZE,
            "/big.bin",
            out_tmp.path().to_str().unwrap(),
        )
        .unwrap();
        let exported = std::fs::read(out_tmp.path()).unwrap();
        assert_eq!(exported, big_data);
    }

    // --- 4N tree tests ---

    #[test]
    fn test_cmd_tree_hierarchy() {
        let (_tmp, path) = format_temp("2M");
        {
            let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
            vol.mkdir("/docs").unwrap();
            vol.mkdir("/docs/notes").unwrap();
            vol.create_file("/readme.txt").unwrap();
            vol.write_file("/readme.txt", 0, b"hi").unwrap();
            vol.create_file("/docs/notes/todo.txt").unwrap();
            vol.write_file("/docs/notes/todo.txt", 0, b"stuff").unwrap();
            vol.sync().unwrap();
        }
        let mut out = Vec::new();
        cmd_tree(&path, DEFAULT_BLOCK_SIZE, "/", &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("docs/"));
        assert!(output.contains("readme.txt"));
        assert!(output.contains("notes/"));
        assert!(output.contains("todo.txt"));
        assert!(output.contains("directories"));
        assert!(output.contains("files"));
    }

    // --- 4O End-to-End tests ---

    #[test]
    fn test_e2e_cli_workflow() {
        let (_tmp, path) = format_temp("2M");

        // info
        cmd_info(&path, DEFAULT_BLOCK_SIZE).unwrap();

        // mkdir
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/docs", false).unwrap();
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/docs/notes", false).unwrap();

        // write files
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/readme.txt", Some("Hello CFS"), None).unwrap();
        cmd_write(&path, DEFAULT_BLOCK_SIZE, "/docs/plan.md", Some("# Plan"), None).unwrap();

        // ls
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/", false, false).unwrap();
        cmd_ls(&path, DEFAULT_BLOCK_SIZE, "/docs", true, true).unwrap();

        // cat
        let mut out = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/readme.txt", &mut out).unwrap();
        assert_eq!(out, b"Hello CFS");

        // stat
        cmd_stat(&path, DEFAULT_BLOCK_SIZE, "/readme.txt").unwrap();

        // mv
        cmd_mv(&path, DEFAULT_BLOCK_SIZE, "/readme.txt", "/docs/readme.txt").unwrap();
        let mut out2 = Vec::new();
        cmd_cat(&path, DEFAULT_BLOCK_SIZE, "/docs/readme.txt", &mut out2).unwrap();
        assert_eq!(out2, b"Hello CFS");

        // cp
        cmd_cp(&path, DEFAULT_BLOCK_SIZE, "/docs/readme.txt", "/docs/readme_backup.txt").unwrap();

        // rm
        cmd_rm(&path, DEFAULT_BLOCK_SIZE, "/docs/readme_backup.txt").unwrap();
        cmd_rm(&path, DEFAULT_BLOCK_SIZE, "/docs/plan.md").unwrap();
        cmd_rm(&path, DEFAULT_BLOCK_SIZE, "/docs/readme.txt").unwrap();

        // rmdir
        cmd_rmdir(&path, DEFAULT_BLOCK_SIZE, "/docs/notes").unwrap();
        cmd_rmdir(&path, DEFAULT_BLOCK_SIZE, "/docs").unwrap();

        // tree Ã¢â‚¬â€ should be empty root
        let mut tout = Vec::new();
        cmd_tree(&path, DEFAULT_BLOCK_SIZE, "/", &mut tout).unwrap();
        let tree_out = String::from_utf8(tout).unwrap();
        assert!(tree_out.contains("0 directories, 0 files"));

        // Verify clean: only . and .. in root
        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        let entries = vol.list_dir("/").unwrap();
        let real = entries.iter().filter(|e| {
            let n = e.name_str();
            n != "." && n != ".."
        }).count();
        assert_eq!(real, 0);
    }

    #[test]
    fn test_e2e_import_export_roundtrip() {
        let (_tmp, path) = format_temp("2M");

        // Create a host file with known data
        let host_in = NamedTempFile::new().unwrap();
        let data: Vec<u8> = (0..10240).map(|i| (i % 251) as u8).collect();
        std::fs::write(host_in.path(), &data).unwrap();

        // Import
        cmd_import(
            &path,
            DEFAULT_BLOCK_SIZE,
            host_in.path().to_str().unwrap(),
            "/data.bin",
        ).unwrap();

        // Export
        let host_out = NamedTempFile::new().unwrap();
        cmd_export(
            &path,
            DEFAULT_BLOCK_SIZE,
            "/data.bin",
            host_out.path().to_str().unwrap(),
        ).unwrap();

        // Compare
        let exported = std::fs::read(host_out.path()).unwrap();
        assert_eq!(data, exported);
    }

    #[test]
    fn test_e2e_mkdir_parents() {
        let (_tmp, path) = format_temp("2M");
        cmd_mkdir(&path, DEFAULT_BLOCK_SIZE, "/a/b/c/d", true).unwrap();

        let vol = open_volume(&path, DEFAULT_BLOCK_SIZE).unwrap();
        assert!(vol.exists("/a").unwrap());
        assert!(vol.exists("/a/b").unwrap());
        assert!(vol.exists("/a/b/c").unwrap());
        assert!(vol.exists("/a/b/c/d").unwrap());
    }

    // --- 6E encryption CLI tests ---

    #[test]
    fn test_format_encrypted_creates_header() {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        // Use low iters for test speed
        let opts = FormatOptions::default();
        cmd_format_with_password(&path, "2M", "test_pw", &fast_pbkdf2(), &opts).unwrap();

        // First 4 bytes should be "CFSE"
        let mut dev = open_device(&path, None).unwrap();
        let mut buf = vec![0u8; 512];
        dev.read(0, &mut buf).unwrap();
        assert_eq!(&buf[0..4], b"CFSE");
    }

    #[test]
    fn test_auto_detect_plaintext() {
        let (_tmp, path) = format_temp("2M");
        // open_volume works on plaintext (no password needed)
        let vol = auto_open_volume_with_password(&path, DEFAULT_BLOCK_SIZE, None).unwrap();
        assert_eq!(&vol.superblock().magic, b"CFS1");
    }

    #[test]
    fn test_auto_detect_encrypted() {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        let opts = FormatOptions::default();
        cmd_format_with_password(&path, "2M", "secret123", &fast_pbkdf2(), &opts).unwrap();

        // Without password Ã¢â€ â€™ should fail
        let result = auto_open_volume_with_password(&path, DEFAULT_BLOCK_SIZE, None);
        assert!(result.is_err());

        // With correct password Ã¢â€ â€™ should succeed
        let vol = auto_open_volume_with_password(&path, DEFAULT_BLOCK_SIZE, Some("secret123")).unwrap();
        assert_eq!(&vol.superblock().magic, b"CFS1");
    }

    #[test]
    fn test_passwd_changes_password() {
        let tmp = temp_image_path();
        let path = tmp.path().to_str().unwrap().to_string();
        let opts = FormatOptions::default();
        cmd_format_with_password(&path, "2M", "old_pass", &fast_pbkdf2(), &opts).unwrap();

        // Change password directly (non-interactive)
        let mut dev = open_device(&path, None).unwrap();
        crate::crypto::change_password(
            &mut *dev,
            b"old_pass",
            b"new_pass",
            None,
            DEFAULT_BLOCK_SIZE,
        ).unwrap();
        drop(dev);

        // Old password fails
        let result = auto_open_volume_with_password(&path, DEFAULT_BLOCK_SIZE, Some("old_pass"));
        assert!(result.is_err());

        // New password works
        let vol = auto_open_volume_with_password(&path, DEFAULT_BLOCK_SIZE, Some("new_pass")).unwrap();
        assert_eq!(&vol.superblock().magic, b"CFS1");
    }
}
