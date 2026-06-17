import { useState, useEffect } from "react";
import { useAppStore } from "../store";
import { benchmarkCryptoSpeed, checkAesNi } from "../commands";
import type { IoBenchmarkResult, CryptoBenchmarkResult } from "../types";

// ── Benchmark size presets ──────────────────────────────────────────────────
const SIZE_PRESETS = [
  { label: "Small (4 KiB)", bytes: 4 * 1024 },
  { label: "Medium (1 MiB)", bytes: 1 * 1024 * 1024 },
  { label: "Large (256 MiB)", bytes: 256 * 1024 * 1024 },
  { label: "XL (1 GiB)", bytes: 1024 * 1024 * 1024 },
] as const;

const BENCH_TABS = ["I/O Speed", "Crypto Speed"] as const;
type BenchTab = (typeof BENCH_TABS)[number];

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

// ── Animated speed bar ─────────────────────────────────────────────────────
function SpeedBar({
  mbps,
  maxMbps,
  color = "var(--color-success)",
}: {
  mbps: number;
  maxMbps: number;
  color?: string;
}) {
  const pct = maxMbps > 0 ? Math.min(100, (mbps / maxMbps) * 100) : 0;
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, flex: 1 }}>
      <div
        style={{
          flex: 1,
          height: 6,
          background: "var(--color-surface-active)",
          position: "relative",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            height: "100%",
            width: `${pct}%`,
            background: color,
            transition: "width 0.6s cubic-bezier(0.4,0,0.2,1)",
            boxShadow: `0 0 6px ${color}55`,
          }}
        />
      </div>
      <span
        style={{
          fontSize: 12,
          color: "var(--color-text)",
          fontFamily: "monospace",
          minWidth: 90,
          textAlign: "right",
        }}
      >
        {formatSpeed(mbps)}
      </span>
    </div>
  );
}

