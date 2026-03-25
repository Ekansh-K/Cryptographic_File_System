import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store";

const appWindow = getCurrentWindow();

export default function Titlebar() {
  const volumeInfo = useAppStore((s) => s.volumeInfo);

  const volumeName = volumeInfo
    ? volumeInfo.path.split(/[/\\]/).pop() ?? ""
    : "";

  const title = volumeInfo ? `CFS — ${volumeName}` : "CFS";

  return (
    <div
      className="h-titlebar flex items-center justify-between bg-surface border-b border-border select-none"
    >
      {/* Drag region is only on the title area, not the whole bar */}
      <div
        className="flex-1 flex items-center gap-2 px-3 text-sm text-text-muted h-full"
        data-tauri-drag-region
      >
        <span className="text-text-bright pointer-events-none">&#x25AA;</span>
        <span className="pointer-events-none">{title}</span>
      </div>
      <div className="flex items-center h-full">
        <button
          className="h-full px-3 text-text-muted hover:bg-surface-hover hover:text-text transition-none"
          onClick={() => appWindow.minimize()}
          aria-label="Minimize"
        >
          &#x2500;
        </button>
        <button
          className="h-full px-3 text-text-muted hover:bg-surface-hover hover:text-text transition-none"
          onClick={() => appWindow.toggleMaximize()}
          aria-label="Maximize"
        >
          &#x25A1;
        </button>
        <button
          className="h-full px-3 text-text-muted hover:bg-error hover:text-text-bright transition-none"
          onClick={() => appWindow.close()}
          aria-label="Close"
        >
          &#x2715;
        </button>
      </div>
    </div>
  );
}
