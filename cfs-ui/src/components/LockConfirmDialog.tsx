import { useState } from "react";

interface Props {
  volumeName: string;
  isMounted: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

const STORAGE_KEY = "cfs.skipLockConfirm";

export function shouldSkipLockConfirm(): boolean {
  return localStorage.getItem(STORAGE_KEY) === "true";
}

export default function LockConfirmDialog({ volumeName, isMounted, onConfirm, onCancel }: Props) {
  const [dontAsk, setDontAsk] = useState(false);

  function handleConfirm() {
    if (dontAsk) {
      localStorage.setItem(STORAGE_KEY, "true");
    }
    onConfirm();
  }

  return (
    <>
      {/* Backdrop — blurs everything behind the dialog */}
      <div
        className="fixed inset-0 z-40 bg-bg/70 backdrop-blur-sm"
        onClick={onCancel}
      />

      {/* Dialog */}
      <div className="fixed inset-0 z-50 flex items-center justify-center px-4">
        <div className="w-full max-w-sm border border-border bg-surface p-5 space-y-4">
          {/* Header */}
          <div className="flex items-start justify-between gap-4">
            <div className="flex items-center gap-2">
              <span className="text-text-bright text-base">&#xF023;</span>
              <span className="text-sm text-text-bright font-medium">Lock Volume</span>
            </div>
            <button
              className="text-text-muted hover:text-text text-sm"
              onClick={onCancel}
              aria-label="Cancel"
            >
              &#x2715;
            </button>
          </div>

          {/* Body */}
          <div className="space-y-2 text-sm text-text">
            <p>
              You are about to lock{" "}
              <span className="text-text-bright">{volumeName}</span>.
            </p>
            <p className="text-text-muted leading-relaxed">
              This will close the current volume and return you to the start
              screen, where you can unlock a different volume or create a new
              one.
              {isMounted && (
                <span className="block mt-1 text-error">
                  &#x26A0; The drive is currently mounted — locking will unmount
                  it first.
                </span>
              )}
            </p>
          </div>

          {/* Don't show again */}
          <label className="flex items-center gap-2 cursor-pointer select-none">
            <div
              className={`w-3.5 h-3.5 border flex items-center justify-center shrink-0 ${
                dontAsk ? "border-border-focus bg-surface-active" : "border-border bg-surface"
              }`}
              onClick={() => setDontAsk((v) => !v)}
            >
              {dontAsk && (
                <span className="text-text-bright text-xs leading-none">&#x2713;</span>
              )}
            </div>
            <span className="text-xs text-text-muted">Don't ask me again</span>
          </label>

          {/* Actions */}
          <div className="flex gap-2 justify-end pt-1">
            <button
              className="px-3 py-1.5 text-sm text-text-muted border border-border hover:border-border-focus hover:text-text"
              onClick={onCancel}
            >
              Cancel
            </button>
            <button
              className="px-3 py-1.5 text-sm text-text-bright border border-border bg-surface-active hover:border-border-focus"
              onClick={handleConfirm}
            >
              &#xF023; Lock &amp; Exit
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