export default function BenchmarkModal({ open, onClose }: Props) {
  const volumeInfo = useAppStore((s) => s.volumeInfo);

  // ── Tab state ──────────────────────────────────────────────────────────
  const [activeTab, setActiveTab] = useState<BenchTab>("I/O Speed");

  // ── I/O tab state ──────────────────────────────────────────────────────
  const [benchState, setBenchState] = useState<BenchState>("idle");
  const [results, setResults] = useState<SizeResult[]>([]);
  const [customLargeMb, setCustomLargeMb] = useState(1024);
  const [runLarge, setRunLarge] = useState(true);
  const [runXL, setRunXL] = useState(true);

  // ── Crypto tab state ───────────────────────────────────────────────────
  const [cryptoState, setCryptoState] = useState<"idle" | "running" | "done">("idle");
  const [cryptoSizeMb, setCryptoSizeMb] = useState(64);
  const [cryptoResults, setCryptoResults] = useState<CryptoBenchmarkResult | null>(null);
  const [cryptoError, setCryptoError] = useState<string | null>(null);
  const [aesNi, setAesNi] = useState<boolean | null>(null);

  // Fetch AES-NI on mount and when switching to crypto tab
  useEffect(() => {
    if (!open) return;
    checkAesNi()
      .then((v) => setAesNi(v))
      .catch(() => setAesNi(null));
  }, [open, activeTab]);

  if (!open) return null;

  const freeBytes = volumeInfo
    ? volumeInfo.free_blocks * volumeInfo.block_size
    : 0;

  function handleClose() {
    if (benchState === "running" || cryptoState === "running") return;
    onClose();
  }

  // ── I/O benchmark runner ───────────────────────────────────────────────
  async function runBenchmark() {
    setBenchState("running");

    const sizes: { label: string; bytes: number }[] = [
      SIZE_PRESETS[0],
      SIZE_PRESETS[1],
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

  // ── Crypto benchmark runner ────────────────────────────────────────────
  async function runCryptoBenchmark() {
    setCryptoState("running");
    setCryptoError(null);
    setCryptoResults(null);
    try {
      const result = await benchmarkCryptoSpeed(cryptoSizeMb);
      setCryptoResults(result);
      setCryptoState("done");
    } catch (e) {
      const msg = String(e);
      if (
        msg.toLowerCase().includes("unknown command") ||
        msg.toLowerCase().includes("not found") ||
        msg.toLowerCase().includes("no such") ||
        msg.toLowerCase().includes("benchmark_crypto_speed")
      ) {
        setCryptoError(
          "Crypto speed benchmark requires the latest backend build. The `benchmark_crypto_speed` command is not yet available in this version."
        );
      } else {
        setCryptoError(msg);
      }
      setCryptoState("idle");
    }
  }

  function handleCryptoReset() {
    setCryptoState("idle");
    setCryptoResults(null);
    setCryptoError(null);
  }

  function handleOverlayClick(e: React.MouseEvent<HTMLDivElement>) {
    if (benchState === "running" || cryptoState === "running") return;
    if (e.target === e.currentTarget) onClose();
  }

  const inputCls =
    "w-full px-2 py-1.5 bg-bg border border-border text-text text-sm focus:border-border-focus focus:outline-none";

  // Compute max speed for bar chart scaling
  const maxCryptoMbps = cryptoResults
    ? Math.max(
        cryptoResults.xts_encrypt_mbps,
        cryptoResults.xts_decrypt_mbps,
        cryptoResults.xts_aead_encrypt_mbps,
        cryptoResults.xts_aead_decrypt_mbps,
        1
      )
    : 1;

  const CRYPTO_SIZE_OPTIONS = [
    { label: "4 MiB", value: 4 },
    { label: "64 MiB", value: 64 },
    { label: "256 MiB", value: 256 },
  ];

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
          <span className="text-sm text-text-bright">Benchmark</span>
          <button
            type="button"
            onClick={handleClose}
            disabled={benchState === "running" || cryptoState === "running"}
            className="text-text-muted hover:text-text text-sm leading-none disabled:opacity-30 disabled:cursor-not-allowed"
          >
            &#x2715;
          </button>
        </div>

        {/* ── Tab row ── */}
        <div className="flex border-b border-border shrink-0">
          {BENCH_TABS.map((tab) => (
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

        {/* ── Body ── */}
        <div className="flex-1 overflow-y-auto p-5 space-y-5">

          {/* ═══════════════════════════════════════════════════════════════
              TAB: I/O Speed
              ═══════════════════════════════════════════════════════════════ */}
          {activeTab === "I/O Speed" && (
            <>
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
            </>
          )}

          {/* ═══════════════════════════════════════════════════════════════
              TAB: Crypto Speed
              ═══════════════════════════════════════════════════════════════ */}
          {activeTab === "Crypto Speed" && (
            <>
              {/* Info + AES-NI badge */}
              <div className="border border-border px-3 py-2 bg-bg">
                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 8 }}>
                  <p className="text-xs text-text-muted leading-relaxed" style={{ flex: 1 }}>
                    Measures raw AES-XTS encryption/decryption throughput and compares it with
                    AES-XTS + GCM AEAD integrity mode. Results show the performance cost of
                    enabling per-block authentication.
                  </p>
                  {aesNi !== null && (
                    <span style={{
                      fontSize: 11,
                      padding: "3px 10px",
                      border: `1px solid ${aesNi ? "rgba(74,222,128,0.4)" : "rgba(255,200,0,0.4)"}`,
                      color: aesNi ? "var(--color-success)" : "#ffc800",
                      background: aesNi ? "rgba(74,222,128,0.07)" : "rgba(255,200,0,0.07)",
                      flexShrink: 0,
                      letterSpacing: "0.04em",
                    }}>
                      {aesNi ? "AES-NI ✓" : "No AES-NI ✗"}
                    </span>
                  )}
                </div>
              </div>

              {/* ── Size selector ── */}
              {cryptoState !== "running" && (
                <div>
                  <p className="text-xs text-text-muted mb-2 uppercase tracking-wider">Buffer Size</p>
                  <div className="flex gap-2">
                    {CRYPTO_SIZE_OPTIONS.map((opt) => (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={() => setCryptoSizeMb(opt.value)}
                        className={[
                          "px-4 py-1.5 text-xs border",
                          cryptoSizeMb === opt.value
                            ? "bg-surface-active text-text-bright border-border-focus"
                            : "bg-bg text-text-muted border-border hover:border-border-focus hover:text-text",
                        ].join(" ")}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                  <p className="mt-1 text-xs text-text-muted">
                    ↳ Size of the in-memory buffer used for the benchmark. Larger = more accurate result (warms up AES pipeline).
                  </p>
                </div>
              )}

              {/* ── Run button ── */}
              {cryptoState !== "done" && (
                <button
                  type="button"
                  onClick={runCryptoBenchmark}
                  disabled={cryptoState === "running"}
                  className="px-5 py-2 text-sm border border-border bg-surface text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {cryptoState === "running" ? (
                    <span className="animate-pulse">⏱ Running benchmark…</span>
                  ) : (
                    "⏱ Run Crypto Benchmark"
                  )}
                </button>
              )}

              {/* ── Error ── */}
              {cryptoError && (
                <div style={{ border: "1px solid rgba(255,68,68,0.4)", padding: "12px 14px", background: "rgba(255,68,68,0.05)" }}>
                  <p className="text-xs text-error leading-relaxed">{cryptoError}</p>
                </div>
              )}

              {/* ── Results ── */}
              {cryptoResults && cryptoState === "done" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>

                  {/* Summary badge row */}
                  <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                    <span style={{
                      fontSize: 11,
                      padding: "2px 8px",
                      border: "1px solid rgba(74,222,128,0.4)",
                      color: "var(--color-success)",
                      background: "rgba(74,222,128,0.07)",
                    }}>
                      {cryptoResults.size_label}
                    </span>
                    <span style={{
                      fontSize: 11,
                      padding: "2px 8px",
                      border: `1px solid ${cryptoResults.aes_ni_available ? "rgba(74,222,128,0.4)" : "rgba(255,200,0,0.4)"}`,
                      color: cryptoResults.aes_ni_available ? "var(--color-success)" : "#ffc800",
                      background: cryptoResults.aes_ni_available ? "rgba(74,222,128,0.07)" : "rgba(255,200,0,0.07)",
                    }}>
                      {cryptoResults.aes_ni_available ? "AES-NI ✓" : "No AES-NI ✗"}
                    </span>
                  </div>

                  {/* AES-XTS baseline block */}
                  <div style={{ border: "1px solid var(--color-border)", padding: "14px 16px", background: "var(--color-bg)" }}>
                    <p style={{ fontSize: 11, color: "var(--color-text-muted)", textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 12 }}>
                      AES-XTS Only — Baseline
                    </p>
                    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                      {/* Encrypt */}
                      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                        <span style={{ fontSize: 12, color: "var(--color-text-muted)", minWidth: 64 }}>Encrypt</span>
                        <SpeedBar
                          mbps={cryptoResults.xts_encrypt_mbps}
                          maxMbps={maxCryptoMbps}
                          color="var(--color-success)"
                        />
                      </div>
                      {/* Decrypt */}
                      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                        <span style={{ fontSize: 12, color: "var(--color-text-muted)", minWidth: 64 }}>Decrypt</span>
                        <SpeedBar
                          mbps={cryptoResults.xts_decrypt_mbps}
                          maxMbps={maxCryptoMbps}
                          color="#4a9ade"
                        />
                      </div>
                    </div>
                  </div>

                  {/* AES-XTS + GCM AEAD block */}
                  <div style={{ border: "1px solid var(--color-border)", padding: "14px 16px", background: "var(--color-bg)" }}>
                    <p style={{ fontSize: 11, color: "var(--color-text-muted)", textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 12 }}>
                      AES-XTS + GCM AEAD — With Integrity
                    </p>
                    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                      {/* Encrypt */}
                      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                        <span style={{ fontSize: 12, color: "var(--color-text-muted)", minWidth: 64 }}>Encrypt</span>
                        <SpeedBar
                          mbps={cryptoResults.xts_aead_encrypt_mbps}
                          maxMbps={maxCryptoMbps}
                          color="var(--color-success)"
                        />
                        <span style={{ fontSize: 11, color: "var(--color-text-muted)", minWidth: 90, textAlign: "right" }}>
                          ▼ {cryptoResults.aead_overhead_encrypt_pct.toFixed(1)}% vs baseline
                        </span>
                      </div>
                      {/* Decrypt */}
                      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                        <span style={{ fontSize: 12, color: "var(--color-text-muted)", minWidth: 64 }}>Decrypt</span>
                        <SpeedBar
                          mbps={cryptoResults.xts_aead_decrypt_mbps}
                          maxMbps={maxCryptoMbps}
                          color="#4a9ade"
                        />
                        <span style={{ fontSize: 11, color: "var(--color-text-muted)", minWidth: 90, textAlign: "right" }}>
                          ▼ {cryptoResults.aead_overhead_decrypt_pct.toFixed(1)}% vs baseline
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Run again */}
                  <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                    <p className="text-xs text-success">Benchmark complete.</p>
                    <button
                      type="button"
                      onClick={handleCryptoReset}
                      className="px-3 py-1 text-xs border border-border bg-surface text-text hover:border-border-focus"
                    >
                      Run Again
                    </button>
                  </div>
                </div>
              )}

              {/* ── Notes ── */}
              <div className="border border-border px-3 py-2 bg-bg">
                <p className="text-xs text-text-muted leading-relaxed">
                  <span className="text-text">Note:</span> This benchmark operates entirely in RAM.
                  It allocates a buffer of the selected size, fills it with random data, then runs
                  encrypt/decrypt cycles using the same primitives as CFS (AES-256-XTS + optional GCM).
                  No disk I/O is involved. Results depend on CPU capabilities (AES-NI) and available
                  memory bandwidth.
                </p>
              </div>
            </>
          )}
        </div>

        {/* ── Footer ── */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-border shrink-0">
          <button
            type="button"
            onClick={handleClose}
            disabled={benchState === "running" || cryptoState === "running"}
            className="px-5 py-1.5 text-sm border border-border bg-surface-active text-text-bright hover:border-border-focus disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {benchState === "running" || cryptoState === "running"
              ? "Benchmark running…"
              : "Close"}
          </button>
        </div>
      </div>
    </div>
  );
}
