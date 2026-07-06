import { useState, useEffect } from "react";
import { listKeySlots, addKeySlot, removeKeySlot } from "../commands";
import type { KeySlotInfo } from "../types";

export default function KeySlotManagerModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [slots, setSlots] = useState<KeySlotInfo[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  
  // Master password state
  const [masterPassword, setMasterPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [actionLoading, setActionLoading] = useState(false);

  useEffect(() => {
    if (!open) {
      setMasterPassword("");
      setNewPassword("");
      setSlots(null);
      setError(null);
      setSuccessMsg(null);
    }
  }, [open]);

  async function handleLoad(e: React.FormEvent) {
    e.preventDefault();
    if (!masterPassword) return;
    setLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const data = await listKeySlots(masterPassword);
      setSlots(data);
    } catch (e) {
      setError(String(e));
      setSlots(null);
    } finally {
      setLoading(false);
    }
  }

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault();
    if (!masterPassword || !newPassword) return;
    setActionLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      await addKeySlot(masterPassword, newPassword);
      setSuccessMsg("Key slot added successfully.");
      setNewPassword("");
      // Refresh slots
      const data = await listKeySlots(masterPassword);
      setSlots(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionLoading(false);
    }
  }

  async function handleRemove(index: number) {
    if (!confirm(`Are you sure you want to remove slot ${index}?`)) return;
    setActionLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      await removeKeySlot(masterPassword, index);
      setSuccessMsg(`Slot ${index} removed successfully.`);
      // Refresh slots
      const data = await listKeySlots(masterPassword);
      setSlots(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setActionLoading(false);
    }
  }

  if (!open) return null;

  return (
    <div className="fixed inset-0 bg-bg/80 backdrop-blur-sm z-50 flex items-center justify-center p-4">
      <div className="bg-bg border border-border shadow-2xl w-full max-w-2xl max-h-[90vh] flex flex-col font-mono text-sm">
        <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-surface shrink-0">
          <h2 className="text-text-bright font-bold">Manage Key Slots</h2>
          <button onClick={onClose} className="text-text-muted hover:text-text">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-6">
          {slots === null ? (
            <div className="space-y-3">
              <p className="text-text-muted text-xs">Enter your current master password to view and manage key slots.</p>
              <form onSubmit={handleLoad} className="flex gap-2">
                <input
                  type="password"
                  value={masterPassword}
                  onChange={(e) => setMasterPassword(e.target.value)}
                  className="flex-1 px-2 py-1.5 bg-surface border border-border text-text text-xs focus:border-border-focus"
                  placeholder="Master password..."
                  autoFocus
                />
                <button
                  type="submit"
                  disabled={loading || !masterPassword}
                  className="px-4 py-1.5 text-xs border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40"
                >
                  {loading ? "Authorizing..." : "Load Slots"}
                </button>
              </form>
            </div>
          ) : (
            <>
              <div className="space-y-2">
                <h3 className="text-text font-bold">Current Slots</h3>
                {slots.length === 0 ? (
                  <div className="text-text-muted text-xs">No active key slots found.</div>
                ) : (
                  <div className="border border-border">
                    {slots.map((s) => (
                      <div key={s.index} className="flex items-center justify-between px-3 py-2 border-b border-border last:border-b-0 bg-surface">
                        <div>
                          <div className="text-text-bright text-xs">Slot {s.index} {s.is_active ? <span className="text-success ml-2">Active</span> : <span className="text-text-muted ml-2">Empty</span>}</div>
                          {s.is_active && (
                            <div className="text-text-muted text-xs mt-1">
                              {s.kdf_algorithm.toUpperCase()} 
                              {s.kdf_algorithm === "argon2id" && ` (${s.argon2_memory_mib}MiB, t=${s.argon2_time_cost}, p=${s.argon2_parallelism})`}
                              {(s.kdf_algorithm === "pbkdf2" || s.kdf_algorithm === "pbkdf2-sha512") && ` (${s.pbkdf2_iterations} iters)`}
                            </div>
                          )}
                        </div>
                        {s.is_active && (
                          <button
                            onClick={() => handleRemove(s.index)}
                            disabled={actionLoading}
                            className="px-2 py-1 text-xs border border-error text-error hover:bg-error hover:text-white transition-colors disabled:opacity-40"
                          >
                            Remove
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>

              <div className="space-y-3 pt-4 border-t border-border">
                <h3 className="text-text font-bold">Add New Key Slot</h3>
                <form onSubmit={handleAdd} className="flex gap-2">
                  <input
                    type="password"
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    className="flex-1 px-2 py-1.5 bg-surface border border-border text-text text-xs focus:border-border-focus"
                    placeholder="Enter new password..."
                  />
                  <button
                    type="submit"
                    disabled={actionLoading || !newPassword}
                    className="px-4 py-1.5 text-xs border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 transition-colors"
                  >
                    {actionLoading ? "Adding..." : "Add Slot"}
                  </button>
                </form>
              </div>
            </>
          )}

          {error && <div className="text-error text-xs p-2 border border-error bg-error/10">{error}</div>}
          {successMsg && <div className="text-success text-xs p-2 border border-success bg-success/10">{successMsg}</div>}
        </div>
      </div>
    </div>
  );
}
