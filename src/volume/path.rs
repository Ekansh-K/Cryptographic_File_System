use anyhow::{bail, Result};

use crate::block_device::CFSBlockDevice;
use super::dir;
use super::file_io;
use super::inode::{InodeTable, INODE_FLAG_INLINE_DATA};
use super::superblock::Superblock;
use super::{INODE_DIR, INODE_SYMLINK, ROOT_INODE};

/// Maximum number of symlinks followed in a single path resolution.
const MAX_SYMLINK_DEPTH: u32 = 40;

/// Resolve an absolute path to its inode index, following symlinks.
pub fn resolve_path(
    dev: &mut dyn CFSBlockDevice,
    inode_table: &InodeTable,
    sb: &Superblock,
    path: &str,
) -> Result<u32> {
    resolve_path_inner(dev, inode_table, sb, path, true, 0)
}

/// Resolve path WITHOUT following the final symlink component.
/// Used by readlink(), lstat(), and rename().
pub fn resolve_path_no_final_follow(
    dev: &mut dyn CFSBlockDevice,
    inode_table: &InodeTable,
    sb: &Superblock,
    path: &str,
) -> Result<u32> {
    resolve_path_inner(dev, inode_table, sb, path, false, 0)
}

fn resolve_path_inner(
    dev: &mut dyn CFSBlockDevice,
    inode_table: &InodeTable,
    sb: &Superblock,
    path: &str,
    follow_final: bool,
    mut symlink_count: u32,
) -> Result<u32> {
    if path.is_empty() {
        bail!("empty path");
    }
    if !path.starts_with('/') {
        bail!("path must be absolute (start with '/')");
    }
    if path == "/" {
        return Ok(ROOT_INODE);
    }

    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let mut current_inode = ROOT_INODE;

    for (i, component) in components.iter().enumerate() {
        let is_last = i == components.len() - 1;

        // Current inode must be a directory
        let parent = inode_table.read_inode(dev, current_inode)?;
        if parent.mode != INODE_DIR {
            bail!("not a directory in path '{}'", path);
        }

        match dir::lookup_dispatch(dev, &parent, current_inode, sb, component)? {
            Some(entry) => current_inode = entry.inode_index,
            None => bail!("not found: '{}' in path '{}'", component, path),
        }

        // Check if resolved inode is a symlink
        let inode = inode_table.read_inode(dev, current_inode)?;
        if inode.mode == INODE_SYMLINK && (follow_final || !is_last) {
            symlink_count += 1;
            if symlink_count > MAX_SYMLINK_DEPTH {
                bail!("too many levels of symbolic links (max {})", MAX_SYMLINK_DEPTH);
            }

            // Read symlink target
            let target = readlink_from_inode(dev, &inode, sb.block_size)?;

            // Build new path with remaining components appended
            let remaining: String = components[i + 1..]
                .iter()
                .map(|c| format!("/{}", c))
                .collect();

            let new_path = if target.starts_with('/') {
                // Absolute symlink: restart from root
                format!("{}{}", target, remaining)
            } else {
                // Relative symlink: resolve from parent directory
                let parent_path: String = if i == 0 {
                    String::new()
                } else {
                    components[..i].iter().map(|c| format!("/{}", c)).collect()
                };
                if parent_path.is_empty() {
                    format!("/{}{}", target, remaining)
                } else {
                    format!("{}/{}{}", parent_path, target, remaining)
                }
            };

            return resolve_path_inner(dev, inode_table, sb, &new_path, follow_final, symlink_count);
        }
    }

    Ok(current_inode)
}

/// Read symlink target from an inode (used by path resolution).
fn readlink_from_inode(
    dev: &mut dyn CFSBlockDevice,
    inode: &super::inode::Inode,
    block_size: u32,
) -> Result<String> {
    let target_len = inode.size as usize;

    if inode.flags & INODE_FLAG_INLINE_DATA != 0 {
        // Fast symlink: in inline_area
        let target = &inode.inline_area[..target_len.min(76)];
        String::from_utf8(target.to_vec())
            .map_err(|_| anyhow::anyhow!("symlink target is not valid UTF-8"))
    } else {
        // Slow symlink: in data block
        let data = file_io::read_data(dev, inode, block_size, 0, target_len as u64)?;
        String::from_utf8(data)
            .map_err(|_| anyhow::anyhow!("symlink target is not valid UTF-8"))
    }
}

