import type { DirEntryDto } from "../types";

// Nerd Font glyphs for file types
const ICON_DIR_CLOSED = "\uF413";
const ICON_FILE = "\uF4A5";
const ICON_TEXT = "\uE612";
const ICON_MARKDOWN = "\uE73C";
const ICON_RUST = "\uE795";
const ICON_TS = "\uE781";
const ICON_PDF = "\uF1C1";
const ICON_IMAGE = "\uF1C5";

function getFileIcon(entry: DirEntryDto): string {
  if (entry.file_type === "directory") return ICON_DIR_CLOSED;
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  switch (ext) {
    case "txt":
    case "log":
    case "cfg":
    case "ini":
    case "csv":
      return ICON_TEXT;
    case "md":
      return ICON_MARKDOWN;
    case "rs":
      return ICON_RUST;
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
      return ICON_TS;
    case "pdf":
      return ICON_PDF;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "bmp":
    case "webp":
      return ICON_IMAGE;
    default:
      return ICON_FILE;
  }
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${bytes}B`;
}

function formatDate(timestamp: number): string {
  if (!timestamp) return "";
  const d = new Date(timestamp * 1000);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const min = String(d.getMinutes()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${min}`;
}

interface Props {
  entry: DirEntryDto;
  isSelected: boolean;
  onSelect: (entry: DirEntryDto) => void;
  onOpen: (entry: DirEntryDto) => void;
}

export default function FileRow({ entry, isSelected, onSelect, onOpen }: Props) {
  return (
    <div
      className={`flex items-center h-7 px-2 text-sm cursor-pointer select-none ${
        isSelected
          ? "bg-surface-active text-text-bright"
          : "text-text hover:bg-surface-hover"
      }`}
      onClick={() => onSelect(entry)}
      onDoubleClick={() => onOpen(entry)}
    >
      {/* Selection indicator */}
      <span className="w-4 shrink-0 text-text-muted">
        {isSelected ? "\u25B8" : ""}
      </span>
      {/* Icon */}
      <span className="w-5 shrink-0 text-center text-text-muted">
        {getFileIcon(entry)}
      </span>
      {/* Name */}
      <span className="flex-1 truncate min-w-0 px-1">
        {entry.name}{entry.file_type === "directory" ? "/" : ""}
      </span>
      {/* Size */}
      <span className="w-20 text-right text-text-muted shrink-0 pr-3">
        {entry.file_type === "file" ? formatSize(entry.size) : ""}
      </span>
      {/* Date */}
      <span className="w-28 text-right text-text-muted shrink-0 pr-3 whitespace-nowrap">
        {formatDate(entry.modified)}
      </span>
    </div>
  );
}
