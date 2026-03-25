import { useState, useCallback, useEffect, useRef } from "react";
import { listDir } from "../commands";
import { useAppStore } from "../store";
import type { DirEntryDto } from "../types";

interface TreeNodeData {
  name: string;
  path: string;
  isFile: boolean;
  children: TreeNodeData[] | null; // null = not yet loaded (dirs only); files always null
  expanded: boolean;
  loading: boolean;
}

function TreeNodeItem({
  node,
  onToggle,
  onNavigate,
  activePath,
  focusedPath,
}: {
  node: TreeNodeData;
  onToggle: (path: string) => void;
  onNavigate: (path: string) => void;
  activePath: string;
  focusedPath: string | null;
}) {
  const isActive = activePath === node.path;
  const isFocused = focusedPath === node.path;
  const depth = node.path.split("/").filter(Boolean).length;
  const indent = depth * 12;

  return (
    <>
      <div
        className={[
          "flex items-center h-7 pr-2 text-sm cursor-pointer select-none",
          isActive
            ? "bg-surface-active text-text-bright"
            : isFocused
            ? "bg-surface-hover text-text"
            : "text-text hover:bg-surface-hover",
        ].join(" ")}
        style={{ paddingLeft: `${indent + 4}px` }}
        onClick={() => onNavigate(node.path)}
      >
        {/* Expand arrow — only for directories */}
        <span className="w-4 shrink-0 text-center text-text-muted">
          {!node.isFile && (
            <button
              className="w-full hover:text-text"
              tabIndex={-1}
              onClick={(e) => {
                e.stopPropagation();
                onToggle(node.path);
              }}
            >
              {node.loading ? "\u00B7" : node.expanded ? "\u25BE" : "\u25B8"}
            </button>
          )}
        </span>
        <span className="truncate min-w-0">
          {node.path === "/" ? "/" : node.isFile ? node.name : `${node.name}/`}
        </span>
      </div>
      {!node.isFile && node.expanded &&
        node.children?.map((child) => (
          <TreeNodeItem
            key={child.path}
            node={child}
            onToggle={onToggle}
            onNavigate={onNavigate}
            activePath={activePath}
            focusedPath={focusedPath}
          />
        ))}
    </>
  );
}

/** Flatten visible tree nodes into an ordered list for keyboard nav */
function flattenVisible(node: TreeNodeData): TreeNodeData[] {
  const result: TreeNodeData[] = [node];
  if (!node.isFile && node.expanded && node.children) {
    for (const child of node.children) {
      result.push(...flattenVisible(child));
    }
  }
  return result;
}

