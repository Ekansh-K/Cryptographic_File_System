import { useRef, useCallback, useState, useEffect } from "react";
import { useAppStore } from "../store";
import FileRow from "./FileRow";
import type { DirEntryDto } from "../types";

export default function FileList() {
  const entries = useAppStore((s) => s.entries);
  const selectedEntry = useAppStore((s) => s.selectedEntry);
  const sortField = useAppStore((s) => s.sortField);
  const sortDir = useAppStore((s) => s.sortDir);
  const loading = useAppStore((s) => s.loading);
  const currentPath = useAppStore((s) => s.currentPath);
  const sort = useAppStore((s) => s.sort);
  const selectEntry = useAppStore((s) => s.selectEntry);
  const navigate = useAppStore((s) => s.navigate);
  const goUp = useAppStore((s) => s.goUp);

  const containerRef = useRef<HTMLDivElement>(null);
  const hasDotDot = currentPath !== "/";
  // true when keyboard focus is on the ".." row
  const [dotdotFocused, setDotdotFocused] = useState(false);

  // Reset dotdot focus whenever entries/path changes
  useEffect(() => {
    setDotdotFocused(false);
  }, [currentPath, entries]);

  // Listen for cross-panel "focus-entry" event dispatched by Sidebar
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    function onFocusEntry(e: Event) {
      const name = (e as CustomEvent<{ name: string }>).detail.name;
      const entry = entries.find((en) => en.name === name);
      if (entry) {
        setDotdotFocused(false);
        selectEntry(entry);
      } else if (hasDotDot) {
        setDotdotFocused(true);
        useAppStore.setState({ selectedEntry: null });
      }
    }
    el.addEventListener("focus-entry", onFocusEntry);
    return () => el.removeEventListener("focus-entry", onFocusEntry);
  }, [entries, hasDotDot, selectEntry]);

  // Returns the index of the selected entry in `entries` (-1 if none found)
  const getSelectedIndex = useCallback((): number => {
    if (!selectedEntry) return -1;
    return entries.findIndex((e) => e.name === selectedEntry.name);
  }, [selectedEntry, entries]);

  function handleOpen(entry: DirEntryDto) {
    if (entry.file_type === "directory") {
      const target =
        currentPath === "/" ? `/${entry.name}` : `${currentPath}/${entry.name}`;
      navigate(target);
    } else {
      selectEntry(entry);
    }
  }

  const sortIndicator = (field: string) => {
    if (field !== sortField) return "";
    return sortDir === "asc" ? " \u25B4" : " \u25BE";
  };

  function focusSidebar(entryName?: string) {
    const sidebar = document.querySelector("[data-sidebar]") as HTMLElement;
    if (entryName) {
      sidebar?.dispatchEvent(
        new CustomEvent("focus-node", { detail: { name: entryName, currentPath } })
      );
    }
    sidebar?.focus();
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    e.stopPropagation();
    const maxIdx = entries.length - 1;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setDotdotFocused(false);
      const cur = getSelectedIndex();
      if (cur === -1 && entries.length > 0) {
        selectEntry(entries[0]);
      } else if (cur < maxIdx) {
        selectEntry(entries[cur + 1]);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const cur = getSelectedIndex();
      if (cur > 0) {
        setDotdotFocused(false);
        selectEntry(entries[cur - 1]);
      } else if (cur === 0 && hasDotDot) {
        setDotdotFocused(true);
        useAppStore.setState({ selectedEntry: null });
      } else if (cur === -1 && dotdotFocused) {
        // already at .., go to sidebar
        focusSidebar();
      }
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      // Move focus to sidebar, targeting the selected entry's node or current dir
      const name = selectedEntry?.name;
      focusSidebar(name);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (dotdotFocused) {
        goUp();
      } else {
        const cur = getSelectedIndex();
        if (cur >= 0 && entries[cur]) {
          handleOpen(entries[cur]);
        }
      }
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* Column headers */}
      <div className="flex items-center h-7 px-2 text-xs text-text-muted bg-surface border-b border-border shrink-0 select-none">
        <span className="w-4 shrink-0" />
        <span className="w-5 shrink-0" />
        <button
          className="flex-1 text-left px-1 hover:text-text"
          onClick={() => sort("name")}
        >
          NAME{sortIndicator("name")}
        </button>
        <button
          className="w-20 text-right hover:text-text shrink-0 pr-3"
          onClick={() => sort("size")}
        >
          SIZE{sortIndicator("size")}
        </button>
        <button
          className="w-28 text-right hover:text-text shrink-0 pr-3"
          onClick={() => sort("modified")}
        >
          DATE{sortIndicator("modified")}
        </button>
      </div>

      {/* Scrollable file list */}
      <div
        ref={containerRef}
        data-filelist
        className="flex-1 overflow-y-auto min-h-0 outline-none"
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        {/* Parent directory entry — single click or Enter to go up */}
        {hasDotDot && (
          <div
            className={`flex items-center h-7 px-2 text-sm cursor-pointer select-none ${
              dotdotFocused
                ? "bg-surface-active text-text-bright"
                : "text-text-muted hover:bg-surface-hover"
            }`}
            onClick={() => goUp()}
          >
            <span className="w-4 shrink-0">{dotdotFocused ? "\u25B8" : ""}</span>
            <span className="w-5 shrink-0 text-center">..</span>
            <span className="flex-1 px-1" />
          </div>
        )}

        {/* Loading skeleton */}
        {loading && entries.length === 0 && (
          <>
            {[1, 2, 3].map((i) => (
              <div key={i} className="flex items-center h-7 px-2">
                <div className="w-full h-3 bg-surface-hover animate-pulse" />
              </div>
            ))}
          </>
        )}

        {/* Empty state */}
        {!loading && entries.length === 0 && (
          <div className="flex items-center justify-center h-24 text-sm text-text-muted">
            Empty directory
          </div>
        )}

        {/* Entries */}
        {entries.map((entry) => (
          <FileRow
            key={entry.name}
            entry={entry}
            isSelected={selectedEntry?.name === entry.name}
            onSelect={selectEntry}
            onOpen={handleOpen}
          />
        ))}
      </div>
    </div>
  );
}
