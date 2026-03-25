pub mod commands;

use clap::{Args, Parser, Subcommand};
use anyhow::{bail, Result};

use std::path::Path;

use zeroize::Zeroize;

use crate::block_device::{CFSBlockDevice, FileBlockDevice, RawPartitionBlockDevice};
use crate::crypto::{self, EncryptedBlockDevice, KdfAlgorithm, KdfParams};
use crate::volume::{CFSVolume, FormatOptions, MountOptions, AtimeMode, ErrorBehavior, INODE_DIR, INODE_FILE, INODE_UNUSED, INODE_SYMLINK};

// ---------------------------------------------------------------------------
// Clap structs
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "cfs", version, about = "CFS — Custom Filesystem Tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Shared arguments for commands that operate on an existing image.
#[derive(Args, Clone)]
pub struct ImageArgs {
    /// Path to the CFS image file (e.g., cfs.img)
    pub image: String,

    /// Block size in bytes
    #[arg(long, default_value = "4096")]
    pub block_size: u32,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Format a new CFS filesystem image
    Format {
        /// Path to the image file to create
        image: String,
        /// Image size (e.g., "64", "2M", "1G")
        size: String,
        /// Block size in bytes
        #[arg(long, default_value = "4096")]
        block_size: u32,
        /// Create an encrypted volume (prompts for password)
        #[arg(long, alias = "encrypt")]
        encrypted: bool,
        /// KDF algorithm: "argon2id" or "pbkdf2" (default: argon2id)
        #[arg(long, default_value = "argon2id")]
        kdf: String,
        /// PBKDF2 iteration count (only with --kdf pbkdf2)
        #[arg(long, default_value = "600000")]
        pbkdf2_iters: u32,
        /// Argon2id memory cost in MiB (only with --kdf argon2id)
        #[arg(long, default_value = "32")]
        argon2_memory: u32,
        /// Argon2id time cost / iterations (only with --kdf argon2id)
        #[arg(long, default_value = "2")]
        argon2_time: u32,
        /// Argon2id parallelism / lanes (only with --kdf argon2id)
        #[arg(long, default_value = "1")]
        argon2_parallelism: u32,
        /// Inode size: 128 (legacy) or 256 (v3 full features)
        #[arg(long)]
        inode_size: Option<u32>,
        /// Bytes-per-inode ratio (1024..65536)
        #[arg(long)]
        inode_ratio: Option<u32>,
        /// Journal size as percentage of volume (0 = disabled, 0.5..5.0)
        #[arg(long)]
        journal: Option<f32>,
        /// Volume label (max 31 chars)
        #[arg(long)]
        label: Option<String>,
        /// Apply a format preset: general, large-files, small-files, max-security, minimal
        #[arg(long)]
        preset: Option<String>,
        /// Enable secure delete
        #[arg(long)]
        secure_delete: Option<bool>,
        /// Default file permissions in octal (e.g. 755)
        #[arg(long)]
        default_perms: Option<String>,
        /// Error behavior: continue or read-only
        #[arg(long)]
        error_behavior: Option<String>,
        /// Blocks per group (max: block_size * 8)
        #[arg(long)]
        blocks_per_group: Option<u32>,
    },
    /// Display filesystem superblock information
    Info {
        #[command(flatten)]
        args: ImageArgs,
    },
    /// List directory contents
    Ls {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS path to list (e.g., "/", "/docs")
        path: String,
        /// Show "." and ".." entries
        #[arg(short, long)]
        all: bool,
        /// Long format: show type, size, inode, timestamps
        #[arg(short, long)]
        long: bool,
    },
    /// Create a directory
    Mkdir {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS path for the new directory
        path: String,
        /// Create parent directories as needed
        #[arg(short, long)]
        parents: bool,
    },
    /// Read a file and print to stdout
    Cat {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path to read
        path: String,
    },
    /// Write data to a file (creates if missing, truncates+rewrites if exists)
    Write {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path to write to
        path: String,
        /// Literal text data to write
        #[arg(long)]
        data: Option<String>,
        /// Host file to read data from
        #[arg(long)]
        from_file: Option<String>,
    },
    /// Remove a file
    Rm {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path to delete
        path: String,
    },
    /// Remove an empty directory
    Rmdir {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS directory path to remove
        path: String,
    },
    /// Show file or directory metadata
    Stat {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS path to stat
        path: String,
    },
    /// Rename or move a file/directory
    Mv {
        #[command(flatten)]
        args: ImageArgs,
        /// Current CFS path
        old_path: String,
        /// New CFS path
        new_path: String,
    },
    /// Copy a file within the CFS volume
    Cp {
        #[command(flatten)]
        args: ImageArgs,
        /// Source CFS path
        src: String,
        /// Destination CFS path
        dest: String,
    },
    /// Import a host file into the CFS volume
    Import {
        #[command(flatten)]
        args: ImageArgs,
        /// Host filesystem path to read from
        host_file: String,
        /// CFS path to write to
        cfs_path: String,
    },
    /// Export a CFS file to the host filesystem
    Export {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS path to read from
        cfs_path: String,
        /// Host filesystem path to write to
        host_file: String,
    },
    /// Show directory tree recursively
    Tree {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS path to start from (default: "/")
        #[arg(default_value = "/")]
        path: String,
    },
    /// Mount a CFS image as a Windows drive via WinFSP
    Mount {
        #[command(flatten)]
        args: ImageArgs,
        /// Drive letter or mount path (e.g., "X:", "C:\\MountPoint")
        #[arg(short, long)]
        mount_point: Option<String>,
        /// Inode cache size (0..4096)
        #[arg(long)]
        cache_inodes: Option<u32>,
        /// Block cache size (0..8192)
        #[arg(long)]
        cache_blocks: Option<u32>,
        /// Access time mode: always, relatime, never
        #[arg(long)]
        atime: Option<String>,
        /// Override to enable secure delete for this session
        #[arg(long)]
        secure_delete: Option<bool>,
        /// Mount as read-only
        #[arg(long)]
        read_only: bool,
        /// Number of WinFSP dispatcher threads (0 = auto, default: 0)
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
    /// Change password on an encrypted CFS volume
    Passwd {
        #[command(flatten)]
        args: ImageArgs,
        /// New KDF algorithm: "argon2id" or "pbkdf2" (default: keep current)
        #[arg(long)]
        kdf: Option<String>,
        /// New PBKDF2 iteration count (only with --kdf pbkdf2)
        #[arg(long)]
        pbkdf2_iters: Option<u32>,
        /// New Argon2id memory cost in MiB (only with --kdf argon2id)
        #[arg(long)]
        argon2_memory: Option<u32>,
        /// New Argon2id time cost (only with --kdf argon2id)
        #[arg(long)]
        argon2_time: Option<u32>,
        /// New Argon2id parallelism (only with --kdf argon2id)
        #[arg(long)]
        argon2_parallelism: Option<u32>,
    },
    /// Benchmark KDF parameters (measure derivation time)
    BenchKdf {
        /// KDF algorithm: "argon2id" or "pbkdf2"
        #[arg(long, default_value = "argon2id")]
        kdf: String,
        /// PBKDF2 iterations (only with --kdf pbkdf2)
        #[arg(long, default_value = "600000")]
        pbkdf2_iters: u32,
        /// Argon2id memory in MiB (only with --kdf argon2id)
        #[arg(long, default_value = "32")]
        argon2_memory: u32,
        /// Argon2id time cost (only with --kdf argon2id)
        #[arg(long, default_value = "2")]
        argon2_time: u32,
        /// Argon2id parallelism (only with --kdf argon2id)
        #[arg(long, default_value = "1")]
        argon2_parallelism: u32,
    },
    /// Show available format presets
    Presets,
    /// Change file/directory permissions
    Chmod {
        #[command(flatten)]
        args: ImageArgs,
        /// Path within the filesystem
        path: String,
        /// Permissions (octal like "755" or symbolic like "rwxr-xr-x")
        mode: String,
    },
    /// Change file/directory owner and group
    Chown {
        #[command(flatten)]
        args: ImageArgs,
        /// Path within the filesystem
        path: String,
        /// Owner ID
        #[arg(long)]
        owner: Option<u32>,
        /// Group ID
        #[arg(long)]
        group: Option<u32>,
    },
    /// Show journal status for a CFS volume
    JournalStatus {
        #[command(flatten)]
        args: ImageArgs,
    },
    /// Create a hard link or symbolic link
    Ln {
        #[command(flatten)]
        args: ImageArgs,
        /// Target path (existing file for hard link, or symlink target)
        target: String,
        /// Link path to create
        link_path: String,
        /// Create a symbolic link instead of a hard link
        #[arg(short = 's', long)]
        symbolic: bool,
    },
    /// Read the target of a symbolic link
    Readlink {
        #[command(flatten)]
        args: ImageArgs,
        /// Path to the symbolic link
        path: String,
    },
    /// Manage extended attributes
    Xattr {
        #[command(flatten)]
        args: ImageArgs,
        /// Operation: get, set, list, rm
        op: String,
        /// Path to the file/directory
        path: String,
        /// Attribute key (required for get, set, rm)
        key: Option<String>,
        /// Attribute value (required for set)
        value: Option<String>,
    },
    /// Preallocate disk space for a file (uninitialized extents)
    Fallocate {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path
        path: String,
        /// Byte offset to start preallocation (e.g. "0", "1M")
        #[arg(long, default_value = "0")]
        offset: String,
        /// Length to preallocate (e.g. "100M", "1G")
        length: String,
    },
    /// Punch a hole in a file — free blocks in a range without changing file size
    PunchHole {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path
        path: String,
        /// Byte offset of the hole start (e.g. "0", "1M")
        offset: String,
        /// Length of the hole (e.g. "50M")
        length: String,
    },
    /// Defragment a file or the entire volume
    Defrag {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path (omit for --volume)
        path: Option<String>,
        /// Defragment all files on the volume
        #[arg(long)]
        volume: bool,
        /// Report fragmentation without modifying anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Show fragmentation report for a file
    Fragstat {
        #[command(flatten)]
        args: ImageArgs,
        /// CFS file path
        path: String,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a human-readable size string into bytes.
/// Accepts: "1024", "2K", "2KB", "4M", "4MB", "1G", "1GB"
pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty size string");
    }

    // Strip trailing 'B' or 'b' if present (e.g. "2KB" → "2K")
    let s = if s.len() > 1 && s.as_bytes()[s.len() - 1].to_ascii_lowercase() == b'b' {
        let prev = s.as_bytes()[s.len() - 2];
        if prev.is_ascii_alphabetic() {
            &s[..s.len() - 1]
        } else {
            s
        }
    } else {
        s
    };

    let last = s.as_bytes()[s.len() - 1];
    let (num_str, multiplier) = match last.to_ascii_lowercase() {
        b'k' => (&s[..s.len() - 1], 1024u64),
        b'm' => (&s[..s.len() - 1], 1024u64 * 1024),
        b'g' => (&s[..s.len() - 1], 1024u64 * 1024 * 1024),
        _ => (s, 1u64),
    };

    let num: u64 = num_str.parse()
        .map_err(|_| anyhow::anyhow!("invalid size: {}", s))?;
    let result = num.checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("size overflow: {}", s))?;

    if result == 0 {
        bail!("size must be greater than zero");
    }
    Ok(result)
}

/// Returns true if `path` looks like a raw device path (`\\.\X:`, `\\.\PhysicalDriveN`, etc.).
pub fn is_raw_device_path(path: &str) -> bool {
    path.starts_with(r"\\.\")
        || path.starts_with(r"\\?\")
}

/// Open the appropriate block device for `image`.
///
/// If `image` starts with `\\.\` (e.g. `\\.\Z:`), a `RawPartitionBlockDevice`
/// is opened.  Otherwise a `FileBlockDevice` is used.
///
/// `create_size` is only used for `FileBlockDevice` (create a new backing
/// file of that size); raw partitions always open an existing device.
pub fn open_device(image: &str, create_size: Option<u64>) -> Result<Box<dyn CFSBlockDevice>> {
    if is_raw_device_path(image) {
        Ok(Box::new(RawPartitionBlockDevice::open(image)?))
    } else {
        Ok(Box::new(FileBlockDevice::open(Path::new(image), create_size)?))
    }
}

/// Open an existing CFS volume from an image file or raw partition.
pub fn open_volume(image: &str, block_size: u32) -> Result<CFSVolume> {
    let dev = open_device(image, None)?;
    CFSVolume::mount(dev, block_size)
}

/// Auto-detect whether a volume is encrypted and open it accordingly.
///
/// If the first 4 bytes are "CFSE", prompts for a password and wraps the
/// device in `EncryptedBlockDevice`. Otherwise opens directly.
pub fn auto_open_volume(image: &str, block_size: u32) -> Result<CFSVolume> {
    let mut dev = open_device(image, None)?;

    if crypto::is_encrypted_device(&mut *dev)? {
        let mut password = prompt_password("Password: ")?;
        let result = EncryptedBlockDevice::open_encrypted(dev, password.as_bytes());
        password.zeroize();
        let enc = result?;
        CFSVolume::mount(Box::new(enc), block_size)
    } else {
        CFSVolume::mount(dev, block_size)
    }
}

/// Auto-detect and open volume with an explicit password (for testing).
pub fn auto_open_volume_with_password(
    image: &str,
    block_size: u32,
    password: Option<&str>,
) -> Result<CFSVolume> {
    let mut dev = open_device(image, None)?;

    if crypto::is_encrypted_device(&mut *dev)? {
        let pw = password.ok_or_else(|| anyhow::anyhow!("encrypted volume requires a password"))?;
        let enc = EncryptedBlockDevice::open_encrypted(dev, pw.as_bytes())?;
        CFSVolume::mount(Box::new(enc), block_size)
    } else {
        CFSVolume::mount(dev, block_size)
    }
}

/// Prompt user for a password with no terminal echo.
pub fn prompt_password(prompt: &str) -> Result<String> {
    let pw = rpassword::prompt_password(prompt)
        .map_err(|e| anyhow::anyhow!("failed to read password: {}", e))?;
    if pw.is_empty() {
        bail!("password cannot be empty");
    }
    Ok(pw)
}

/// Format a mode value as a human-readable string.
pub fn format_mode(mode: u16) -> &'static str {
    match mode {
        INODE_UNUSED => "UNUSED",
        INODE_FILE => "FILE",
        INODE_DIR => "DIR",
        INODE_SYMLINK => "SYMLINK",
        _ => "UNKNOWN",
    }
}

/// Format a byte count as a human-readable string.
pub fn format_size_human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Format a Unix timestamp as a raw seconds value.
pub fn format_timestamp(ts: u64) -> String {
    format!("{ts}")
}

/// Format a nanosecond timestamp for display.
/// Attempts human-readable output; falls back to raw ns.
pub fn format_timestamp_ns(ns: u64) -> String {
    if ns == 0 {
        return "0 (unset)".to_string();
    }
    let secs = ns / 1_000_000_000;
    let subsec_ns = ns % 1_000_000_000;
    let duration = std::time::Duration::new(secs, subsec_ns as u32);
    let time = std::time::UNIX_EPOCH + duration;
    // Format as ISO8601-ish with full nanosecond precision
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => {
            let total_secs = d.as_secs();
            let nanos = d.subsec_nanos();
            // Simple breakdown: YYYY-MM-DD HH:MM:SS.nnnnnnnnn
            // We use a rough manual approach since we don't have chrono
            let days_since_epoch = total_secs / 86400;
            let time_of_day = total_secs % 86400;
            let hours = time_of_day / 3600;
            let minutes = (time_of_day % 3600) / 60;
            let seconds = time_of_day % 60;

            // Convert days to y/m/d (simplified civil date from days since 1970-01-01)
            let (y, m, d) = days_to_civil(days_since_epoch as i64);
            format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02}.{nanos:09}")
        }
        Err(_) => format!("{ns} ns"),
    }
}