export default function Sidebar() {
  const currentPath = useAppStore((s) => s.currentPath);
  const navigate = useAppStore((s) => s.navigate);
  const containerRef = useRef<HTMLDivElement>(null);
  const [focusedPath, setFocusedPath] = useState<string | null>(null);

  const [tree, setTree] = useState<TreeNodeData>({
    name: "/",
    path: "/",
    isFile: false,
    children: null,
    expanded: true,
    loading: false,
  });

  // Load root children on mount
  useEffect(() => {
    loadChildren("/");
  }, []);

  const loadChildren = useCallback(async (path: string) => {
    setTree((prev) => updateNode(prev, path, { loading: true }));
    try {
      const entries = await listDir(path);
      const dirs: TreeNodeData[] = entries
        .filter((e: DirEntryDto) => e.file_type === "directory")
        .sort((a: DirEntryDto, b: DirEntryDto) => a.name.localeCompare(b.name))
        .map((e: DirEntryDto) => ({
          name: e.name,
          path: path === "/" ? `/${e.name}` : `${path}/${e.name}`,
          isFile: false,
          children: null,
          expanded: false,
          loading: false,
        }));
      const files: TreeNodeData[] = entries
        .filter((e: DirEntryDto) => e.file_type === "file")
        .sort((a: DirEntryDto, b: DirEntryDto) => a.name.localeCompare(b.name))
        .map((e: DirEntryDto) => ({
          name: e.name,
          path: path === "/" ? `/${e.name}` : `${path}/${e.name}`,
          isFile: true,
          children: null,
          expanded: false,
          loading: false,
        }));
      setTree((prev) =>
        updateNode(prev, path, {
          children: [...dirs, ...files],
          expanded: true,
          loading: false,
        })
      );
    } catch {
      setTree((prev) => updateNode(prev, path, { loading: false }));
    }
  }, []);

  // Listen for cross-panel "focus-node" event dispatched by FileList
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    function onFocusNode(e: Event) {
      const { name, currentPath: cp } = (e as CustomEvent<{ name: string; currentPath: string }>).detail;
      if (!name) {
        setFocusedPath(cp || "/");
        return;
      }
      const nodePath = cp === "/" ? `/${name}` : `${cp}/${name}`;
      const node = findNode(tree, nodePath);
      setFocusedPath(node ? nodePath : cp || "/");
    }
    el.addEventListener("focus-node", onFocusNode);
    return () => el.removeEventListener("focus-node", onFocusNode);
  }, [tree]);

  function handleToggle(path: string) {
    setTree((prev) => {
      const node = findNode(prev, path);
      if (!node || node.isFile) return prev;
      if (node.expanded) {
        return updateNode(prev, path, { expanded: false });
      }
      return prev;
    });
    const node = findNode(tree, path);
    if (node && !node.isFile && !node.expanded) {
      if (node.children === null) {
        loadChildren(path);
      } else {
        setTree((prev) => updateNode(prev, path, { expanded: true }));
      }
    }
  }

  function handleNavigate(path: string) {
    setFocusedPath(path);
    const node = findNode(tree, path);
    if (node?.isFile) return; // files are leaf nodes — no dir navigation
    navigate(path);
    if (node && !node.expanded) {
      if (node.children === null) {
        loadChildren(path);
      } else {
        setTree((prev) => updateNode(prev, path, { expanded: true }));
      }
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    const flat = flattenVisible(tree);
    const idx = focusedPath
      ? flat.findIndex((n) => n.path === focusedPath)
      : -1;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = flat[idx + 1];
      if (next) setFocusedPath(next.path);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = flat[idx - 1];
      if (prev) setFocusedPath(prev.path);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      const node = focusedPath ? findNode(tree, focusedPath) : null;
      if (!node) return;
      if (!node.isFile && !node.expanded) {
        // Expand the directory
        handleToggle(node.path);
        navigate(node.path);
      } else {
        // Already expanded or it's a file — move focus to FileList, pre-select this node
        const fileList = document.querySelector("[data-filelist]") as HTMLElement;
        if (fileList) {
          fileList.dispatchEvent(
            new CustomEvent("focus-entry", { detail: { name: node.name } })
          );
          fileList.focus();
        }
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      const node = focusedPath ? findNode(tree, focusedPath) : null;
      if (!node) return;
      if (node.isFile) {
        // Navigate to the file's parent dir so it appears in the main list
        const parts = node.path.split("/").filter(Boolean);
        parts.pop();
        navigate(parts.length > 0 ? "/" + parts.join("/") : "/");
        return;
      }
      // Toggle expand/collapse for directories
      if (node.expanded) {
        setTree((prev) => updateNode(prev, node.path, { expanded: false }));
      } else {
        handleToggle(node.path);
        navigate(node.path);
      }
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      const node = focusedPath ? findNode(tree, focusedPath) : null;
      if (!node) return;
      if (!node.isFile && node.expanded) {
        setTree((prev) => updateNode(prev, node.path, { expanded: false }));
      } else {
        const parts = node.path.split("/").filter(Boolean);
        parts.pop();
        setFocusedPath("/" + parts.join("/") || "/");
      }
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="h-7 flex items-center px-2 text-xs text-text-muted bg-surface border-b border-border shrink-0 select-none">
        TREE
      </div>
      <div
        ref={containerRef}
        data-sidebar
        className="flex-1 overflow-y-auto min-h-0 outline-none"
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        <TreeNodeItem
          node={tree}
          onToggle={handleToggle}
          onNavigate={handleNavigate}
          activePath={currentPath}
          focusedPath={focusedPath}
        />
      </div>
    </div>
  );
}

// Tree manipulation helpers
function findNode(node: TreeNodeData, path: string): TreeNodeData | null {
  if (node.path === path) return node;
  if (node.children) {
    for (const child of node.children) {
      const found = findNode(child, path);
      if (found) return found;
    }
  }
  return null;
}

function updateNode(
  node: TreeNodeData,
  path: string,
  updates: Partial<TreeNodeData>
): TreeNodeData {
  if (node.path === path) return { ...node, ...updates };
  if (node.children) {
    return {
      ...node,
      children: node.children.map((child) => updateNode(child, path, updates)),
    };
  }
  return node;
}
