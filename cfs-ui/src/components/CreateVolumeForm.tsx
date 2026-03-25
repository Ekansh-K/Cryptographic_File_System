import { useState, useEffect } from "react";
import { useAppStore } from "../store";
import { getDiskFreeSpace, listVolumeFiles } from "../commands";
import AdvancedSettingsModal from "./AdvancedSettingsModal";
import type { FormatOptionsDto } from "../types";

interface Props {
  onCreated: () => void;
  onCancel: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${bytes} B`;
}

/** Build a unique .img path in dir that doesn't collide with existing files. */
function uniqueVolumePath(dir: string, existingNames: string[]): string {
  const sep = "\\";
  const base = "volume";
  if (!existingNames.includes(`${base}.img`)) {
    return `${dir}${sep}${base}.img`;
  }
  // Try volume-2, volume-3, …
  let n = 2;
  while (n < 1000) {
    const name = `${base}-${n}.img`;
    if (!existingNames.includes(name)) return `${dir}${sep}${name}`;
    n++;
  }
  // Fallback: timestamp
  return `${dir}${sep}${base}-${Date.now()}.img`;
}

const SIZE_OPTIONS = ["64 MB", "128 MB", "256 MB", "512 MB", "1 GB", "Custom"];

/** Extract just the stem (no directory, no .img) from a full path. */
function stemFromPath(fullPath: string): string {
  const name = fullPath.split(/[/\\]/).pop() ?? "";
  return name.endsWith(".img") ? name.slice(0, -4) : name;
}

/** Rebuild full path from dir + stem. */
function buildPath(dir: string, stem: string): string {
  const clean = stem.trim().replace(/[\\/:*?"<>|]/g, "_") || "volume";
  return `${dir}\\${clean}.img`;
}

export default function CreateVolumeForm({ onCreated, onCancel }: Props) {
  const [volumeName, setVolumeName] = useState("");
  const [dir, setDir] = useState("");
  const [size, setSize] = useState("256 MB");
  const [customSize, setCustomSize] = useState("");
  const [customUnit, setCustomUnit] = useState("MB");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [pbkdf2Iters, setPbkdf2Iters] = useState(300_000);
  const [kdf, setKdf] = useState<"argon2id" | "pbkdf2">("argon2id");
  const [argon2MemoryMib, setArgon2MemoryMib] = useState(32);
  const [argon2Time, setArgon2Time] = useState(2);
  const [argon2Parallelism, setArgon2Parallelism] = useState(1);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [freeSpace, setFreeSpace] = useState<number | null>(null);

  // ── Volume Format options (Phase 10A) ──────────────────────────────────
  const [formatPreset, setFormatPreset] = useState("general");
  const [formatBlockSize, setFormatBlockSize] = useState(4096);
  const [formatInodeSize, setFormatInodeSize] = useState(256);
  const [formatInodeRatio, setFormatInodeRatio] = useState(16384);
  const [formatJournalPercent, setFormatJournalPercent] = useState(1.0);
  const [formatVolumeLabel, setFormatVolumeLabel] = useState("");
  const [formatSecureDelete, setFormatSecureDelete] = useState(true);
  const [formatDefaultPermissions, setFormatDefaultPermissions] = useState(0o755);
  const [formatErrorBehavior, setFormatErrorBehavior] = useState("continue");

  const loading = useAppStore((s) => s.loading);
  const create = useAppStore((s) => s.create);
  const defaultVolumesDir = useAppStore((s) => s.defaultVolumesDir);

  // Derive full path from dir + volumeName
  const path = dir && volumeName.trim() ? buildPath(dir, volumeName) : "";

  // Auto-populate name + dir from default directory
  useEffect(() => {
    if (!defaultVolumesDir) return;
    setDir(defaultVolumesDir);
    listVolumeFiles()
      .then((files) => {
        const existingNames = files.map((f) => f.name);
        const fullPath = uniqueVolumePath(defaultVolumesDir, existingNames);
        setVolumeName(stemFromPath(fullPath));
      })
      .catch(() => setVolumeName("volume"));
  }, [defaultVolumesDir]);

  // Load free disk space for the directory
  useEffect(() => {
    getDiskFreeSpace(dir || undefined)
      .then(setFreeSpace)
      .catch(() => setFreeSpace(null));
  }, [dir]);

  async function handleChangeLocation() {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const defaultPath = path || (dir ? `${dir}\\volume.img` : "volume.img");
    const selected = await save({
      filters: [{ name: "CFS Volume", extensions: ["img"] }],
      defaultPath,
    });
    if (selected) {
      // Update both dir and name from the user's chosen path
      const parts = selected.replace(/\//g, "\\").split("\\");
      const filename = parts.pop() ?? "volume.img";
      setDir(parts.join("\\"));
      setVolumeName(filename.endsWith(".img") ? filename.slice(0, -4) : filename);
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);

    if (!volumeName.trim()) {
      setError("Enter a name for the volume");
      return;
    }
    if (!dir) {
      setError("No save location selected");
      return;
    }
    if (password.length < 8) {
      setError("Password must be at least 8 characters");
      return;
    }
    if (password !== confirm) {
      setError("Passwords do not match");
      return;
    }

    // Build size argument
    let sizeArg: string;
    if (size === "Custom") {
      const num = parseInt(customSize, 10);
      if (!num || num <= 0) {
        setError("Enter a valid custom size");
        return;
      }
      // Construct e.g. "512M" or "2G"
      const unitLetter = customUnit === "GB" ? "G" : "M";
      sizeArg = `${num}${unitLetter}`;
    } else {
      // Convert display size like "256 MB" to "256M"
      sizeArg = size.replace(/\s+/g, "").replace(/B$/i, "");
    }

    // Build FormatOptionsDto from current format settings
    const formatOpts: FormatOptionsDto = {
      preset: formatPreset !== "general" ? formatPreset : undefined,
      block_size: formatBlockSize !== 4096 ? formatBlockSize : undefined,
      inode_size: formatInodeSize !== 256 ? formatInodeSize : undefined,
      inode_ratio: formatInodeRatio !== 16384 ? formatInodeRatio : undefined,
      journal_percent: formatJournalPercent !== 1.0 ? formatJournalPercent : undefined,
      volume_label: formatVolumeLabel || undefined,
      secure_delete: formatSecureDelete !== true ? formatSecureDelete : undefined,
      default_permissions: formatDefaultPermissions !== 0o755 ? formatDefaultPermissions : undefined,
      error_behavior: formatErrorBehavior !== "continue" ? formatErrorBehavior : undefined,
    };
    // Only send formatOpts if any field is set (avoid empty object overhead)
    const hasFormatOpts = Object.values(formatOpts).some((v) => v !== undefined);

    try {
      await create(
        path,
        sizeArg,
        password,
        kdf,
        kdf === "pbkdf2" ? pbkdf2Iters : undefined,
        kdf === "argon2id" ? argon2MemoryMib : undefined,
        kdf === "argon2id" ? argon2Time : undefined,
        kdf === "argon2id" ? argon2Parallelism : undefined,
        hasFormatOpts ? formatOpts : undefined,
      );
      setPassword("");
      setConfirm("");
      onCreated();
    } catch (err) {
      setError(String(err));
      setPassword("");
      setConfirm("");
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3 border border-border p-4 bg-surface">
      <div className="flex items-center justify-between">
        <span className="text-sm text-text-bright">Create Encrypted Volume</span>
        <button
          type="button"
          className="text-sm text-text-muted hover:text-text"
          onClick={onCancel}
        >
          &#x2715;
        </button>
      </div>

      {/* Volume name input */}
      <div className="space-y-1">
        <label className="block text-sm text-text-muted">Volume name</label>
        <div className="flex items-stretch gap-0">
          <input
            type="text"
            value={volumeName}
            onChange={(e) => setVolumeName(e.target.value)}
            className="flex-1 px-2 py-1.5 bg-bg border border-border border-r-0 text-text text-sm focus:border-border-focus focus:z-10 relative"
            placeholder="e.g. my-secrets"
            spellCheck={false}
          />
          <span className="flex items-center px-2 py-1.5 bg-surface border border-border text-text-muted text-sm select-none">
            .img
          </span>
        </div>
        {/* Location row */}
        <div className="flex items-center gap-1 text-xs text-text-muted">
          <span className="truncate" title={dir || undefined}>{dir || "No location selected"}</span>
          <button
            type="button"
            className="shrink-0 text-text-muted hover:text-text underline underline-offset-2"
            onClick={handleChangeLocation}
          >
            change
          </button>
        </div>
      </div>

      {/* Size */}
      <div>
        <label className="block text-sm text-text-muted mb-1">Size</label>
        <select
          value={size}
          onChange={(e) => setSize(e.target.value)}
          className="w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus appearance-none"
        >
          {SIZE_OPTIONS.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
      </div>

      {/* Custom size input */}
      {size === "Custom" && (
        <div className="flex gap-2">
          <input
            type="number"
            min="1"
            value={customSize}
            onChange={(e) => setCustomSize(e.target.value)}
            className="flex-1 px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus"
            placeholder="e.g. 512"
          />
          <select
            value={customUnit}
            onChange={(e) => setCustomUnit(e.target.value)}
            className="px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus appearance-none"
          >
            <option value="MB">MB</option>
            <option value="GB">GB</option>
          </select>
        </div>
      )}

      {/* Free disk space */}
      {freeSpace !== null && (
        <div className="text-xs text-text-muted">
          Available disk space: {formatBytes(freeSpace)}
        </div>
      )}

      {/* Encryption badge + Customize button */}
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-xs text-text-muted">
          <span className="text-success">&#x25CF;</span>
          <span>AES-256-XTS + {kdf === "argon2id" ? "Argon2id" : "PBKDF2"}</span>
          {formatPreset !== "general" && (
            <>
              <span className="text-text-muted">│</span>
              <span className="text-text-muted capitalize">{formatPreset.replace(/-/g, " ")} preset</span>
            </>
          )}
        </div>
        <button
          type="button"
          onClick={() => setShowAdvanced(true)}
          className="text-xs text-text-muted hover:text-text underline underline-offset-2"
        >
          ⚙ Customize encryption &amp; volume format
        </button>
      </div>

      {/* Password */}
      <div>
        <label className="block text-sm text-text-muted mb-1">Password</label>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus"
          placeholder="Minimum 8 characters"
        />
      </div>

      {/* Confirm */}
      <div>
        <label className="block text-sm text-text-muted mb-1">Confirm password</label>
        <input
          type="password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          className="w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus"
          placeholder="Re-enter password"
        />
      </div>

      {/* Error */}
      {error && <div className="text-sm text-error">{error}</div>}

      {/* Submit */}
      <button
        type="submit"
        disabled={loading || !volumeName.trim() || !dir}
        className="w-full py-2 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
      >
        {loading ? "Creating..." : "Create Volume"}
      </button>
      {/* Advanced Settings Modal */}
      <AdvancedSettingsModal
        open={showAdvanced}
        onClose={() => setShowAdvanced(false)}
        kdf={kdf}
        setKdf={setKdf}
        argon2MemoryMib={argon2MemoryMib}
        setArgon2MemoryMib={setArgon2MemoryMib}
        argon2Time={argon2Time}
        setArgon2Time={setArgon2Time}
        argon2Parallelism={argon2Parallelism}
        setArgon2Parallelism={setArgon2Parallelism}
        pbkdf2Iters={pbkdf2Iters}
        setPbkdf2Iters={setPbkdf2Iters}
        setFormatPreset={setFormatPreset}
        formatBlockSize={formatBlockSize}
        setFormatBlockSize={setFormatBlockSize}
        formatInodeSize={formatInodeSize}
        setFormatInodeSize={setFormatInodeSize}
        formatInodeRatio={formatInodeRatio}
        setFormatInodeRatio={setFormatInodeRatio}
        formatJournalPercent={formatJournalPercent}
        setFormatJournalPercent={setFormatJournalPercent}
        formatVolumeLabel={formatVolumeLabel}
        setFormatVolumeLabel={setFormatVolumeLabel}
        formatSecureDelete={formatSecureDelete}
        setFormatSecureDelete={setFormatSecureDelete}
        formatDefaultPermissions={formatDefaultPermissions}
        setFormatDefaultPermissions={setFormatDefaultPermissions}
        formatErrorBehavior={formatErrorBehavior}
        setFormatErrorBehavior={setFormatErrorBehavior}
      />
    </form>
  );
}