/// Resolve the parent directory of a path and return (parent_inode, final_name).
/// Follows symlinks in intermediate components but not the final one.
pub fn resolve_parent(
    dev: &mut dyn CFSBlockDevice,
    inode_table: &InodeTable,
    sb: &Superblock,
    path: &str,
) -> Result<(u32, String)> {
    if path.is_empty() {
        bail!("empty path");
    }
    if path == "/" {
        bail!("cannot get parent of root");
    }

    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if components.is_empty() {
        bail!("invalid path");
    }

    let final_name = components.last().unwrap().to_string();

    if components.len() == 1 {
        return Ok((ROOT_INODE, final_name));
    }

    // Resolve all but the last component (following symlinks)
    let parent_path = format!("/{}", components[..components.len() - 1].join("/"));
    let parent_inode = resolve_path(dev, inode_table, sb, &parent_path)?;

    // Verify parent is a directory
    let inode = inode_table.read_inode(dev, parent_inode)?;
    if inode.mode != INODE_DIR {
        bail!("parent '{}' is not a directory", parent_path);
    }

    Ok((parent_inode, final_name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::{CFSVolume, DEFAULT_BLOCK_SIZE, INODE_DIR};
    use crate::volume::alloc::BlockAlloc;
    use crate::volume::dir::{init_dir_block, add_dir_entry, DirEntry};
    use crate::volume::inode::Inode;
    use tempfile::NamedTempFile;

    fn make_vol() -> (NamedTempFile, CFSVolume) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();
        let vol = CFSVolume::format(Box::new(dev), DEFAULT_BLOCK_SIZE).unwrap();
        (tmp, vol)
    }

    /// Helper to create a subdirectory manually and return its inode index.
    fn make_subdir(vol: &CFSVolume, parent: u32, name: &str) -> u32 {
        let idx = vol.alloc_inode().unwrap();
        let dir_inode = Inode::new_dir();
        vol.write_inode(idx, &dir_inode).unwrap();

        // Phase 1: init_dir_block
        {
            let mut sb = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut dir_inode = vol.inode_table.read_inode(&mut **dg, idx).unwrap();
            let ds = sb.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            init_dir_block(
                &mut **dg, &mut dir_inode, &mut alloc,
                &mut *sb, idx, parent,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, idx, &dir_inode).unwrap();
        }

        // Phase 2: add entry to parent
        {
            let mut sb = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let entry = DirEntry::new(idx, INODE_DIR as u8, name).unwrap();
            let mut parent_inode = vol.inode_table.read_inode(&mut **dg, parent).unwrap();
            let ds = sb.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut parent_inode, &mut alloc,
                &mut *sb, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, parent, &parent_inode).unwrap();
        }
        idx
    }

    #[test]
    fn test_resolve_root() {
        let (_tmp, vol) = make_vol();
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let idx = resolve_path(
            &mut **dg, &vol.inode_table, &sb, "/",
        ).unwrap();
        assert_eq!(idx, ROOT_INODE);
    }

    #[test]
    fn test_resolve_one_level() {
        let (_tmp, vol) = make_vol();
        let foo_idx = make_subdir(&vol, ROOT_INODE, "foo");
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let resolved = resolve_path(
            &mut **dg, &vol.inode_table, &sb, "/foo",
        ).unwrap();
        assert_eq!(resolved, foo_idx);
    }

    #[test]
    fn test_resolve_nested() {
        let (_tmp, vol) = make_vol();
        let foo_idx = make_subdir(&vol, ROOT_INODE, "foo");
        let bar_idx = make_subdir(&vol, foo_idx, "bar");
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let resolved = resolve_path(
            &mut **dg, &vol.inode_table, &sb, "/foo/bar",
        ).unwrap();
        assert_eq!(resolved, bar_idx);
    }

    #[test]
    fn test_resolve_not_found() {
        let (_tmp, vol) = make_vol();
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let result = resolve_path(
            &mut **dg, &vol.inode_table, &sb, "/nonexistent",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_parent() {
        let (_tmp, vol) = make_vol();
        let foo_idx = make_subdir(&vol, ROOT_INODE, "foo");
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let (parent, name) = resolve_parent(
            &mut **dg, &vol.inode_table, &sb, "/foo/bar.txt",
        ).unwrap();
        assert_eq!(parent, foo_idx);
        assert_eq!(name, "bar.txt");
    }

    #[test]
    fn test_resolve_dotdot() {
        let (_tmp, vol) = make_vol();
        let _foo_idx = make_subdir(&vol, ROOT_INODE, "foo");
        let mut dg = vol.dev();
        let sb = vol.sb_read();
        let resolved = resolve_path(
            &mut **dg, &vol.inode_table, &sb, "/foo/..",
        ).unwrap();
        assert_eq!(resolved, ROOT_INODE);
    }
}