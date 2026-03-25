import { create } from "zustand";
import type { VolumeInfo, DirEntryDto, FilePreview, FormatOptionsDto } from "./types";
import * as cmd from "./commands";

type SortField = "name" | "size" | "modified";
type SortDir = "asc" | "desc";

interface AppStore {
  // Volume state
  volumeInfo: VolumeInfo | null;
  isEncrypted: boolean;
  isMounted: boolean;
  driveLetter: string | null;
  winfspAvailable: boolean;
  defaultVolumesDir: string | null;

  // Browse state
  currentPath: string;
  entries: DirEntryDto[];
  selectedEntry: DirEntryDto | null;
  sortField: SortField;
  sortDir: SortDir;

  // Preview state
  preview: FilePreview | null;

  // UI state
  loading: boolean;
  error: string | null;

  // Actions
  unlock: (path: string, password: string) => Promise<void>;
  create: (path: string, size: string, password: string, kdf?: string, pbkdf2Iterations?: number, argon2MemoryMib?: number, argon2Time?: number, argon2Parallelism?: number, formatOptions?: FormatOptionsDto) => Promise<void>;
  lock: () => Promise<void>;
  navigate: (path: string) => Promise<void>;
  goUp: () => Promise<void>;
  refresh: () => Promise<void>;
  selectEntry: (entry: DirEntryDto) => Promise<void>;
  sort: (field: SortField) => void;
  mount: (driveLetter?: string) => Promise<void>;
  unmount: () => Promise<void>;
  checkWinfsp: () => Promise<void>;
  loadDefaultDir: () => Promise<void>;
  clearError: () => void;
}

function sortEntries(
  entries: DirEntryDto[],
  field: SortField,
  dir: SortDir
): DirEntryDto[] {
  const sorted = [...entries];
  sorted.sort((a, b) => {
    // Directories always first
    if (a.file_type !== b.file_type) {
      return a.file_type === "directory" ? -1 : 1;
    }
    let cmp = 0;
    switch (field) {
      case "name":
        cmp = a.name.localeCompare(b.name);
        break;
      case "size":
        cmp = a.size - b.size;
        break;
      case "modified":
        cmp = a.modified - b.modified;
        break;
    }
    return dir === "asc" ? cmp : -cmp;
  });
  return sorted;
}

function parentPath(path: string): string {
  if (path === "/") return "/";
  const parts = path.replace(/\/$/, "").split("/").filter(Boolean);
  parts.pop();
  return "/" + parts.join("/");
}

export const useAppStore = create<AppStore>((set, get) => ({
  volumeInfo: null,
  isEncrypted: false,
  isMounted: false,
  driveLetter: null,
  winfspAvailable: false,
  defaultVolumesDir: null,
  currentPath: "/",
  entries: [],
  selectedEntry: null,
  sortField: "name",
  sortDir: "asc",
  preview: null,
  loading: false,
  error: null,

  unlock: async (path, password) => {
    set({ loading: true, error: null });
    try {
      const info = await cmd.unlockVolume(path, password);
      set({
        volumeInfo: info,
        isEncrypted: info.is_encrypted,
        loading: false,
      });
      // Auto-navigate to root after unlock
      await get().navigate("/");
    } catch (e) {
      set({ loading: false, error: String(e) });
      throw e;
    }
  },

  create: async (path, size, password, kdf, pbkdf2Iterations, argon2MemoryMib, argon2Time, argon2Parallelism, formatOptions) => {
    set({ loading: true, error: null });
    try {
      const info = await cmd.createVolume(
        path,
        size,
        password,
        undefined,
        pbkdf2Iterations,
        kdf,
        argon2MemoryMib,
        argon2Time,
        argon2Parallelism,
        formatOptions,
      );
      set({
        volumeInfo: info,
        isEncrypted: info.is_encrypted,
        loading: false,
      });
      await get().navigate("/");
    } catch (e) {
      set({ loading: false, error: String(e) });
      throw e;
    }
  },

  lock: async () => {
    set({ loading: true, error: null });
    try {
      await cmd.lockVolume();
      set({
        volumeInfo: null,
        isEncrypted: false,
        isMounted: false,
        driveLetter: null,
        currentPath: "/",
        entries: [],
        selectedEntry: null,
        preview: null,
        loading: false,
      });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  navigate: async (path) => {
    set({ loading: true, error: null, selectedEntry: null, preview: null });
    try {
      const raw = await cmd.listDir(path);
      const { sortField, sortDir } = get();
      const entries = sortEntries(raw, sortField, sortDir);
      set({ currentPath: path, entries, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  goUp: async () => {
    const parent = parentPath(get().currentPath);
    await get().navigate(parent);
  },

  refresh: async () => {
    await get().navigate(get().currentPath);
  },

  selectEntry: async (entry) => {
    set({ selectedEntry: entry });
    if (entry.file_type === "file") {
      try {
        const fullPath =
          get().currentPath === "/"
            ? `/${entry.name}`
            : `${get().currentPath}/${entry.name}`;
        const preview = await cmd.readFilePreview(fullPath);
        set({ preview });
      } catch (e) {
        set({ error: String(e), preview: null });
      }
    } else {
      set({ preview: null });
    }
  },

  sort: (field) => {
    const { sortField, sortDir, entries } = get();
    const newDir = field === sortField && sortDir === "asc" ? "desc" : "asc";
    set({
      sortField: field,
      sortDir: newDir,
      entries: sortEntries(entries, field, newDir),
    });
  },

  mount: async (driveLetter) => {
    set({ loading: true, error: null });
    try {
      const info = await cmd.mountDrive(driveLetter);
      set({
        isMounted: info.mounted,
        driveLetter: info.drive_letter,
        loading: false,
      });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  unmount: async () => {
    set({ loading: true, error: null });
    try {
      await cmd.unmountDrive();
      set({
        isMounted: false,
        driveLetter: null,
        loading: false,
      });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  checkWinfsp: async () => {
    try {
      const available = await cmd.checkWinfsp();
      set({ winfspAvailable: available });
    } catch {
      set({ winfspAvailable: false });
    }
  },

  loadDefaultDir: async () => {
    try {
      const dir = await cmd.getDefaultVolumesDir();
      set({ defaultVolumesDir: dir });
    } catch {
      // non-critical
    }
  },

  clearError: () => set({ error: null }),
}));