/// Convert days since 1970-01-01 to (year, month, day). Civil calendar.
fn days_to_civil(days: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant's date library
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Format permission bits as a Unix-style string (e.g., "rwxr-xr-x").
pub fn format_permissions(perms: u32) -> String {
    let mut s = String::with_capacity(9);

    // Owner
    s.push(if perms & 0o400 != 0 { 'r' } else { '-' });
    s.push(if perms & 0o200 != 0 { 'w' } else { '-' });
    s.push(match (perms & 0o4000 != 0, perms & 0o100 != 0) {
        (true, true) => 's',
        (true, false) => 'S',
        (false, true) => 'x',
        (false, false) => '-',
    });

    // Group
    s.push(if perms & 0o040 != 0 { 'r' } else { '-' });
    s.push(if perms & 0o020 != 0 { 'w' } else { '-' });
    s.push(match (perms & 0o2000 != 0, perms & 0o010 != 0) {
        (true, true) => 's',
        (true, false) => 'S',
        (false, true) => 'x',
        (false, false) => '-',
    });

    // Other
    s.push(if perms & 0o004 != 0 { 'r' } else { '-' });
    s.push(if perms & 0o002 != 0 { 'w' } else { '-' });
    s.push(match (perms & 0o1000 != 0, perms & 0o001 != 0) {
        (true, true) => 't',
        (true, false) => 'T',
        (false, true) => 'x',
        (false, false) => '-',
    });

    s
}

/// Parse a Unix-style permission string or octal number.
/// Accepts: "rwxr-xr-x", "755", "0755", "0o755"
pub fn parse_permissions(s: &str) -> Result<u32> {
    // Try octal number first
    let s_clean = s.trim_start_matches("0o").trim_start_matches("0O");
    // If original starts with 0o or remaining is all digits, parse as octal
    if s.starts_with("0o") || s.starts_with("0O") || s_clean.chars().all(|c| c.is_ascii_digit()) {
        let digits = if s_clean.is_empty() { "0" } else { s_clean };
        if let Ok(val) = u32::from_str_radix(digits, 8) {
            if val > 0o7777 {
                bail!("permissions out of range: 0o{:o}", val);
            }
            return Ok(val);
        }
    }

    // Try symbolic string (exactly 9 characters: rwxrwxrwx)
    if s.len() == 9 {
        let chars: Vec<char> = s.chars().collect();
        let mut perms = 0u32;

        if chars[0] == 'r' { perms |= 0o400; }
        if chars[1] == 'w' { perms |= 0o200; }
        match chars[2] {
            'x' => perms |= 0o100,
            's' => perms |= 0o100 | 0o4000,
            'S' => perms |= 0o4000,
            _ => {}
        }
        if chars[3] == 'r' { perms |= 0o040; }
        if chars[4] == 'w' { perms |= 0o020; }
        match chars[5] {
            'x' => perms |= 0o010,
            's' => perms |= 0o010 | 0o2000,
            'S' => perms |= 0o2000,
            _ => {}
        }
        if chars[6] == 'r' { perms |= 0o004; }
        if chars[7] == 'w' { perms |= 0o002; }
        match chars[8] {
            'x' => perms |= 0o001,
            't' => perms |= 0o001 | 0o1000,
            'T' => perms |= 0o1000,
            _ => {}
        }

        return Ok(perms);
    }

    bail!("cannot parse permissions: '{}'", s);
}

/// Parse CLI KDF flags into a `KdfParams`.
pub fn parse_kdf_params(
    kdf: &str,
    pbkdf2_iters: u32,
    argon2_memory_mib: u32,
    argon2_time: u32,
    argon2_parallelism: u32,
) -> Result<KdfParams> {
    match kdf.to_ascii_lowercase().as_str() {
        "pbkdf2" | "pbkdf2-hmac-sha256" => Ok(KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: pbkdf2_iters,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }),
        "argon2id" | "argon2" => Ok(KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: argon2_memory_mib * 1024,
            argon2_time_cost: argon2_time,
            argon2_parallelism,
        }),
        other => bail!("unknown KDF algorithm: '{other}' (expected: argon2id, pbkdf2)"),
    }
}

