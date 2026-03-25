import { useState, useEffect } from "react";
import { useAppStore } from "../store";
import { statEntry } from "../commands";
import type { InodeDto } from "../types";

function formatTimestamp(ts: number): string {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  return d.toLocaleString();
}

export default function MetadataPanel() {
  const selectedEntry = useAppStore((s) => s.selectedEntry);
  const currentPath = useAppStore((s) => s.currentPath);
  const [inode, setInode] = useState<InodeDto | null>(null);

  useEffect(() => {
    if (!selectedEntry) {
      setInode(null);
      return;
    }
    const fullPath =
      currentPath === "/"
        ? `/${selectedEntry.name}`
        : `${currentPath}/${selectedEntry.name}`;
    statEntry(fullPath)
      .then(setInode)
      .catch(() => setInode(null));
  }, [selectedEntry, currentPath]);

  if (!inode || !selectedEntry) return null;

  return (
    <div className="border-t border-border bg-surface px-2 py-2 text-xs text-text-muted space-y-1 shrink-0">
      <div className="text-text-bright text-xs mb-1">METADATA</div>
      <div className="flex justify-between">
        <span>Type</span>
        <span className="text-text">{inode.file_type}</span>
      </div>
      <div className="flex justify-between">
        <span>Size</span>
        <span className="text-text">{inode.size.toLocaleString()} B</span>
      </div>
      <div className="flex justify-between">
        <span>Blocks</span>
        <span className="text-text">{inode.block_count}</span>
      </div>
      <div className="flex justify-between">
        <span>Links</span>
        <span className="text-text">{inode.nlinks}</span>
      </div>
      <div className="flex justify-between">
        <span>Created</span>
        <span className="text-text">{formatTimestamp(inode.created)}</span>
      </div>
      <div className="flex justify-between">
        <span>Modified</span>
        <span className="text-text">{formatTimestamp(inode.modified)}</span>
      </div>
      {inode.has_indirect && (
        <div className="flex justify-between">
          <span>Indirect</span>
          <span className="text-text">yes</span>
        </div>
      )}
    </div>
  );
}
