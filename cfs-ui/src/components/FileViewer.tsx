import { useState } from "react";
import { useAppStore } from "../store";
import TextViewer from "./TextViewer";
import HexViewer from "./HexViewer";
import MetadataPanel from "./MetadataPanel";

type ViewMode = "text" | "hex";

export default function FileViewer() {
  const preview = useAppStore((s) => s.preview);
  const selectedEntry = useAppStore((s) => s.selectedEntry);
  const [mode, setMode] = useState<ViewMode>("text");

  if (!selectedEntry || selectedEntry.file_type !== "file") {
    return (
      <div className="flex flex-col h-full">
        <div className="h-7 flex items-center px-2 text-xs text-text-muted bg-surface border-b border-border shrink-0 select-none">
          PREVIEW
        </div>
        <div className="flex-1 flex items-center justify-center text-sm text-text-muted">
          Select a file to preview
        </div>
      </div>
    );
  }

  if (!preview) {
    return (
      <div className="flex flex-col h-full">
        <div className="h-7 flex items-center px-2 text-xs text-text-muted bg-surface border-b border-border shrink-0 select-none">
          PREVIEW
        </div>
        <div className="flex-1 flex items-center justify-center text-sm text-text-muted">
          Loading...
        </div>
      </div>
    );
  }

  // Decode base64 to bytes
  const raw = atob(preview.data_base64);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) {
    bytes[i] = raw.charCodeAt(i);
  }

  // Default to hex if not text
  const effectiveMode = preview.is_text ? mode : "hex";

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* Header with tabs */}
      <div className="h-7 flex items-center justify-between px-2 bg-surface border-b border-border shrink-0 select-none">
        <span className="text-xs text-text-muted truncate min-w-0">
          {selectedEntry.name}
        </span>
        <div className="flex items-center gap-0 shrink-0">
          <button
            className={`px-2 py-0.5 text-xs ${
              effectiveMode === "text"
                ? "text-text-bright bg-surface-active"
                : "text-text-muted hover:text-text"
            }`}
            onClick={() => setMode("text")}
          >
            TXT
          </button>
          <button
            className={`px-2 py-0.5 text-xs ${
              effectiveMode === "hex"
                ? "text-text-bright bg-surface-active"
                : "text-text-muted hover:text-text"
            }`}
            onClick={() => setMode("hex")}
          >
            HEX
          </button>
        </div>
      </div>

      {/* Truncation warning */}
      {preview.truncated && (
        <div className="px-2 py-1 text-xs text-text-muted bg-surface border-b border-border">
          Showing first {formatPreviewSize(bytes.length)} of{" "}
          {formatPreviewSize(preview.total_size)}
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-auto min-h-0">
        {effectiveMode === "text" ? (
          <TextViewer data={bytes} />
        ) : (
          <HexViewer data={bytes} />
        )}
      </div>

      {/* Metadata */}
      <MetadataPanel />
    </div>
  );
}

function formatPreviewSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}
