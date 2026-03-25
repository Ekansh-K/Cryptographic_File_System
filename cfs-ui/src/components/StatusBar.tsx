import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAppStore } from "../store";
import { listFreeDriveLetters } from "../commands";
import LockConfirmDialog, { shouldSkipLockConfirm } from "./LockConfirmDialog";

export default function StatusBar() {
  const volumeInfo = useAppStore((s) => s.volumeInfo);
  const isEncrypted = useAppStore((s) => s.isEncrypted);
  const isMounted = useAppStore((s) => s.isMounted);
  const driveLetter = useAppStore((s) => s.driveLetter);
  const winfspAvailable = useAppStore((s) => s.winfspAvailable);
  const loading = useAppStore((s) => s.loading);
  const lock = useAppStore((s) => s.lock);
  const mount = useAppStore((s) => s.mount);
  const unmount = useAppStore((s) => s.unmount);
  const checkWinfsp = useAppStore((s) => s.checkWinfsp);
  const navigate = useNavigate();

  const [showLockConfirm, setShowLockConfirm] = useState(false);

  function handleLockClick() {
    if (shouldSkipLockConfirm()) {
      void doLock();
    } else {
      setShowLockConfirm(true);
    }
  }

  async function doLock() {
    setShowLockConfirm(false);
    await lock();
    navigate("/");
  }

  const [showMountPicker, setShowMountPicker] = useState(false);
  const [freeDriveLetters, setFreeDriveLetters] = useState<string[]>([]);
  const [selectedLetter, setSelectedLetter] = useState<string>("");
  const [mountLoading, setMountLoading] = useState(false);

  useEffect(() => {
    checkWinfsp();
  }, [checkWinfsp]);

  async function handleMountClick() {
    if (!winfspAvailable) return;
    try {
      const letters = await listFreeDriveLetters();
      setFreeDriveLetters(letters);
      // Default to last letter (near Z)
      setSelectedLetter(letters.length > 0 ? letters[letters.length - 1] : "");
      setShowMountPicker(true);
    } catch {
      setFreeDriveLetters([]);
      setShowMountPicker(true);
    }
  }

  async function handleMountConfirm() {
    if (!selectedLetter) return;
    setMountLoading(true);
    try {
      await mount(selectedLetter);
      setShowMountPicker(false);
    } catch {
      // error is handled by store
    } finally {
      setMountLoading(false);
    }
  }

  if (!volumeInfo) {
    return (
      <div className="h-statusbar flex items-center px-3 bg-surface border-t border-border text-sm text-text-muted">
        No volume loaded
      </div>
    );
  }

  const volumeName = volumeInfo.path.split(/[/\\]/).pop() ?? "";
  const badge = isEncrypted ? "CFSE" : "CFS1";
  const freeBlocks = `${volumeInfo.free_blocks}/${volumeInfo.total_blocks} blocks free`;

  return (
    <div className="h-statusbar flex items-center justify-between px-3 bg-surface border-t border-border text-sm">
      <div className="flex items-center gap-3">
        <span className="text-text">{volumeName}</span>
        <span className="text-text-muted">│</span>
        <span className="text-text-muted">{badge}</span>
        <span className="text-text-muted">│</span>
        <span className="text-text-muted">{freeBlocks}</span>
        {isMounted && driveLetter && (
          <>
            <span className="text-text-muted">│</span>
            <span className="text-text-muted">
              Mounted: {driveLetter}
            </span>
            <span className="inline-block w-1.5 h-1.5 bg-success" />
          </>
        )}
      </div>
      <div className="flex items-center gap-2 relative">
        {!isMounted ? (
          <>
            <button
              className="px-2 py-0.5 text-sm text-text-muted border border-border hover:border-border-focus hover:text-text disabled:opacity-30 disabled:cursor-not-allowed"
              onClick={handleMountClick}
              disabled={loading || !winfspAvailable}
              title={
                !winfspAvailable
                  ? "WinFSP not installed — required to mount as a Windows drive. Download from winfsp.dev/rel"
                  : "Mount as Windows drive"
              }
              aria-label="Mount drive"
            >
              &#xF0A0; Mount
            </button>
            {!winfspAvailable && (
              <span className="text-xs text-error" title="Download WinFSP from winfsp.dev/rel">
                WinFSP required
              </span>
            )}
          </>
        ) : (
          <button
            className="px-2 py-0.5 text-sm text-text-muted border border-border hover:border-border-focus hover:text-text"
            onClick={() => unmount()}
            disabled={loading}
            aria-label="Unmount drive"
          >
            &#xF0A0; Unmount
          </button>
        )}
        <button
          className="px-2 py-0.5 text-sm text-text-muted border border-border hover:border-border-focus hover:text-text"
          onClick={handleLockClick}
          disabled={loading}
          aria-label="Lock volume"
        >
          &#xF023; Lock
        </button>

        {/* Lock confirmation dialog */}
        {showLockConfirm && (
          <LockConfirmDialog
            volumeName={volumeName}
            isMounted={isMounted}
            onConfirm={doLock}
            onCancel={() => setShowLockConfirm(false)}
          />
        )}

        {/* Drive letter picker popup */}
        {showMountPicker && (
          <div className="absolute bottom-full right-0 mb-2 border border-border bg-surface p-3 min-w-[220px] z-50 shadow-lg">
            <div className="text-sm text-text-bright mb-2">Select Drive Letter</div>
            {freeDriveLetters.length === 0 ? (
              <div className="text-xs text-error mb-2">No free drive letters available</div>
            ) : (
              <select
                className="w-full px-2 py-1 bg-surface border border-border text-text text-sm focus:border-border-focus mb-2"
                value={selectedLetter}
                onChange={(e) => setSelectedLetter(e.target.value)}
              >
                {freeDriveLetters.map((l) => (
                  <option key={l} value={l}>
                    {l}
                  </option>
                ))}
              </select>
            )}
            <div className="flex gap-2 justify-end">
              <button
                className="px-2 py-0.5 text-xs text-text-muted border border-border hover:border-border-focus hover:text-text"
                onClick={() => setShowMountPicker(false)}
              >
                Cancel
              </button>
              <button
                className="px-2 py-0.5 text-xs text-text-bright border border-border bg-surface-active hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
                onClick={handleMountConfirm}
                disabled={!selectedLetter || mountLoading}
              >
                {mountLoading ? "Mounting..." : `Mount ${selectedLetter}`}
              </button>
            </div>
            <div className="text-xs text-text-muted mt-2">
              The CFS volume will appear in Windows Explorer at this drive letter.
              {!winfspAvailable && (
                <span className="block text-error mt-1">
                  WinFSP is required for mounting. Download from winfsp.dev/rel
                </span>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