/// Build a `FormatOptions` from CLI arguments.
///
/// Starts from a preset (if given), then overrides with individual flags.
pub fn build_format_options(
    block_size: u32,
    inode_size: Option<u32>,
    inode_ratio: Option<u32>,
    journal: Option<f32>,
    label: Option<&str>,
    preset: Option<&str>,
    secure_delete: Option<bool>,
    default_perms: Option<&str>,
    error_behavior: Option<&str>,
    blocks_per_group: Option<u32>,
) -> Result<FormatOptions> {
    // Start from preset or default
    let mut opts = match preset {
        Some("general") | None => FormatOptions::default(),
        Some("large-files") => FormatOptions::large_files(),
        Some("small-files") => FormatOptions::small_files(),
        Some("max-security") => FormatOptions::max_security(),
        Some("minimal") => FormatOptions::minimal_legacy(),
        Some(other) => bail!("unknown preset: '{}' (expected: general, large-files, small-files, max-security, minimal)", other),
    };

    // Override block_size (always from CLI — has a default of 4096)
    opts.block_size = block_size;

    // If blocks_per_group not explicitly provided, auto-derive from block_size
    if blocks_per_group.is_none() {
        opts.blocks_per_group = block_size * 8;
    }

    // Override individual fields if provided
    if let Some(is) = inode_size {
        opts.inode_size = is;
    }
    if let Some(ir) = inode_ratio {
        opts.inode_ratio = ir;
    }
    if let Some(j) = journal {
        opts.journal_percent = j;
    }
    if let Some(l) = label {
        opts.volume_label = l.to_string();
    }
    if let Some(sd) = secure_delete {
        opts.secure_delete = sd;
    }
    if let Some(perms_str) = default_perms {
        let perms = u32::from_str_radix(perms_str, 8)
            .map_err(|_| anyhow::anyhow!("invalid octal permissions: '{}'", perms_str))?;
        opts.default_permissions = perms;
    }
    if let Some(eb) = error_behavior {
        opts.error_behavior = match eb {
            "continue" => ErrorBehavior::Continue,
            "read-only" | "readonly" => ErrorBehavior::ReadOnly,
            other => bail!("unknown error behavior: '{}' (expected: continue, read-only)", other),
        };
    }
    if let Some(bpg) = blocks_per_group {
        opts.blocks_per_group = bpg;
    }

    opts.validate()?;
    Ok(opts)
}

