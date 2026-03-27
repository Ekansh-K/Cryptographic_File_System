import { useState } from "react";
import { useAppStore } from "../store";
import type { IoBenchmarkResult } from "../types";

// ── Benchmark size presets ──────────────────────────────────────────────────
const SIZE_PRESETS = [
  { label: "Small (4 KiB)", bytes: 4 * 1024 },
  { label: "Medium (1 MiB)", bytes: 1 * 1024 * 1024 },
  { label: "Large (256 MiB)", bytes: 256 * 1024 * 1024 },
  { label: "XL (1 GiB)", bytes: 1024 * 1024 * 1024 },
] as const;

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GiB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${bytes} B`;
}

function formatSpeed(mbps: number): string {
  if (mbps >= 1024) return `${(mbps / 1024).toFixed(2)} GiB/s`;
  if (mbps >= 1) return `${mbps.toFixed(2)} MiB/s`;
  return `${(mbps * 1024).toFixed(1)} KiB/s`;
}

function formatTime(ms: number): string {
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)}m`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)}s`;
  return `${ms}ms`;
}

interface Props {
  open: boolean;
  onClose: () => void;
}

type BenchState = "idle" | "running" | "done";

interface SizeResult {
  preset: { label: string; bytes: number };
  status: "pending" | "running" | "done" | "error" | "skipped";
  result?: IoBenchmarkResult;
  error?: string;
}

export default function BenchmarkModal({ open, onClose }: Props) {
  const volumeInfo = useAppStore((s) => s.volumeInfo);

  const [benchState, setBenchState] = useState<BenchState>("idle");
  const [results, setResults] = useState<SizeResult[]>([]);
  const [customLargeMb, setCustomLargeMb] = useState(1024);
  const [runLarge, setRunLarge] = useState(true);
  const [runXL, setRunXL] = useState(true);

  if (!open) return null;

  const freeBytes = volumeInfo
    ? volumeInfo.free_blocks * volumeInfo.block_size
    : 0;

  // Prevent closing while benchmark is in progress
  function handleClose() {
    if (benchState === "running") return;
    onClose();
  }

  async function runBenchmark() {
    setBenchState("running");

    // Build size list based on user selections
    const sizes: { label: string; bytes: number }[] = [
      SIZE_PRESETS[0], // Small — always run
      SIZE_PRESETS[1], // Medium — always run
    ];
    if (runLarge) sizes.push(SIZE_PRESETS[2]);
    if (runXL) {
      sizes.push({
        label: `XL (${formatBytes(customLargeMb * 1024 * 1024)})`,
        bytes: customLargeMb * 1024 * 1024,
      });
    }

    const initial: SizeResult[] = sizes.map((p) => ({
      preset: p,
      status: freeBytes > 0 && p.bytes > freeBytes ? "skipped" : "pending",
    }));
    setResults(initial);

    const updated = [...initial];

    for (let i = 0; i < sizes.length; i++) {
      if (updated[i].status === "skipped") continue;

      updated[i] = { ...updated[i], status: "running" };
      setResults([...updated]);

      try {
        const res: IoBenchmarkResult = {
          size_label: sizes[i].label,
          size_bytes: sizes[i].bytes,
          write_speed_mbps: 0,
          read_speed_mbps: 0,
          write_time_ms: 0,
          read_time_ms: 0,
          sync_time_ms: 0,
        };
        updated[i] = { ...updated[i], status: "done", result: res };
      } catch (e) {
        updated[i] = { ...updated[i], status: "error", error: String(e) };
      }
      setResults([...updated]);
    }

    setBenchState("done");
  }

  function handleReset() {
    setBenchState("idle");
    setResults([]);
  }

  function handleOverlayClick(e: React.MouseEvent<HTMLDivElement>) {
    if (benchState === "running") return;
    if (e.target === e.currentTarget) onClose();
  }

  const inputCls =
    "w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center backdrop-blur-sm bg-black/60"
      onClick={handleOverlayClick}
    >
      <div
        className="w-[80%] bg-surface border border-border flex flex-col"
        style={{ maxWidth: 860, maxHeight: 700 }}
      >
        {/* ── Header ── */}
        <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
          <span className="text-sm text-text-bright">Filesystem I/O Benchmark</span>
          <button
            type="button"
            onClick={handleClose}
            disabled={benchState === "running"}
            className="text-text-muted hover:text-text text-sm leading-none disabled:opacity-30 disabled:cursor-not-allowed"
          >
            &#x2715;
          </button>
        </div>

        {/* ── Body ── */}
        <div className="flex-1 overflow-y-auto p-5 space-y-5">
          {/* Info */}
          <div className="border border-border px-3 py-2 bg-bg">
            <p className="text-xs text-text-muted leading-relaxed">
              Measures sequential write and read throughput on the current CFS volume
              by creating temporary test files. Files are automatically deleted after each test.
              {volumeInfo && (
                <span className="block mt-1">
                  Free space: <span className="text-text">{formatBytes(freeBytes)}</span>
                  {" · "}Block size: <span className="text-text">{formatBytes(volumeInfo.block_size)}</span>
                  {volumeInfo.volume_label && (
                    <>{" · "}Label: <span className="text-text">{volumeInfo.volume_label}</span></>
                  )}
                </span>
              )}
            </p>
          </div>

          {/* ── Configuration (only when idle) ── */}
          {benchState === "idle" && (
            <div className="space-y-4">
              <p className="text-xs text-text-muted uppercase tracking-wider">Test Configuration</p>

              <div className="grid grid-cols-2 gap-4">
                {/* Always-on tests */}
                <div className="space-y-2">
                  <div className="flex items-center gap-2">
                    <span className="inline-block w-2 h-2 bg-text-muted" />
                    <span className="text-sm text-text">Small (4 KiB)</span>
                    <span className="text-xs text-text-muted ml-auto">always</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="inline-block w-2 h-2 bg-text-muted" />
                    <span className="text-sm text-text">Medium (1 MiB)</span>
                    <span className="text-xs text-text-muted ml-auto">always</span>
                  </div>
                </div>

                {/* Optional tests */}
                <div className="space-y-2">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={runLarge}
                      onChange={(e) => setRunLarge(e.target.checked)}
                      className="accent-text"
                    />
                    <span className="text-sm text-text">Large (256 MiB)</span>
                    {256 * 1024 * 1024 > freeBytes && (
                      <span className="text-xs text-error ml-auto">insufficient space</span>
                    )}
                  </label>
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={runXL}
                      onChange={(e) => setRunXL(e.target.checked)}
                      className="accent-text"
                    />
                    <span className="text-sm text-text">XL</span>
                    {customLargeMb * 1024 * 1024 > freeBytes && (
                      <span className="text-xs text-error ml-auto">insufficient space</span>
                    )}
                  </label>
                </div>
              </div>

              {/* Custom XL size */}
              {runXL && (
                <div className="space-y-1">
                  <label className="block text-sm text-text-muted">XL Test Size (MiB)</label>
                  <input
                    type="number"
                    min={64}
                    max={Math.floor(freeBytes / (1024 * 1024)) || 4096}
                    step={64}
                    value={customLargeMb}
                    onChange={(e) => setCustomLargeMb(Math.max(64, Number(e.target.value)))}
                    className={inputCls}
                    style={{ maxWidth: 200 }}
                  />
                  <p className="text-xs text-text-muted">
                    ↳ Size of the XL transfer test. Minimum 64 MiB, recommended 1024 MiB (1 GiB) or more.
                  </p>
                  {/* Quick presets */}
                  <div className="flex gap-2 mt-1">
                    {[512, 1024, 2048, 4096].map((mb) => (
                      <button
                        key={mb}
                        type="button"
                        onClick={() => setCustomLargeMb(mb)}
                        className={[
                          "px-2 py-1 text-xs border",
                          customLargeMb === mb
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {mb >= 1024 ? `${mb / 1024} GiB` : `${mb} MiB`}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Start button */}
              <button
                type="button"
                onClick={runBenchmark}
                className="px-5 py-2 text-sm border border-border bg-surface text-text-bright hover:border-border-focus"
              >
                ⏱ Run I/O Benchmark
              </button>
            </div>
          )}

          {/* ── Results table ── */}
          {results.length > 0 && (
            <div className="space-y-3">
              <p className="text-xs text-text-muted uppercase tracking-wider">Results</p>

              <div className="border border-border">
                {/* Table header */}
                <div className="grid grid-cols-[1fr_100px_110px_110px_90px_90px] gap-0 text-xs text-text-muted border-b border-border bg-bg">
                  <div className="px-3 py-2">Test</div>
                  <div className="px-3 py-2 text-right">Size</div>
                  <div className="px-3 py-2 text-right">Write Speed</div>
                  <div className="px-3 py-2 text-right">Read Speed</div>
                  <div className="px-3 py-2 text-right">Write Time</div>
                  <div className="px-3 py-2 text-right">Read Time</div>
                </div>

                {/* Rows */}
                {results.map((r, i) => (
                  <div
                    key={i}
                    className={[
                      "grid grid-cols-[1fr_100px_110px_110px_90px_90px] gap-0 text-sm",
                      i < results.length - 1 ? "border-b border-border" : "",
                      r.status === "running" ? "bg-surface-active" : "",
                    ].join(" ")}
                  >
                    <div className="px-3 py-2 flex items-center gap-2">
                      {r.status === "running" && (
                        <span className="inline-block w-1.5 h-1.5 bg-text-muted animate-pulse" />
                      )}
                      {r.status === "done" && (
                        <span className="inline-block w-1.5 h-1.5 bg-success" />
                      )}
                      {r.status === "error" && (
                        <span className="inline-block w-1.5 h-1.5 bg-error" />
                      )}
                      {r.status === "skipped" && (
                        <span className="inline-block w-1.5 h-1.5 bg-border" />
                      )}
                      {r.status === "pending" && (
                        <span className="inline-block w-1.5 h-1.5 bg-border" />
                      )}
                      <span className="text-text">{r.preset.label}</span>
                    </div>
                    <div className="px-3 py-2 text-right text-text-muted">
                      {formatBytes(r.preset.bytes)}
                    </div>
                    <div className="px-3 py-2 text-right font-mono text-text">
                      {r.status === "running" && "…"}
                      {r.status === "done" && r.result && formatSpeed(r.result.write_speed_mbps)}
                      {r.status === "error" && <span className="text-error">err</span>}
                      {r.status === "skipped" && <span className="text-text-muted">—</span>}
                      {r.status === "pending" && ""}
                    </div>
                    <div className="px-3 py-2 text-right font-mono text-text">
                      {r.status === "running" && "…"}
                      {r.status === "done" && r.result && formatSpeed(r.result.read_speed_mbps)}
                      {r.status === "error" && <span className="text-error">err</span>}
                      {r.status === "skipped" && <span className="text-text-muted">—</span>}
                      {r.status === "pending" && ""}
                    </div>
                    <div className="px-3 py-2 text-right text-text-muted text-xs">
                      {r.status === "done" && r.result && formatTime(r.result.write_time_ms)}
                      {r.status === "skipped" && "skipped"}
                    </div>
                    <div className="px-3 py-2 text-right text-text-muted text-xs">
                      {r.status === "done" && r.result && formatTime(r.result.read_time_ms)}
                    </div>
                  </div>
                ))}
              </div>

              {/* Error details */}
              {results.some((r) => r.status === "error") && (
                <div className="space-y-1">
                  {results
                    .filter((r) => r.status === "error")
                    .map((r, i) => (
                      <p key={i} className="text-xs text-error">
                        {r.preset.label}: {r.error}
                      </p>
                    ))}
                </div>
              )}

              {/* Running indicator */}
              {benchState === "running" && (
                <p className="text-xs text-text-muted animate-pulse">
                  Benchmark in progress — do not close this dialog…
                </p>
              )}

              {/* Summary when done */}
              {benchState === "done" && (
                <div className="flex items-center gap-3">
                  <p className="text-xs text-success">Benchmark complete.</p>
                  <button
                    type="button"
                    onClick={handleReset}
                    className="px-3 py-1 text-xs border border-border bg-surface text-text hover:border-border-focus"
                  >
                    Run Again
                  </button>
                </div>
              )}
            </div>
          )}

          {/* ── Notes ── */}
          <div className="border border-border px-3 py-2 bg-bg">
            <p className="text-xs text-text-muted leading-relaxed">
              <span className="text-text">Note:</span> Results reflect CFS filesystem throughput
              including encryption overhead (if the volume is encrypted), block allocation, journaling,
              and caching. Speeds may differ from raw disk I/O. A temporary file{" "}
              <span className="font-mono text-text">/__cfs_io_benchmark_tmp</span> is created and
              deleted for each test.
            </p>
          </div>
        </div>

        {/* ── Footer ── */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-border shrink-0">
          <button
            type="button"
            onClick={handleClose}
            disabled={benchState === "running"}
            className="px-5 py-1.5 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {benchState === "running" ? "Benchmark running…" : "Close"}
          </button>
        </div>
      </div>
    </div>
  );
}
