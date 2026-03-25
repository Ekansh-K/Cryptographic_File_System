import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { open } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../store";
import { detectVolume, listRawPartitions, listVolumeFiles } from "../commands";
import CreateVolumeForm from "../components/CreateVolumeForm";
import type { DetectResult, RawPartitionInfo, VolumeFileDto } from "../types";

type Tab = "file" | "partition";

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

export default function UnlockScreen() {
  const [tab, setTab] = useState<Tab>("file");
  const [path, setPath] = useState("");
  const [password, setPassword] = useState("");
  const [detected, setDetected] = useState<DetectResult | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);

  // Existing volumes from default directory
  const [volumeFiles, setVolumeFiles] = useState<VolumeFileDto[]>([]);
  const [volumeFilesLoading, setVolumeFilesLoading] = useState(false);

  // Partition tab state
  const [partitions, setPartitions] = useState<RawPartitionInfo[]>([]);
  const [scanLoading, setScanLoading] = useState(false);
  const [selectedPartition, setSelectedPartition] = useState<RawPartitionInfo | null>(null);
  const [partPassword, setPartPassword] = useState("");
  const [partError, setPartError] = useState<string | null>(null);

  const loading = useAppStore((s) => s.loading);
  const unlock = useAppStore((s) => s.unlock);
  const defaultVolumesDir = useAppStore((s) => s.defaultVolumesDir);
  const loadDefaultDir = useAppStore((s) => s.loadDefaultDir);
  const navigate = useNavigate();

  // Load default dir and scan for existing volumes on mount
  useEffect(() => {
    loadDefaultDir();
  }, [loadDefaultDir]);

  useEffect(() => {
    if (defaultVolumesDir) {
      loadVolumeFiles();
    }
  }, [defaultVolumesDir]);

  async function loadVolumeFiles() {
    setVolumeFilesLoading(true);
    try {
      const files = await listVolumeFiles();
      setVolumeFiles(files);
    } catch {
      setVolumeFiles([]);
    } finally {
      setVolumeFilesLoading(false);
    }
  }

  // Scan partitions when partition tab is selected
  useEffect(() => {
    if (tab === "partition") {
      handleScanPartitions();
    }
  }, [tab]);

  async function handleScanPartitions() {
    setScanLoading(true);
    try {
      const parts = await listRawPartitions();
      setPartitions(parts);
    } catch {
      setPartitions([]);
    } finally {
      setScanLoading(false);
    }
  }

  async function handleSelectVolumeFile(file: VolumeFileDto) {
    setPath(file.path);
    setAuthError(null);
    try {
      const result = await detectVolume(file.path);
      setDetected(result);
    } catch {
      setDetected({
        exists: true,
        is_encrypted: file.is_encrypted,
        size_bytes: file.size_bytes,
      });
    }
  }

  async function handleBrowse() {
    const selected = await open({
      filters: [{ name: "CFS Volume", extensions: ["img"] }],
      multiple: false,
      directory: false,
      defaultPath: defaultVolumesDir ?? undefined,
    });
    if (selected) {
      const filePath = typeof selected === "string" ? selected : selected;
      setPath(filePath);
      setAuthError(null);
      try {
        const result = await detectVolume(filePath);
        setDetected(result);
      } catch {
        setDetected(null);
      }
    }
  }

  async function handleUnlock(e: React.FormEvent) {
    e.preventDefault();
    if (!path || !password) return;
    setAuthError(null);
    try {
      await unlock(path, password);
      navigate("/browse");
    } catch (err) {
      setAuthError(String(err));
    } finally {
      setPassword("");
    }
  }

  async function handlePartitionUnlock(e: React.FormEvent) {
    e.preventDefault();
    if (!selectedPartition) return;
    setPartError(null);
    try {
      await unlock(selectedPartition.device_path, partPassword);
      navigate("/browse");
    } catch (err) {
      setPartError(String(err));
    } finally {
      setPartPassword("");
    }
  }

  function handleCreated() {
    setShowCreate(false);
    navigate("/browse");
  }

  return (
    <div className="flex flex-col items-center justify-center h-full px-4 overflow-y-auto">
      <div className="w-full max-w-md space-y-6 py-4">
        {/* Logo */}
        <div className="text-center py-4">
          <div className="inline-block border border-border px-6 py-2">
            <span className="text-xl text-text-bright">&#x2302; CFS</span>
          </div>
        </div>

        {/* Tab bar */}
        <div className="flex border border-border">
          <button
            className={`flex-1 py-2 text-sm text-center ${
              tab === "file"
                ? "bg-surface-active text-text-bright border-r border-border"
                : "bg-surface text-text-muted border-r border-border hover:bg-surface-hover"
            }`}
            onClick={() => setTab("file")}
          >
            &#xF0A0; Image File
          </button>
          <button
            className={`flex-1 py-2 text-sm text-center ${
              tab === "partition"
                ? "bg-surface-active text-text-bright"
                : "bg-surface text-text-muted hover:bg-surface-hover"
            }`}
            onClick={() => setTab("partition")}
          >
            &#x2580; Raw Partition
          </button>
        </div>

        {/* File tab */}
        {tab === "file" && (
          <>
            {/* Default directory info */}
            {defaultVolumesDir && (
              <div className="text-xs text-text-muted px-1">
                Volume storage: <span className="text-text">{defaultVolumesDir}</span>
              </div>
            )}

            {/* Existing volume files */}
            {volumeFiles.length > 0 && (
              <div className="space-y-1">
                <span className="text-xs text-text-muted px-1">Recent volumes</span>
                <div className="border border-border max-h-[140px] overflow-y-auto">
                  {volumeFiles.map((file) => (
                    <button
                      key={file.path}
                      className={`w-full flex items-center justify-between px-3 py-1.5 text-sm border-b border-border last:border-b-0 ${
                        path === file.path
                          ? "bg-surface-active text-text-bright"
                          : "bg-surface text-text hover:bg-surface-hover"
                      }`}
                      onClick={() => handleSelectVolumeFile(file)}
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <span className="shrink-0">&#xF0A0;</span>
                        <span className="truncate">{file.name}</span>
                      </div>
                      <div className="flex items-center gap-2 shrink-0 ml-2">
                        <span className="text-text-muted text-xs">{formatBytes(file.size_bytes)}</span>
                        <span className={file.is_encrypted ? "text-success text-xs" : "text-text-muted text-xs"}>
                          {file.is_encrypted ? "CFSE" : "CFS1"}
                        </span>
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {volumeFilesLoading && (
              <div className="text-xs text-text-muted text-center py-2">Scanning for volumes...</div>
            )}

            {/* Unlock form */}
            <form onSubmit={handleUnlock} className="space-y-4">
              {/* File picker row */}
              <div className="flex gap-2">
                <button
                  type="button"
                  className="px-3 py-1.5 text-sm border border-border bg-surface hover:border-border-focus hover:bg-surface-hover text-text shrink-0"
                  onClick={handleBrowse}
                >
                  Browse...
                </button>
                <div className="flex-1 flex items-center px-2 py-1.5 bg-surface border border-border text-sm text-text-muted overflow-hidden">
                  <span className="truncate">{path || "No file selected"}</span>
                </div>
              </div>

              {/* Detection badge */}
              {detected && (
                <div className="flex items-center gap-2 text-sm">
                  <span className={detected.is_encrypted ? "text-success" : "text-text-muted"}>
                    {detected.is_encrypted ? "\u25CF CFSE" : "CFS1"}
                  </span>
                  <span className="text-text-muted">│</span>
                  <span className="text-text-muted">{formatBytes(detected.size_bytes)}</span>
                  {detected.is_encrypted && (
                    <>
                      <span className="text-text-muted">│</span>
                      <span className="text-xs text-text-muted">AES-256-XTS Encrypted</span>
                    </>
                  )}
                </div>
              )}

              {/* Password — only shown once a file is selected */}
              {path && (
              <div>
                <label className="block text-sm text-text-muted mb-1">Password</label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full px-2 py-1.5 bg-surface border border-border text-text text-sm focus:border-border-focus"
                  placeholder="Enter password..."
                />
              </div>
              )}

              {/* Auth error */}
              {authError && (
                <div className="text-sm text-error">{authError}</div>
              )}

              {/* Unlock button */}
              <button
                type="submit"
                disabled={!path || (detected?.is_encrypted !== false && !password) || loading}
                className="w-full py-2 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
              >
                {loading ? "Unlocking..." : "Unlock Volume"}
              </button>
            </form>

            {/* Divider */}
            <div className="flex items-center gap-3 text-text-muted text-sm">
              <div className="flex-1 border-t border-border" />
              <span>or</span>
              <div className="flex-1 border-t border-border" />
            </div>

            {/* Create toggle */}
            {!showCreate ? (
              <button
                className="w-full py-2 text-sm border border-border bg-surface text-text-muted hover:border-border-focus hover:text-text"
                onClick={() => setShowCreate(true)}
              >
                Create New Volume
              </button>
            ) : (
              <CreateVolumeForm
                onCreated={handleCreated}
                onCancel={() => setShowCreate(false)}
              />
            )}
          </>
        )}

        {/* Partition tab */}
        {tab === "partition" && (
          <div className="space-y-4">
            {/* Info note about partitions */}
            <div className="text-xs text-text-muted border border-border bg-surface p-2">
              Raw partitions are for advanced users. For most use cases, Image File
              volumes (above tab) are recommended — they are portable, safe, and don't
              require administrator privileges. Creating new partitions from free disk
              space requires Windows Disk Management.
            </div>

            <div className="flex items-center justify-between">
              <span className="text-sm text-text-muted">
                Detected raw partitions
              </span>
              <button
                type="button"
                disabled={scanLoading}
                className="px-2 py-1 text-xs border border-border bg-surface text-text-muted hover:border-border-focus hover:text-text disabled:opacity-40"
                onClick={handleScanPartitions}
              >
                {scanLoading ? "Scanning..." : "Rescan"}
              </button>
            </div>

            {partitions.length === 0 && !scanLoading && (
              <div className="text-sm text-text-muted text-center py-6 border border-border bg-surface">
                No raw partitions found.
                <br />
                <span className="text-xs">
                  RAW/unformatted partitions with CFS volumes will appear here.
                </span>
              </div>
            )}

            {scanLoading && (
              <div className="text-sm text-text-muted text-center py-6 border border-border bg-surface">
                Scanning drives...
              </div>
            )}

            {partitions.length > 0 && (
              <div className="border border-border">
                {partitions.map((part) => (
                  <button
                    key={part.device_path}
                    className={`w-full flex items-center justify-between px-3 py-2 text-sm border-b border-border last:border-b-0 ${
                      selectedPartition?.device_path === part.device_path
                        ? "bg-surface-active text-text-bright"
                        : "bg-surface text-text hover:bg-surface-hover"
                    }`}
                    onClick={() => setSelectedPartition(part)}
                  >
                    <div className="flex items-center gap-2">
                      <span>&#xF0A0;</span>
                      <span>{part.drive_letter}</span>
                      <span className="text-text-muted">({part.device_path})</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-text-muted">{formatBytes(part.size_bytes)}</span>
                      {part.is_cfs && (
                        <span className={part.is_encrypted ? "text-success" : "text-text-muted"}>
                          {part.is_encrypted ? "CFSE" : "CFS1"}
                        </span>
                      )}
                      {!part.is_cfs && (
                        <span className="text-text-muted text-xs">RAW</span>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            )}

            {selectedPartition && (
              <form onSubmit={handlePartitionUnlock} className="space-y-3">
                <div className="flex items-center gap-2 text-sm">
                  <span className="text-text-muted">Selected:</span>
                  <span className="text-text-bright">{selectedPartition.drive_letter}</span>
                  <span className="text-text-muted">│</span>
                  <span className="text-text-muted">{formatBytes(selectedPartition.size_bytes)}</span>
                  {selectedPartition.is_encrypted && (
                    <>
                      <span className="text-text-muted">│</span>
                      <span className="text-xs text-success">AES-256-XTS</span>
                    </>
                  )}
                </div>

                {selectedPartition.is_encrypted && (
                  <div>
                    <label className="block text-sm text-text-muted mb-1">Password</label>
                    <input
                      type="password"
                      value={partPassword}
                      onChange={(e) => setPartPassword(e.target.value)}
                      className="w-full px-2 py-1.5 bg-surface border border-border text-text text-sm focus:border-border-focus"
                      placeholder="Enter password..."
                    />
                  </div>
                )}

                {partError && (
                  <div className="text-sm text-error">{partError}</div>
                )}

                <button
                  type="submit"
                  disabled={
                    !selectedPartition ||
                    (selectedPartition.is_encrypted && !partPassword) ||
                    loading
                  }
                  className="w-full py-2 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {loading ? "Unlocking..." : "Unlock Partition"}
                </button>
              </form>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
