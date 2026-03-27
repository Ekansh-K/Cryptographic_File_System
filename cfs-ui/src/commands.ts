import { invoke } from "@tauri-apps/api/core";
import type {
  DetectResult,
  VolumeInfo,
  DirEntryDto,
  InodeDto,
  FilePreview,
  AppStatus,
  MountInfo,
  RawPartitionInfo,
  VolumeFileDto,
  FormatOptionsDto,
  IoBenchmarkResult,
} from "./types";

export async function detectVolume(path: string): Promise<DetectResult> {
  return invoke("detect_volume", { path });
}

export async function createVolume(
  path: string,
  size: string,
  password: string,
  blockSize?: number,
  pbkdf2Iterations?: number,
  kdf?: string,
  argon2MemoryMib?: number,
  argon2Time?: number,
  argon2Parallelism?: number,
  formatOptions?: FormatOptionsDto
): Promise<VolumeInfo> {
  return invoke("create_volume", {
    path,
    size,
    password,
    blockSize,
    pbkdf2Iterations,
    kdf,
    argon2MemoryMib,
    argon2Time,
    argon2Parallelism,
    formatOptions,
  });
}

export async function benchmarkKdf(
  kdf: string,
  pbkdf2Iterations?: number,
  argon2MemoryMib?: number,
  argon2Time?: number,
  argon2Parallelism?: number
): Promise<number> {
  return invoke("benchmark_kdf", {
    kdf,
    pbkdf2Iterations,
    argon2MemoryMib,
    argon2Time,
    argon2Parallelism,
  });
}

export async function unlockVolume(
  path: string,
  password: string,
  blockSize?: number
): Promise<VolumeInfo> {
  return invoke("unlock_volume", { path, password, blockSize });
}

export async function lockVolume(): Promise<void> {
  return invoke("lock_volume");
}

export async function getVolumeInfo(): Promise<VolumeInfo> {
  return invoke("get_volume_info");
}

export async function getStatus(): Promise<AppStatus> {
  return invoke("get_status");
}

export async function listDir(path: string): Promise<DirEntryDto[]> {
  return invoke("list_dir", { path });
}

export async function statEntry(path: string): Promise<InodeDto> {
  return invoke("stat_entry", { path });
}

export async function readFilePreview(
  path: string,
  maxBytes?: number
): Promise<FilePreview> {
  return invoke("read_file_preview", { path, maxBytes });
}

export async function listRawPartitions(): Promise<RawPartitionInfo[]> {
  return invoke("list_raw_partitions");
}

export async function checkWinfsp(): Promise<boolean> {
  return invoke("check_winfsp");
}

export async function mountDrive(
  driveLetter?: string
): Promise<MountInfo> {
  return invoke("mount_drive", { driveLetter });
}

export async function unmountDrive(): Promise<void> {
  return invoke("unmount_drive");
}

export async function getDefaultVolumesDir(): Promise<string> {
  return invoke("get_default_volumes_dir");
}

export async function listVolumeFiles(dir?: string): Promise<VolumeFileDto[]> {
  return invoke("list_volume_files", { dir });
}

export async function listFreeDriveLetters(): Promise<string[]> {
  return invoke("list_free_drive_letters");
}

export async function getDiskFreeSpace(path?: string): Promise<number> {
  return invoke("get_disk_free_space", { path });
}

export async function benchmarkFormatIo(
  formatOptions: FormatOptionsDto,
  sizeBytes: number,
  label: string,
  runs: number
): Promise<IoBenchmarkResult> {
  return invoke("benchmark_format_io", { formatOptions, sizeBytes, label, runs });
}

export async function cancelBenchmark(): Promise<void> {
  return invoke("cancel_benchmark");
}
