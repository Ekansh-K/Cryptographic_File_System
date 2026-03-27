import { useState } from "react";
import { benchmarkKdf, benchmarkFormatIo, cancelBenchmark } from "../commands";
import type { IoBenchmarkResult, FormatOptionsDto } from "../types";

// ─── Preset definitions (must mirror Rust FormatOptions presets) ────────────
interface PresetValues {
  blockSize: number;
  inodeSize: number;
  inodeRatio: number;
  journalPercent: number;
  secureDelete: boolean;
  errorBehavior: string;
}

const FORMAT_PRESETS: Record<string, PresetValues> = {
  general:      { blockSize: 4096,  inodeSize: 256, inodeRatio: 16384, journalPercent: 1.0, secureDelete: true,  errorBehavior: "continue"  },
  "large-files":{ blockSize: 16384, inodeSize: 256, inodeRatio: 65536, journalPercent: 0.5, secureDelete: true,  errorBehavior: "continue"  },
  "small-files":{ blockSize: 4096,  inodeSize: 256, inodeRatio: 4096,  journalPercent: 2.0, secureDelete: true,  errorBehavior: "continue"  },
  "max-security":{ blockSize: 4096, inodeSize: 256, inodeRatio: 16384, journalPercent: 2.0, secureDelete: true,  errorBehavior: "read-only" },
  minimal:      { blockSize: 4096,  inodeSize: 128, inodeRatio: 16384, journalPercent: 0.0, secureDelete: false, errorBehavior: "continue"  },
};

const PRESET_LABELS: Record<string, string> = {
  general:       "General Purpose",
  "large-files": "Large Files",
  "small-files": "Small Files",
  "max-security":"Max Security",
  minimal:       "Minimal",
};

const PRESET_DESCRIPTIONS: Record<string, string> = {
  general:        "Balanced defaults. Best for general encrypted storage.",
  "large-files":  "Optimised for video, archives, large blobs. Fewer inodes, bigger blocks.",
  "small-files":  "Optimised for source code, configs, many tiny files. More inodes, bigger journal.",
  "max-security": "All integrity features on. Switches to read-only on any error.",
  minimal:        "v2-compatible, smallest metadata overhead. No journal, no secure delete.",
};

interface Props {
  open: boolean;
  onClose: () => void;
  // ── KDF state (controlled by CreateVolumeForm) ──────────────────────────
  kdf: "argon2id" | "pbkdf2";
  setKdf: (v: "argon2id" | "pbkdf2") => void;
  argon2MemoryMib: number;
  setArgon2MemoryMib: (v: number) => void;
  argon2Time: number;
  setArgon2Time: (v: number) => void;
  argon2Parallelism: number;
  setArgon2Parallelism: (v: number) => void;
  pbkdf2Iters: number;
  setPbkdf2Iters: (v: number) => void;
  // ── Volume Format state (controlled by CreateVolumeForm) ─────────────────
  setFormatPreset: (v: string) => void;
  formatBlockSize: number;
  setFormatBlockSize: (v: number) => void;
  formatInodeSize: number;
  setFormatInodeSize: (v: number) => void;
  formatInodeRatio: number;
  setFormatInodeRatio: (v: number) => void;
  formatJournalPercent: number;
  setFormatJournalPercent: (v: number) => void;
  formatVolumeLabel: string;
  setFormatVolumeLabel: (v: string) => void;
  formatSecureDelete: boolean;
  setFormatSecureDelete: (v: boolean) => void;
  formatDefaultPermissions: number;
  setFormatDefaultPermissions: (v: number) => void;
  formatErrorBehavior: string;
  setFormatErrorBehavior: (v: string) => void;
}

const TABS = ["Key Derivation", "Volume Format"] as const;
type Tab = (typeof TABS)[number];

const PBKDF2_PRESETS: { label: string; value: number }[] = [
  { label: "Fast (100k)", value: 100_000 },
  { label: "Balanced (300k) — rec", value: 300_000 },
  { label: "Strong (600k)", value: 600_000 },
  { label: "Maximum (1M)", value: 1_000_000 },
];

const BLOCK_SIZES = [512, 1024, 2048, 4096, 8192, 16384, 32768, 65536];
const JOURNAL_PERCENTS = [0, 0.5, 1.0, 2.0, 5.0];

/** Format octal number as 3-digit string, e.g. 0o755 → "755" */
function toOctalStr(n: number): string {
  return n.toString(8).padStart(3, "0");
}

/** Parse octal string strictly. Returns NaN if invalid. */
function parseOctalStr(s: string): number {
  if (!/^[0-7]{1,4}$/.test(s)) return NaN;
  return parseInt(s, 8);
}

