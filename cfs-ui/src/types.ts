/** Matches the Rust DetectResult DTO */
export interface DetectResult {
  exists: boolean;
  is_encrypted: boolean;
  size_bytes: number;
}

/** Matches the Rust VolumeInfo DTO */
export interface VolumeInfo {
  path: string;
  is_encrypted: boolean;
  block_size: number;
  total_blocks: number;
  free_blocks: number;
  inode_count: number;
  total_size: number;
  free_size: number;
  // v3 fields
  version: number;
  inode_size: number;
  feature_flags: number;
  block_groups: number;
  journal_blocks: number;
  volume_label: string;
  error_behavior: string;
  default_permissions: number;
}

/** Matches the Rust DirEntryDto */
export interface DirEntryDto {
  name: string;
  inode_index: number;
  file_type: "file" | "directory";
  size: number;
  modified: number;
  created: number;
}

/** Matches the Rust InodeDto */
export interface InodeDto {
  file_type: "file" | "directory" | "unused";
  size: number;
  block_count: number;
  nlinks: number;
  created: number;
  modified: number;
  direct_blocks: number[];
  has_indirect: boolean;
  has_double_indirect: boolean;
}

/** Matches the Rust FilePreview DTO */
export interface FilePreview {
  data_base64: string;
  is_text: boolean;
  total_size: number;
  truncated: boolean;
}

/** Matches the Rust AppStatus DTO */
export interface AppStatus {
  volume_loaded: boolean;
  volume_path: string | null;
  is_encrypted: boolean;
  is_mounted: boolean;
  drive_letter: string | null;
}

/** Matches the Rust MountInfo DTO */
export interface MountInfo {
  drive_letter: string;
  mounted: boolean;
}

/** Matches the Rust RawPartitionInfo DTO */
export interface RawPartitionInfo {
  device_path: string;
  drive_letter: string;
  size_bytes: number;
  is_cfs: boolean;
  is_encrypted: boolean;
}

/** Matches the Rust VolumeFileDto */
export interface VolumeFileDto {
  path: string;
  name: string;
  size_bytes: number;
  is_encrypted: boolean;
}

/** Format options passed to create_volume (all optional; server applies defaults) */
export interface FormatOptionsDto {
  block_size?: number;
  inode_size?: number;
  inode_ratio?: number;
  journal_percent?: number;
  volume_label?: string;
  secure_delete?: boolean;
  default_permissions?: number;
  error_behavior?: string;
  blocks_per_group?: number;
  preset?: string;
}

/** Matches the Rust IoBenchmarkResult DTO */
export interface IoBenchmarkResult {
  size_label: string;
  size_bytes: number;
  write_speed_mbps: number;
  read_speed_mbps: number;
  write_time_ms: number;
  read_time_ms: number;
  sync_time_ms: number;
}