/// Build `MountOptions` from CLI arguments.
pub fn build_mount_options(
    cache_inodes: Option<u32>,
    cache_blocks: Option<u32>,
    atime: Option<&str>,
    secure_delete: Option<bool>,
    read_only: bool,
) -> Result<MountOptions> {
    let mut opts = MountOptions::default();
    if let Some(ci) = cache_inodes {
        opts.cache_inodes = ci;
    }
    if let Some(cb) = cache_blocks {
        opts.cache_blocks = cb;
    }
    if let Some(atime_str) = atime {
        opts.atime_mode = match atime_str {
            "always" => AtimeMode::Always,
            "relatime" => AtimeMode::Relatime,
            "never" => AtimeMode::Never,
            other => bail!("unknown atime mode: '{}' (expected: always, relatime, never)", other),
        };
    }
    if let Some(sd) = secure_delete {
        opts.secure_delete = sd;
    }
    if read_only {
        opts.read_only = true;
    }
    Ok(opts)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Format { image, size, block_size, encrypted, kdf, pbkdf2_iters, argon2_memory, argon2_time, argon2_parallelism, inode_size, inode_ratio, journal, label, preset, secure_delete, default_perms, error_behavior, blocks_per_group } => {
            let kdf_params = parse_kdf_params(&kdf, pbkdf2_iters, argon2_memory, argon2_time, argon2_parallelism)?;
            let format_opts = build_format_options(
                block_size,
                inode_size,
                inode_ratio,
                journal,
                label.as_deref(),
                preset.as_deref(),
                secure_delete,
                default_perms.as_deref(),
                error_behavior.as_deref(),
                blocks_per_group,
            )?;
            commands::cmd_format(&image, &size, encrypted, &kdf_params, &format_opts)
        }
        Commands::Info { args } => {
            commands::cmd_info(&args.image, args.block_size)
        }
        Commands::Ls { args, path, all, long } => {
            commands::cmd_ls(&args.image, args.block_size, &path, all, long)
        }
        Commands::Mkdir { args, path, parents } => {
            commands::cmd_mkdir(&args.image, args.block_size, &path, parents)
        }
        Commands::Cat { args, path } => {
            commands::cmd_cat(&args.image, args.block_size, &path, &mut std::io::stdout())
        }
        Commands::Write { args, path, data, from_file } => {
            commands::cmd_write(&args.image, args.block_size, &path, data.as_deref(), from_file.as_deref())
        }
        Commands::Rm { args, path } => {
            commands::cmd_rm(&args.image, args.block_size, &path)
        }
        Commands::Rmdir { args, path } => {
            commands::cmd_rmdir(&args.image, args.block_size, &path)
        }
        Commands::Stat { args, path } => {
            commands::cmd_stat(&args.image, args.block_size, &path)
        }
        Commands::Mv { args, old_path, new_path } => {
            commands::cmd_mv(&args.image, args.block_size, &old_path, &new_path)
        }
        Commands::Cp { args, src, dest } => {
            commands::cmd_cp(&args.image, args.block_size, &src, &dest)
        }
        Commands::Import { args, host_file, cfs_path } => {
            commands::cmd_import(&args.image, args.block_size, &host_file, &cfs_path)
        }
        Commands::Export { args, cfs_path, host_file } => {
            commands::cmd_export(&args.image, args.block_size, &cfs_path, &host_file)
        }
        Commands::Tree { args, path } => {
            commands::cmd_tree(&args.image, args.block_size, &path, &mut std::io::stdout())
        }
        Commands::Mount { args, mount_point, cache_inodes, cache_blocks, atime, secure_delete, read_only, threads } => {
            let _mount_opts = build_mount_options(
                cache_inodes,
                cache_blocks,
                atime.as_deref(),
                secure_delete,
                read_only,
            )?;
            // TODO: pass mount_opts to fuse layer when it supports v3
            crate::fuse::cmd_mount(&args.image, args.block_size, mount_point.as_deref(), threads)
        }
        Commands::Passwd { args, kdf, pbkdf2_iters, argon2_memory, argon2_time, argon2_parallelism } => {
            let kdf_params = match kdf {
                Some(ref k) => Some(parse_kdf_params(
                    k,
                    pbkdf2_iters.unwrap_or(600_000),
                    argon2_memory.unwrap_or(32),
                    argon2_time.unwrap_or(2),
                    argon2_parallelism.unwrap_or(1),
                )?),
                None => None,
            };
            commands::cmd_passwd(&args.image, args.block_size, kdf_params)
        }
        Commands::BenchKdf { kdf, pbkdf2_iters, argon2_memory, argon2_time, argon2_parallelism } => {
            let kdf_params = parse_kdf_params(&kdf, pbkdf2_iters, argon2_memory, argon2_time, argon2_parallelism)?;
            commands::cmd_bench_kdf(&kdf_params)
        }
        Commands::Presets => {
            commands::cmd_presets()
        }
        Commands::Chmod { args, path, mode } => {
            commands::cmd_chmod(&args.image, args.block_size, &path, &mode)
        }
        Commands::Chown { args, path, owner, group } => {
            commands::cmd_chown(&args.image, args.block_size, &path, owner, group)
        }
        Commands::JournalStatus { args } => {
            commands::cmd_journal_status(&args.image, args.block_size)
        }
        Commands::Ln { args, target, link_path, symbolic } => {
            commands::cmd_ln(&args.image, args.block_size, &target, &link_path, symbolic)
        }
        Commands::Readlink { args, path } => {
            commands::cmd_readlink(&args.image, args.block_size, &path)
        }
        Commands::Xattr { args, op, path, key, value } => {
            commands::cmd_xattr(&args.image, args.block_size, &op, &path, key.as_deref(), value.as_deref())
        }
        Commands::Fallocate { args, path, offset, length } => {
            let off = parse_size(&offset).unwrap_or(0);
            let len = parse_size(&length)?;
            commands::cmd_fallocate(&args.image, args.block_size, &path, off, len)
        }
        Commands::PunchHole { args, path, offset, length } => {
            let off = parse_size(&offset)?;
            let len = parse_size(&length)?;
            commands::cmd_punch_hole(&args.image, args.block_size, &path, off, len)
        }
        Commands::Defrag { args, path, volume, dry_run } => {
            commands::cmd_defrag(&args.image, args.block_size, path.as_deref(), volume, dry_run)
        }
        Commands::Fragstat { args, path } => {
            commands::cmd_fragstat(&args.image, args.block_size, &path)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("512").unwrap(), 512);
    }

    #[test]
    fn test_parse_size_kilobytes() {
        assert_eq!(parse_size("2K").unwrap(), 2048);
        assert_eq!(parse_size("2k").unwrap(), 2048);
        assert_eq!(parse_size("2KB").unwrap(), 2048);
    }

    #[test]
    fn test_parse_size_megabytes() {
        assert_eq!(parse_size("4M").unwrap(), 4_194_304);
        assert_eq!(parse_size("4MB").unwrap(), 4_194_304);
    }

    #[test]
    fn test_parse_size_gigabytes() {
        assert_eq!(parse_size("1G").unwrap(), 1_073_741_824);
        assert_eq!(parse_size("1GB").unwrap(), 1_073_741_824);
    }

    #[test]
    fn test_parse_size_invalid() {
        assert!(parse_size("abc").is_err());
        assert!(parse_size("").is_err());
        assert!(parse_size("0K").is_err());
        assert!(parse_size("0").is_err());
    }

    #[test]
    fn test_format_mode() {
        assert_eq!(format_mode(INODE_FILE), "FILE");
        assert_eq!(format_mode(INODE_DIR), "DIR");
        assert_eq!(format_mode(INODE_UNUSED), "UNUSED");
        assert_eq!(format_mode(99), "UNKNOWN");
    }
}