export default function AdvancedSettingsModal({
  open,
  onClose,
  kdf,
  setKdf,
  argon2MemoryMib,
  setArgon2MemoryMib,
  argon2Time,
  setArgon2Time,
  argon2Parallelism,
  setArgon2Parallelism,
  pbkdf2Iters,
  setPbkdf2Iters,
  setFormatPreset,
  formatBlockSize,
  setFormatBlockSize,
  formatInodeSize,
  setFormatInodeSize,
  formatInodeRatio,
  setFormatInodeRatio,
  formatJournalPercent,
  setFormatJournalPercent,
  formatVolumeLabel,
  setFormatVolumeLabel,
  formatSecureDelete,
  setFormatSecureDelete,
  formatDefaultPermissions,
  setFormatDefaultPermissions,
  formatErrorBehavior,
  setFormatErrorBehavior,
}: Props) {
  const [activeTab, setActiveTab] = useState<Tab>("Key Derivation");
  const [benchmarkMs, setBenchmarkMs] = useState<number | null>(null);
  const [benchmarking, setBenchmarking] = useState(false);
  const [benchmarkError, setBenchmarkError] = useState<string | null>(null);
  // Track raw octal text while user types so we don't mangle partial input
  const [permOctalText, setPermOctalText] = useState(() => toOctalStr(formatDefaultPermissions));

  // ── I/O benchmark state ──────────────────────────────────────────────────
  type IoBenchState = "idle" | "running" | "done";
  interface SizeResult {
    label: string;
    bytes: number;
    status: "pending" | "running" | "done" | "error";
    result?: IoBenchmarkResult;
    runCount?: number; // how many runs were averaged
    error?: string;
  }
  const ALL_PRESETS: { key: string; label: string; bytes: number }[] = [
    { key: "small",  label: "Small (4 KiB)",  bytes: 4 * 1024 },
    { key: "medium", label: "Medium (1 MiB)", bytes: 1 * 1024 * 1024 },
    { key: "large",  label: "Large (16 MiB)", bytes: 16 * 1024 * 1024 },
    { key: "xl",     label: "XL (128 MiB)",   bytes: 128 * 1024 * 1024 },
    { key: "xxl",    label: "XXL (512 MiB)",  bytes: 512 * 1024 * 1024 },
  ];
  const GORLOCK = { key: "gorlock", label: "Gorlock (4 GiB)", bytes: 4 * 1024 * 1024 * 1024 };

  const [ioBenchState, setIoBenchState] = useState<IoBenchState>("idle");
  const [ioBenchResults, setIoBenchResults] = useState<SizeResult[]>([]);
  // Per-preset toggle — all on by default
  const [enabledKeys, setEnabledKeys] = useState<Set<string>>(
    () => new Set(["small", "medium", "large", "xl", "xxl"])
  );
  const [enableGorlock, setEnableGorlock] = useState(false);
  // Averaging
  const [useAvg, setUseAvg] = useState(true);
  const [avgRuns, setAvgRuns] = useState(5);

  function fmtSpeed(mbps: number): string {
    if (mbps >= 1024) return `${(mbps / 1024).toFixed(2)} GiB/s`;
    if (mbps >= 1) return `${mbps.toFixed(2)} MiB/s`;
    return `${(mbps * 1024).toFixed(1)} KiB/s`;
  }
  function fmtTime(ms: number): string {
    if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)}m`;
    if (ms >= 1000) return `${(ms / 1000).toFixed(2)}s`;
    if (ms >= 1) return `${ms.toFixed(1)}ms`;
    if (ms > 0) return `${(ms * 1000).toFixed(0)}µs`;
    return `<1µs`;
  }

  if (!open) return null;

  // ── Preset apply ─────────────────────────────────────────────────────────
  function applyPreset(preset: string) {
    const p = FORMAT_PRESETS[preset];
    if (!p) return;
    setFormatPreset(preset);
    setFormatBlockSize(p.blockSize);
    setFormatInodeSize(p.inodeSize);
    setFormatInodeRatio(p.inodeRatio);
    setFormatJournalPercent(p.journalPercent);
    setFormatSecureDelete(p.secureDelete);
    setFormatErrorBehavior(p.errorBehavior);
  }

  async function handleBenchmark() {
    setBenchmarking(true);
    setBenchmarkError(null);
    setBenchmarkMs(null);
    try {
      const ms = await benchmarkKdf(
        kdf,
        kdf === "pbkdf2" ? pbkdf2Iters : undefined,
        kdf === "argon2id" ? argon2MemoryMib : undefined,
        kdf === "argon2id" ? argon2Time : undefined,
        kdf === "argon2id" ? argon2Parallelism : undefined
      );
      setBenchmarkMs(ms);
    } catch (e) {
      setBenchmarkError(String(e));
    } finally {
      setBenchmarking(false);
    }
  }

  async function handleIoBenchmark() {
    const activeSizes = [
      ...ALL_PRESETS.filter((p) => enabledKeys.has(p.key)),
      ...(enableGorlock ? [GORLOCK] : []),
    ];
    if (activeSizes.length === 0) return;

    setIoBenchState("running");
    const fmtOpts: FormatOptionsDto = {
      block_size: formatBlockSize,
      inode_size: formatInodeSize,
      inode_ratio: formatInodeRatio,
      journal_percent: formatJournalPercent,
      volume_label: formatVolumeLabel || undefined,
      secure_delete: formatSecureDelete,
      default_permissions: formatDefaultPermissions,
      error_behavior: formatErrorBehavior,
    };

    const initial: SizeResult[] = activeSizes.map((p) => ({
      label: p.label,
      bytes: p.bytes,
      status: "pending" as const,
    }));
    setIoBenchResults(initial);
    const updated = [...initial];

    const n = useAvg ? avgRuns : 1;

    for (let i = 0; i < activeSizes.length; i++) {
      updated[i] = { ...updated[i], status: "running" };
      setIoBenchResults([...updated]);
      try {
        // Averaging is now handled inside Rust (single volume per size tier).
        const res = await benchmarkFormatIo(fmtOpts, activeSizes[i].bytes, activeSizes[i].label, n);
        updated[i] = { ...updated[i], status: "done", result: res, runCount: n };
      } catch (e) {
        const msg = String(e);
        if (msg.includes("cancelled")) {
          // Mark remaining as pending and stop
          for (let j = i; j < activeSizes.length; j++) {
            updated[j] = { ...updated[j], status: "pending" };
          }
          setIoBenchResults([...updated]);
          setIoBenchState("idle");
          return;
        }
        updated[i] = { ...updated[i], status: "error", error: msg };
      }
      setIoBenchResults([...updated]);
    }
    setIoBenchState("done");
  }

  async function handleCancelBenchmark() {
    try { await cancelBenchmark(); } catch (_) { /* best-effort */ }
  }

  function handleOverlayClick(e: React.MouseEvent<HTMLDivElement>) {
    if (e.target === e.currentTarget) onClose();
  }

  const inputCls =
    "w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none";

  // ── Compute effective preset label ────────────────────────────────────────
  function isPresetActive(key: string): boolean {
    const p = FORMAT_PRESETS[key];
    if (!p) return false;
    return (
      formatBlockSize === p.blockSize &&
      formatInodeSize === p.inodeSize &&
      formatInodeRatio === p.inodeRatio &&
      formatJournalPercent === p.journalPercent &&
      formatSecureDelete === p.secureDelete &&
      formatErrorBehavior === p.errorBehavior
    );
  }

  return (
    /* ── Overlay ── */
    <div
      className="fixed inset-0 z-50 flex items-center justify-center backdrop-blur-sm bg-black/60"
      onClick={handleOverlayClick}
    >
      {/* ── Content panel ── */}
      <div
        className="w-[88%] h-[88%] bg-surface border border-border flex flex-col"
        style={{ maxWidth: 1320, maxHeight: 800 }}
      >
        {/* ── Header ── */}
        <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
          <span className="text-sm text-text-bright">Advanced Settings</span>
          <button
            type="button"
            onClick={onClose}
            className="text-text-muted hover:text-text text-sm leading-none"
          >
            &#x2715;
          </button>
        </div>

        {/* ── Tab row ── */}
        <div className="flex border-b border-border shrink-0">
          {TABS.map((tab) => (
            <button
              key={tab}
              type="button"
              onClick={() => setActiveTab(tab)}
              className={[
                "px-4 py-2 text-xs border-r border-border",
                activeTab === tab
                  ? "bg-surface-active text-text-bright border-b border-border-focus"
                  : "bg-bg text-text-muted hover:text-text",
              ].join(" ")}
            >
              {tab}
            </button>
          ))}
        </div>

        {/* ── Body (scrollable) ── */}
        <div className="flex-1 overflow-y-auto p-5 space-y-5">

          {/* ═══════════════════════════════════════════════════════════════
              TAB: Key Derivation
              ═══════════════════════════════════════════════════════════════ */}
          {activeTab === "Key Derivation" && (
            <>
              {/* ── Algorithm selector ── */}
              <div>
                <p className="text-xs text-text-muted mb-2 uppercase tracking-wider">Algorithm</p>
                <div className="flex gap-2">
                  {(["argon2id", "pbkdf2"] as const).map((algo) => {
                    const active = kdf === algo;
                    return (
                      <button
                        key={algo}
                        type="button"
                        onClick={() => {
                          setKdf(algo);
                          setBenchmarkMs(null);
                          setBenchmarkError(null);
                        }}
                        className={[
                          "px-4 py-1.5 text-xs border",
                          active
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {algo === "argon2id" ? "Argon2id" : "PBKDF2"}
                        {algo === "argon2id" && (
                          <span className="ml-1.5 text-text-muted text-[10px]">
                            recommended
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>

                {/* Algorithm help text */}
                <p className="mt-2 text-xs text-text-muted leading-relaxed">
                  {kdf === "argon2id"
                    ? "Modern memory-hard KDF. Best protection against GPU/ASIC brute-force attacks. Recommended for all new volumes."
                    : "Older Industry-standard KDF approved by NIST (SP 800-132) and OWASP. Widely compatible but less resistant to GPU/ASIC attacks than Argon2id. Use when FIPS compliance is required or for maximum interoperability."}
                </p>
              </div>

              <div className="border-t border-border" />

              {/* ── Argon2id params ── */}
              {kdf === "argon2id" && (
                <div className="space-y-4">
                  <p className="text-xs text-text-muted uppercase tracking-wider">
                    Argon2id Parameters
                  </p>

                  {/* Memory */}
                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">
                      Memory (MiB)
                    </label>
                    <input
                      type="number"
                      min={16}
                      max={256}
                      step={8}
                      value={argon2MemoryMib}
                      onChange={(e) => {
                        setArgon2MemoryMib(Math.max(16, Number(e.target.value)));
                        setBenchmarkMs(null);
                      }}
                      className={inputCls}
                    />
                    <p className="text-xs text-text-muted">
                      ↳ Higher = more secure but uses more RAM. Range: 16–256 MiB.
                      This is the primary defence against GPU/ASIC attacks.
                    </p>
                  </div>

                  {/* Time cost */}
                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">
                      Time cost (iterations)
                    </label>
                    <input
                      type="number"
                      min={1}
                      max={6}
                      step={1}
                      value={argon2Time}
                      onChange={(e) => {
                        setArgon2Time(Math.max(1, Number(e.target.value)));
                        setBenchmarkMs(null);
                      }}
                      className={inputCls}
                    />
                    <p className="text-xs text-text-muted">
                      ↳ Number of passes. Range: 1–6. Keep low (2–3) and
                      increase memory instead for better protection.
                    </p>
                  </div>

                  {/* Parallelism */}
                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">
                      Parallelism (threads)
                    </label>
                    <input
                      type="number"
                      min={1}
                      max={4}
                      step={1}
                      value={argon2Parallelism}
                      onChange={(e) => {
                        setArgon2Parallelism(Math.max(1, Number(e.target.value)));
                        setBenchmarkMs(null);
                      }}
                      className={inputCls}
                    />
                    <p className="text-xs text-text-muted">
                      ↳ Parallel threads. Range: 1–4. More threads can
                      compensate for higher memory, keeping unlock time
                      reasonable. Only helps if your CPU has spare cores.
                    </p>
                  </div>

                  {/* Argon2id presets */}
                  <div className="space-y-1">
                    <p className="text-xs text-text-muted uppercase tracking-wider">
                      Presets
                    </p>
                    <div className="flex gap-2">
                      {[
                        { label: "Fast", memory: 16, time: 1, p: 2 },
                        { label: "Balanced", memory: 32, time: 2, p: 2 },
                        { label: "Maximum", memory: 256, time: 4, p: 4 },
                      ].map((preset) => {
                        const active =
                          argon2MemoryMib === preset.memory &&
                          argon2Time === preset.time &&
                          argon2Parallelism === preset.p;
                        return (
                          <button
                            key={preset.label}
                            type="button"
                            onClick={() => {
                              setArgon2MemoryMib(preset.memory);
                              setArgon2Time(preset.time);
                              setArgon2Parallelism(preset.p);
                              setBenchmarkMs(null);
                            }}
                            className={[
                              "px-3 py-1 text-xs border",
                              active
                                ? "bg-surface-active text-text-bright border-border-focus"
                                : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                            ].join(" ")}
                          >
                            {preset.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              )}

              {/* ── PBKDF2 params ── */}
              {kdf === "pbkdf2" && (
                <div className="space-y-4">
                  <p className="text-xs text-text-muted uppercase tracking-wider">
                    PBKDF2-HMAC-SHA256 Parameters
                  </p>

                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">
                      Iterations
                    </label>
                    <select
                      value={pbkdf2Iters}
                      onChange={(e) => {
                        setPbkdf2Iters(Number(e.target.value));
                        setBenchmarkMs(null);
                      }}
                      className="w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none appearance-none"
                    >
                      {PBKDF2_PRESETS.map((p) => (
                        <option key={p.value} value={p.value}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <p className="text-xs text-text-muted">
                      ↳ More iterations = slower unlock, but also slower for
                      attackers. OWASP recommends ≥600,000 for
                      PBKDF2-HMAC-SHA256 as of 2025.
                    </p>
                    {pbkdf2Iters > 1_000_000 && (
                      <p className="text-xs text-error">
                        ⚠ Values above 1,000,000 may cause noticeable unlock
                        delays on some hardware.
                      </p>
                    )}
                  </div>

                  {/* PBKDF2 presets */}
                  <div className="space-y-1">
                    <p className="text-xs text-text-muted uppercase tracking-wider">
                      Presets
                    </p>
                    <div className="flex gap-2 flex-wrap">
                      {PBKDF2_PRESETS.map((preset) => {
                        const active = pbkdf2Iters === preset.value;
                        return (
                          <button
                            key={preset.value}
                            type="button"
                            onClick={() => {
                              setPbkdf2Iters(preset.value);
                              setBenchmarkMs(null);
                            }}
                            className={[
                              "px-3 py-1 text-xs border",
                              active
                                ? "bg-surface-active text-text-bright border-border-focus"
                                : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                            ].join(" ")}
                          >
                            {preset.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              )}

              <div className="border-t border-border" />

              {/* ── Benchmark ── */}
              <div className="space-y-2">
                <button
                  type="button"
                  onClick={handleBenchmark}
                  disabled={benchmarking}
                  className="px-4 py-1.5 text-xs border border-border bg-surface text-text hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {benchmarking ? "Running benchmark…" : "⏱ Run Benchmark"}
                </button>

                {benchmarkMs !== null && !benchmarkError && (
                  <p className="text-xs text-success">
                    Estimated unlock time:{" "}
                    {benchmarkMs < 1000
                      ? `${benchmarkMs}ms`
                      : `${(benchmarkMs / 1000).toFixed(2)}s`}
                  </p>
                )}
                {benchmarkError && (
                  <p className="text-xs text-error">Benchmark failed: {benchmarkError}</p>
                )}
              </div>

              {/* ── Security note ── */}
              <div className="border border-border px-3 py-2 bg-bg">
                <p className="text-xs text-text-muted leading-relaxed">
                  <span className="text-text">Note:</span> Parameters are
                  embedded in the volume header. You must use the same password
                  to unlock — parameters are recovered automatically.
                </p>
              </div>
            </>
          )}

          {/* ═══════════════════════════════════════════════════════════════
              TAB: Volume Format
              ═══════════════════════════════════════════════════════════════ */}
          {activeTab === "Volume Format" && (
            <>
              {/* ── Presets ── */}
              <div>
                <p className="text-xs text-text-muted mb-2 uppercase tracking-wider">Preset</p>
                <div className="flex flex-wrap gap-2">
                  {Object.keys(FORMAT_PRESETS).map((key) => {
                    const active = isPresetActive(key);
                    return (
                      <button
                        key={key}
                        type="button"
                        onClick={() => applyPreset(key)}
                        className={[
                          "px-3 py-1.5 text-xs border",
                          active
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {PRESET_LABELS[key]}
                      </button>
                    );
                  })}
                </div>
                {/* Preset description */}
                {Object.keys(FORMAT_PRESETS).map((key) => isPresetActive(key) ? (
                  <p key={key} className="mt-2 text-xs text-text-muted leading-relaxed">
                    {PRESET_DESCRIPTIONS[key]}
                  </p>
                ) : null)}
              </div>

              <div className="border-t border-border" />

              {/* ── Block & Inode sizes ── */}
              <div>
                <p className="text-xs text-text-muted mb-3 uppercase tracking-wider">Block &amp; Inode Layout</p>
                <div className="grid grid-cols-2 gap-4">

                  {/* Block size */}
                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">Block Size (bytes)</label>
                    <select
                      value={formatBlockSize}
                      onChange={(e) => setFormatBlockSize(Number(e.target.value))}
                      className="w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none appearance-none"
                    >
                      {BLOCK_SIZES.map((bs) => (
                        <option key={bs} value={bs}>
                          {bs >= 1024 ? `${bs / 1024} KiB` : `${bs} B`} ({bs})
                        </option>
                      ))}
                    </select>
                    <p className="text-xs text-text-muted">
                      ↳ Larger blocks reduce metadata overhead for big files. Smaller blocks cut internal fragmentation for tiny files. Default: 4 KiB.
                    </p>
                  </div>

                  {/* Inode size */}
                  <div className="space-y-1">
                    <label className="block text-sm text-text-muted">Inode Size</label>
                    <div className="flex gap-2">
                      {([128, 256] as const).map((sz) => {
                        const active = formatInodeSize === sz;
                        return (
                          <button
                            key={sz}
                            type="button"
                            onClick={() => setFormatInodeSize(sz)}
                            className={[
                              "flex-1 py-1.5 text-xs border",
                              active
                                ? "bg-surface-active text-text-bright border-border-focus"
                                : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                            ].join(" ")}
                          >
                            {sz}B {sz === 256 ? "(v3 full)" : "(v2 legacy)"}
                          </button>
                        );
                      })}
                    </div>
                    <p className="text-xs text-text-muted">
                      ↳ 256B enables checksums, extended timestamps, and extent trees. 128B is v2-compatible but lacks v3 features.
                    </p>
                  </div>
                </div>

                {/* Inode ratio */}
                <div className="mt-4 space-y-1">
                  <label className="block text-sm text-text-muted">Bytes per Inode</label>
                  <input
                    type="number"
                    min={1024}
                    max={65536}
                    step={1024}
                    value={formatInodeRatio}
                    onChange={(e) => setFormatInodeRatio(Math.max(1024, Math.min(65536, Number(e.target.value))))}
                    className={inputCls}
                  />
                  <p className="text-xs text-text-muted">
                    ↳ One inode is allocated per this many bytes of total volume size. Lower = more inodes available. Range: 1,024–65,536.
                    Default: 16,384 (one inode per 16 KiB).
                  </p>
                </div>
              </div>

              <div className="border-t border-border" />

              {/* ── Journal ── */}
              <div>
                <p className="text-xs text-text-muted mb-2 uppercase tracking-wider">Journal (Write-Ahead Log)</p>
                <div className="flex flex-wrap gap-2">
                  {JOURNAL_PERCENTS.map((pct) => {
                    const active = formatJournalPercent === pct;
                    return (
                      <button
                        key={pct}
                        type="button"
                        onClick={() => setFormatJournalPercent(pct)}
                        className={[
                          "px-3 py-1.5 text-xs border min-w-[52px] text-center",
                          active
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {pct === 0 ? "None" : `${pct}%`}
                      </button>
                    );
                  })}
                </div>
                <p className="mt-2 text-xs text-text-muted leading-relaxed">
                  {formatJournalPercent === 0
                    ? "No journal. Faster writes but metadata may be inconsistent after a crash. Use only with --minimal preset or when performance is critical."
                    : `Journal occupies ${formatJournalPercent}% of volume space. Protects metadata integrity across crashes. Higher % = fewer replays needed before full journal flush.`}
                </p>
              </div>

              <div className="border-t border-border" />

              {/* ── Volume label ── */}
              <div>
                <p className="text-xs text-text-muted mb-2 uppercase tracking-wider">Volume Label</p>
                <input
                  type="text"
                  maxLength={31}
                  value={formatVolumeLabel}
                  onChange={(e) => setFormatVolumeLabel(e.target.value.slice(0, 31))}
                  placeholder="Optional — up to 31 characters"
                  className={inputCls}
                  spellCheck={false}
                />
                <p className="mt-1 text-xs text-text-muted">
                  ↳ Human-readable label stored in the superblock. Shown in volume info. Max 31 bytes UTF-8.
                </p>
              </div>

              <div className="border-t border-border" />

              {/* ── Security & Permissions ── */}
              <div>
                <p className="text-xs text-text-muted mb-3 uppercase tracking-wider">Security &amp; Permissions</p>

                <div className="grid grid-cols-2 gap-4">
                  {/* Secure delete */}
                  <div className="space-y-2">
                    <label className="block text-sm text-text-muted">Secure Delete</label>
                    <div className="flex gap-2">
                      {([true, false] as const).map((val) => {
                        const active = formatSecureDelete === val;
                        return (
                          <button
                            key={String(val)}
                            type="button"
                            onClick={() => setFormatSecureDelete(val)}
                            className={[
                              "flex-1 py-1.5 text-xs border",
                              active
                                ? "bg-surface-active text-text-bright border-border-focus"
                                : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                            ].join(" ")}
                          >
                            {val ? "Enabled" : "Disabled"}
                          </button>
                        );
                      })}
                    </div>
                    <p className="text-xs text-text-muted">
                      ↳ When enabled, freed data blocks are zeroed before release. Prevents recovery of deleted file content.
                    </p>
                  </div>

                  {/* Error behavior */}
                  <div className="space-y-2">
                    <label className="block text-sm text-text-muted">On Error</label>
                    <div className="flex gap-2">
                      {([
                        { label: "Continue", value: "continue" },
                        { label: "Read-Only", value: "read-only" },
                      ] as const).map(({ label, value }) => {
                        const active = formatErrorBehavior === value;
                        return (
                          <button
                            key={value}
                            type="button"
                            onClick={() => setFormatErrorBehavior(value)}
                            className={[
                              "flex-1 py-1.5 text-xs border",
                              active
                                ? "bg-surface-active text-text-bright border-border-focus"
                                : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                            ].join(" ")}
                          >
                            {label}
                          </button>
                        );
                      })}
                    </div>
                    <p className="text-xs text-text-muted">
                      ↳ Continue: log errors and keep operating. Read-Only: remount read-only on any checksum or I/O error.
                    </p>
                  </div>
                </div>

                {/* Default permissions */}
                <div className="mt-4 space-y-1">
                  <label className="block text-sm text-text-muted">Default Permissions (octal)</label>
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      maxLength={4}
                      value={permOctalText}
                      onChange={(e) => {
                        const raw = e.target.value.replace(/[^0-7]/g, "");
                        setPermOctalText(raw);
                        const parsed = parseOctalStr(raw);
                        if (!isNaN(parsed)) setFormatDefaultPermissions(parsed);
                      }}
                      onBlur={() => {
                        // Normalize on blur to always show 3 digits
                        setPermOctalText(toOctalStr(formatDefaultPermissions));
                      }}
                      className="w-24 px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none font-mono"
                      spellCheck={false}
                    />
                    <span className="text-xs text-text-muted">
                      = {formatDefaultPermissions.toString(8).padStart(3, "0")} octal
                      {" · "}
                      {/* Quick decode */}
                      {["rwx", "rwx", "rwx"].map((_, gi) => {
                        const bits = (formatDefaultPermissions >> ((2 - gi) * 3)) & 0b111;
                        return (
                          <span key={gi} className="font-mono">
                            {bits & 4 ? "r" : "-"}
                            {bits & 2 ? "w" : "-"}
                            {bits & 1 ? "x" : "-"}
                          </span>
                        );
                      })}
                    </span>
                  </div>
                  <div className="flex gap-2 mt-1">
                    {[
                      { label: "755 (dirs)", value: 0o755 },
                      { label: "644 (files)", value: 0o644 },
                      { label: "700 (private)", value: 0o700 },
                    ].map(({ label, value }) => (
                      <button
                        key={value}
                        type="button"
                        onClick={() => {
                          setFormatDefaultPermissions(value);
                          setPermOctalText(toOctalStr(value));
                        }}
                        className={[
                          "px-2 py-1 text-xs border",
                          formatDefaultPermissions === value
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  <p className="text-xs text-text-muted">
                    ↳ Unix-style permission mask applied to newly created files and directories. Enter as 3-digit octal (e.g. 755).
                  </p>
                </div>
              </div>

              <div className="border-t border-border" />

              {/* ── I/O Benchmark ── */}
              <div>
                <p className="text-xs text-text-muted mb-3 uppercase tracking-wider">I/O Benchmark</p>
                <p className="text-xs text-text-muted mb-3 leading-relaxed">
                  Creates a temporary volume using the format settings above and measures sequential write/read throughput.
                  Select which sizes to include, configure averaging, then click Run.
                </p>

                {/* Test selection */}
                <div className="mb-3">
                  <p className="text-xs text-text-muted mb-2">Test Sizes</p>
                  <div className="grid grid-cols-3 gap-x-6 gap-y-1.5">
                    {ALL_PRESETS.map((p) => (
                      <label key={p.key} className="flex items-center gap-2 cursor-pointer select-none">
                        <input
                          type="checkbox"
                          checked={enabledKeys.has(p.key)}
                          onChange={(e) => {
                            const next = new Set(enabledKeys);
                            if (e.target.checked) next.add(p.key); else next.delete(p.key);
                            setEnabledKeys(next);
                          }}
                          disabled={ioBenchState === "running"}
                          className="accent-text"
                        />
                        <span className="text-sm text-text">{p.label}</span>
                      </label>
                    ))}
                    {/* Gorlock — heavyweight, separate warning */}
                    <label className="flex items-center gap-2 cursor-pointer select-none col-span-2">
                      <input
                        type="checkbox"
                        checked={enableGorlock}
                        onChange={(e) => setEnableGorlock(e.target.checked)}
                        disabled={ioBenchState === "running"}
                        className="accent-text"
                      />
                      <span className="text-sm text-text">Gorlock (4 GiB)</span>
                      <span className="text-xs text-text-muted">⚠ needs ~4 GiB free on system drive, takes several minutes</span>
                    </label>
                  </div>
                </div>

                <div className="border-t border-border mb-3" />

                {/* Averaging */}
                <div className="mb-3">
                  <label className="flex items-center gap-2 cursor-pointer select-none mb-2">
                    <input
                      type="checkbox"
                      checked={useAvg}
                      onChange={(e) => setUseAvg(e.target.checked)}
                      disabled={ioBenchState === "running"}
                      className="accent-text"
                    />
                    <span className="text-sm text-text">Average consecutive runs</span>
                  </label>
                  {useAvg && (
                    <div className="flex items-center gap-2 ml-5">
                      <span className="text-xs text-text-muted">Runs:</span>
                      {([2, 3, 5, 10] as const).map((n) => (
                        <button
                          key={n}
                          type="button"
                          onClick={() => setAvgRuns(n)}
                          disabled={ioBenchState === "running"}
                          className={[
                            "px-2.5 py-0.5 text-xs border",
                            avgRuns === n
                              ? "bg-surface-active text-text-bright border-border-focus"
                              : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                          ].join(" ")}
                        >
                          {n}
                        </button>
                      ))}
                      <span className="text-xs text-text-muted">
                        — each test runs {avgRuns}×, result is the mean
                      </span>
                    </div>
                  )}
                </div>

                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={handleIoBenchmark}
                    disabled={ioBenchState === "running" || (enabledKeys.size === 0 && !enableGorlock)}
                    className="px-4 py-1.5 text-sm border border-border bg-bg text-text hover:border-border-focus hover:text-text-bright disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    {ioBenchState === "running" ? "Running…" : ioBenchState === "done" ? "⟳ Re-run" : "⏱ Run Benchmark"}
                  </button>
                  {ioBenchState === "running" && (
                    <button
                      type="button"
                      onClick={handleCancelBenchmark}
                      className="px-4 py-1.5 text-sm border border-border bg-bg text-error hover:border-error hover:text-error"
                    >
                      ✕ Cancel
                    </button>
                  )}
                </div>

                {ioBenchResults.length > 0 && (
                  <div className="mt-3 border border-border">
                    <table className="w-full text-xs">
                      <thead>
                        <tr className="border-b border-border bg-bg">
                          <th className="text-left px-2 py-1 text-text-muted font-normal">Size</th>
                          <th className="text-left px-2 py-1 text-text-muted font-normal">Status</th>
                          <th className="text-right px-2 py-1 text-text-muted font-normal">Write</th>
                          <th className="text-right px-2 py-1 text-text-muted font-normal">Read</th>
                          <th className="text-right px-2 py-1 text-text-muted font-normal">W Time</th>
                          <th className="text-right px-2 py-1 text-text-muted font-normal">R Time</th>
                          <th className="text-right px-2 py-1 text-text-muted font-normal">Sync</th>
                        </tr>
                      </thead>
                      <tbody>
                        {ioBenchResults.map((r, i) => (
                          <tr key={i} className="border-b border-border last:border-b-0">
                            <td className="px-2 py-1 text-text">{r.label}</td>
                            <td className="px-2 py-1">
                              {r.status === "pending" && <span className="text-text-muted">pending</span>}
                              {r.status === "running" && <span className="text-text-bright">running…</span>}
                              {r.status === "done" && (
                                <span className="text-text">
                                  ✓{r.runCount && r.runCount > 1
                                    ? <span className="text-text-muted"> avg×{r.runCount}</span>
                                    : null}
                                </span>
                              )}
                              {r.status === "error" && <span className="text-error" title={r.error}>✗</span>}
                            </td>
                            <td className="px-2 py-1 text-right text-text font-mono">
                              {r.result ? fmtSpeed(r.result.write_speed_mbps) : "—"}
                            </td>
                            <td className="px-2 py-1 text-right text-text font-mono">
                              {r.result ? fmtSpeed(r.result.read_speed_mbps) : "—"}
                            </td>
                            <td className="px-2 py-1 text-right text-text-muted font-mono">
                              {r.result ? fmtTime(r.result.write_time_ms) : "—"}
                            </td>
                            <td className="px-2 py-1 text-right text-text-muted font-mono">
                              {r.result ? fmtTime(r.result.read_time_ms) : "—"}
                            </td>
                            <td className="px-2 py-1 text-right text-text-muted font-mono">
                              {r.result ? fmtTime(r.result.sync_time_ms) : "—"}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>

              <div className="border-t border-border" />

              {/* ── Info box ── */}
              <div className="border border-border px-3 py-2 bg-bg">
                <p className="text-xs text-text-muted leading-relaxed">
                  <span className="text-text">Note:</span> These parameters are written to the superblock at format time and{" "}
                  <span className="text-text">cannot be changed after the volume is created</span>.
                  Choose them carefully before clicking Create Volume.
                </p>
                <p className="text-xs text-text-muted leading-relaxed mt-1">
                  <span className="text-text">Benchmark info:</span> Speeds measure full CFS API throughput
                  (path resolution, locking, block allocation, extent tree, caching) — not raw disk speed.
                  Sync time is measured separately. Multi-run averaging reuses a single volume; later runs
                  benefit from warm OS page cache.
                </p>
              </div>
            </>
          )}
        </div>

        {/* ── Footer ── */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-border shrink-0">
          <button
            type="button"
            onClick={onClose}
            className="px-5 py-1.5 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
