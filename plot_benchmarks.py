import json
import os
import sys
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker

# Ensure dark background and premium aesthetic
plt.style.use('dark_background')
plt.rcParams['font.family'] = 'sans-serif'
plt.rcParams['font.sans-serif'] = ['Segoe UI', 'Arial', 'Helvetica', 'DejaVu Sans']
plt.rcParams['axes.edgecolor'] = '#333333'
plt.rcParams['axes.linewidth'] = 1.2

# Define output directories
artifact_dir = r"C:\Users\ekans\.gemini\antigravity\brain\0622f928-ff09-4349-b78c-c5923d2805da"
local_dir = r"c:\Users\ekans\Desktop\Btech\CFS\cfs-io\benchmark_graphs"
os.makedirs(artifact_dir, exist_ok=True)
os.makedirs(local_dir, exist_ok=True)

# Load JSON results
json_path = "benchmark_results.json"
if not os.path.exists(json_path):
    print(f"Error: {json_path} not found!")
    sys.exit(1)

with open(json_path, "r") as f:
    data = json.load(f)

print("Loaded benchmark data successfully.")

# Color palettes (Neon / Cyberpunk Premium Dark)
COLOR_CYAN = "#00f2fe"
COLOR_BLUE = "#4facfe"
COLOR_PINK = "#ff0844"
COLOR_ORANGE = "#ffb199"
COLOR_PURPLE = "#b224ef"
COLOR_GREEN = "#00ea8d"
COLOR_GOLD = "#f857a6"
COLOR_GRID = "#222222"

def save_fig(fig, filename):
    p1 = os.path.join(artifact_dir, filename)
    p2 = os.path.join(local_dir, filename)
    fig.savefig(p1, dpi=300, bbox_inches='tight', facecolor='#0d0d0d', edgecolor='none')
    fig.savefig(p2, dpi=300, bbox_inches='tight', facecolor='#0d0d0d', edgecolor='none')
    print(f"Saved: {p1}")
    print(f"Saved: {p2}")
    plt.close(fig)

# -----------------------------------------------------------------------------
# Graph 1: Filesystem I/O Scaling across File Sizes
# -----------------------------------------------------------------------------
io_data = data.get("io_scaling", [])
if io_data:
    labels = [d["label"] for d in io_data]
    writes = [d["write_mbps"] for d in io_data]
    reads = [d["read_mbps"] for d in io_data]
    syncs = [d["sync_ms"] for d in io_data]

    x = range(len(labels))
    width = 0.35

    fig, ax1 = plt.subplots(figsize=(11, 6), facecolor='#0d0d0d')
    ax1.set_facecolor('#141414')
    ax1.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)

    rects1 = ax1.bar([i - width/2 for i in x], writes, width, label='Write Speed (MiB/s)', color=COLOR_CYAN, alpha=0.9, zorder=3)
    rects2 = ax1.bar([i + width/2 for i in x], reads, width, label='Read Speed (MiB/s)', color=COLOR_BLUE, alpha=0.9, zorder=3)

    ax1.set_ylabel('Throughput (MiB/s)', fontsize=12, fontweight='bold', color='#ffffff')
    ax1.set_title('CFS Filesystem I/O Throughput vs. File Size', fontsize=15, fontweight='bold', pad=20, color='#ffffff')
    ax1.set_xticks(x)
    ax1.set_xticklabels(labels, fontsize=11)
    ax1.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=11)
    ax1.set_ylim(bottom=0)

    # Twin axis for Sync Latency
    ax2 = ax1.twinx()
    line = ax2.plot(x, syncs, color=COLOR_PINK, marker='o', linewidth=2.5, markersize=8, label='Sync Latency (ms)', zorder=4)
    ax2.set_ylabel('Sync Latency (ms)', fontsize=12, fontweight='bold', color=COLOR_PINK)
    ax2.tick_params(axis='y', labelcolor=COLOR_PINK)
    ax2.grid(False)
    ax2.set_ylim(bottom=0)

    # Combine legends
    lines, labels_ax = ax1.get_legend_handles_labels()
    lines2, labels2_ax = ax2.get_legend_handles_labels()
    ax1.legend(lines + lines2, labels_ax + labels2_ax, loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=10)

    save_fig(fig, "io_scaling.png")

# -----------------------------------------------------------------------------
# Graph 2A: I/O Performance Matrix across Block Sizes & File Sizes (5-Run Avg)
# -----------------------------------------------------------------------------
matrix_data = data.get("io_matrix", [])
if matrix_data:
    # Group by file_size_label: "4 KiB", "1 MiB", "16 MiB", "128 MiB"
    file_labels = []
    for d in matrix_data:
        if d["file_size_label"] not in file_labels:
            file_labels.append(d["file_size_label"])
    
    # Extract speeds for 4 KB, 16 KB, 64 KB block sizes
    writes_4k = [d["write_mbps"] for d in matrix_data if d["block_size_kb"] == 4]
    writes_16k = [d["write_mbps"] for d in matrix_data if d["block_size_kb"] == 16]
    writes_64k = [d["write_mbps"] for d in matrix_data if d["block_size_kb"] == 64]

    reads_4k = [d["read_mbps"] for d in matrix_data if d["block_size_kb"] == 4]
    reads_16k = [d["read_mbps"] for d in matrix_data if d["block_size_kb"] == 16]
    reads_64k = [d["read_mbps"] for d in matrix_data if d["block_size_kb"] == 64]

    x = range(len(file_labels))
    width = 0.25

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(18, 6.5), facecolor='#0d0d0d')
    
    # Left: Write Speeds
    ax1.set_facecolor('#141414')
    ax1.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
    r_w4  = ax1.bar([i - width for i in x], writes_4k, width, label='4 KiB Block Size', color=COLOR_CYAN, alpha=0.9, zorder=3)
    r_w16 = ax1.bar([i for i in x],         writes_16k, width, label='16 KiB Block Size', color=COLOR_GREEN, alpha=0.9, zorder=3)
    r_w64 = ax1.bar([i + width for i in x], writes_64k, width, label='64 KiB Block Size', color=COLOR_PURPLE, alpha=0.9, zorder=3)
    
    ax1.set_ylabel('Write Throughput (MiB/s)', fontsize=12, fontweight='bold', color='#ffffff')
    ax1.set_title('Write Performance (5-Run Average)', fontsize=14, fontweight='bold', pad=15, color='#ffffff')
    ax1.set_xticks(x)
    ax1.set_xticklabels(file_labels, fontsize=11)
    ax1.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=10)
    ax1.set_ylim(bottom=0)

    # Right: Read Speeds
    ax2.set_facecolor('#141414')
    ax2.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
    r_r4  = ax2.bar([i - width for i in x], reads_4k, width, label='4 KiB Block Size', color=COLOR_CYAN, alpha=0.9, zorder=3)
    r_r16 = ax2.bar([i for i in x],         reads_16k, width, label='16 KiB Block Size', color=COLOR_GREEN, alpha=0.9, zorder=3)
    r_r64 = ax2.bar([i + width for i in x], reads_64k, width, label='64 KiB Block Size', color=COLOR_PURPLE, alpha=0.9, zorder=3)
    
    ax2.set_ylabel('Read Throughput (MiB/s)', fontsize=12, fontweight='bold', color='#ffffff')
    ax2.set_title('Read Performance (5-Run Average)', fontsize=14, fontweight='bold', pad=15, color='#ffffff')
    ax2.set_xticks(x)
    ax2.set_xticklabels(file_labels, fontsize=11)
    ax2.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=10)
    ax2.set_ylim(bottom=0)

    for rects in [r_w4, r_w16, r_w64, r_r4, r_r16, r_r64]:
        for rect in rects:
            height = rect.get_height()
            if height > 0:
                ax1.annotate(f'{height:.0f}',
                             xy=(rect.get_x() + rect.get_width() / 2, height),
                             xytext=(0, 3), textcoords="offset points",
                             ha='center', va='bottom', fontsize=8, color='#e0e0e0', fontweight='bold') if rect in r_w4+r_w16+r_w64 else ax2.annotate(f'{height:.0f}',
                             xy=(rect.get_x() + rect.get_width() / 2, height),
                             xytext=(0, 3), textcoords="offset points",
                             ha='center', va='bottom', fontsize=8, color='#e0e0e0', fontweight='bold')

    fig.suptitle('I/O Performance Comparison across Block Sizes & File Sizes', fontsize=16, fontweight='bold', color='#ffffff', y=0.98)
    save_fig(fig, "io_matrix_comparison.png")

# -----------------------------------------------------------------------------
# Graph 2B: Multiple Small Files Benchmark (500 Files across Varying Block Sizes)
# -----------------------------------------------------------------------------
sf_data = data.get("small_files", [])
if sf_data:
    labels = [f"{d['block_size_kb']} KiB Block Size" for d in sf_data]
    w_fps = [d["write_fps"] for d in sf_data]
    r_fps = [d["read_fps"] for d in sf_data]
    w_ms  = [d["write_ms"] for d in sf_data]
    r_ms  = [d["read_ms"] for d in sf_data]

    x = range(len(labels))
    width = 0.35

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(16, 6), facecolor='#0d0d0d')

    # Left: Throughput (Files/sec)
    ax1.set_facecolor('#141414')
    ax1.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
    rects1 = ax1.bar([i - width/2 for i in x], w_fps, width, label='Write Speed (files/s)', color=COLOR_GREEN, alpha=0.9, zorder=3)
    rects2 = ax1.bar([i + width/2 for i in x], r_fps, width, label='Read Speed (files/s)', color=COLOR_BLUE, alpha=0.9, zorder=3)
    ax1.set_ylabel('Throughput (files/sec)', fontsize=12, fontweight='bold', color='#ffffff')
    ax1.set_title('Small Files Throughput (500 Files — 5-Run Avg)', fontsize=14, fontweight='bold', pad=15, color='#ffffff')
    ax1.set_xticks(x)
    ax1.set_xticklabels(labels, fontsize=11)
    ax1.legend(loc='upper right', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=11)
    ax1.set_ylim(bottom=0)

    # Right: Total Latency (ms)
    ax2.set_facecolor('#141414')
    ax2.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
    rects3 = ax2.bar([i - width/2 for i in x], w_ms, width, label='Write Time (ms)', color=COLOR_PINK, alpha=0.9, zorder=3)
    rects4 = ax2.bar([i + width/2 for i in x], r_ms, width, label='Read Time (ms)', color=COLOR_CYAN, alpha=0.9, zorder=3)
    ax2.set_ylabel('Total Time for 500 Files (ms)', fontsize=12, fontweight='bold', color='#ffffff')
    ax2.set_title('Small Files Latency (500 Files — 5-Run Avg)', fontsize=14, fontweight='bold', pad=15, color='#ffffff')
    ax2.set_xticks(x)
    ax2.set_xticklabels(labels, fontsize=11)
    ax2.legend(loc='upper right', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=11)
    ax2.set_ylim(bottom=0)

    for ax, rects in [(ax1, rects1+rects2), (ax2, rects3+rects4)]:
        for rect in rects:
            height = rect.get_height()
            if height > 0:
                ax.annotate(f'{height:.0f}',
                            xy=(rect.get_x() + rect.get_width() / 2, height),
                            xytext=(0, 3), textcoords="offset points",
                            ha='center', va='bottom', fontsize=9, color='#e0e0e0', fontweight='bold')

    fig.suptitle('Multi-File I/O Benchmark: 500 Small Files across Varying Block Sizes', fontsize=16, fontweight='bold', color='#ffffff', y=0.98)
    save_fig(fig, "small_files_benchmark.png")

# -----------------------------------------------------------------------------
# Graph 3: Cryptographic Engine Speed & AEAD Overhead (Multi-EU)
# -----------------------------------------------------------------------------
crypto_data = data.get("crypto_speed", [])
if crypto_data:
    # Detect new multi-EU format vs old single-AEAD format
    has_multi_eu = "aead_enc_4k_mbps" in crypto_data[0]

    sizes = [f"{d['size_mb']} MiB" for d in crypto_data]
    xts_enc = [d["xts_enc_mbps"] for d in crypto_data]
    xts_dec = [d["xts_dec_mbps"] for d in crypto_data]
    x = range(len(sizes))

    if has_multi_eu:
        aead_4k_enc  = [d["aead_enc_4k_mbps"]  for d in crypto_data]
        aead_16k_enc = [d["aead_enc_16k_mbps"] for d in crypto_data]
        aead_64k_enc = [d["aead_enc_64k_mbps"] for d in crypto_data]

        fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(18, 6.5), facecolor='#0d0d0d')

        # Left subplot: Throughput comparison across EU sizes
        ax1.set_facecolor('#141414')
        ax1.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)

        width = 0.18
        ax1.bar([i - 1.5*width for i in x], xts_enc,      width, label='XTS Encrypt (baseline)', color=COLOR_CYAN,   alpha=0.95, zorder=3)
        ax1.bar([i - 0.5*width for i in x], aead_4k_enc,  width, label='AEAD Parallel (4 KiB EU)',  color=COLOR_PINK,   alpha=0.95, zorder=3)
        ax1.bar([i + 0.5*width for i in x], aead_16k_enc, width, label='AEAD Parallel (16 KiB EU)', color=COLOR_ORANGE, alpha=0.95, zorder=3)
        ax1.bar([i + 1.5*width for i in x], aead_64k_enc, width, label='AEAD Parallel (64 KiB EU)', color=COLOR_GREEN,  alpha=0.95, zorder=3)

        ax1.set_ylabel('Throughput (MiB/s)', fontsize=12, fontweight='bold', color='#ffffff')
        ax1.set_title('AES-256-XTS vs. Optimized Parallel AEAD Tag Throughput', fontsize=13, fontweight='bold', pad=15, color='#ffffff')
        ax1.set_xticks(x)
        ax1.set_xticklabels(sizes, fontsize=10)
        ax1.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=9)
        ax1.set_ylim(bottom=0)
        # Reference line at XTS average
        avg_xts = sum(xts_enc) / len(xts_enc)
        ax1.axhline(y=avg_xts, color=COLOR_CYAN, linestyle=':', alpha=0.5, linewidth=1.5)
        ax1.text(len(sizes)-0.5, avg_xts * 1.02, f'XTS avg: {avg_xts:.0f}', color=COLOR_CYAN, fontsize=9)

        # Right subplot: n_tags count per buffer size across EU sizes — shows reduction in work
        n_tags_4k  = [d["size_mb"] * 1024 * 1024 // (4  * 1024) for d in crypto_data]
        n_tags_16k = [d["size_mb"] * 1024 * 1024 // (16 * 1024) for d in crypto_data]
        n_tags_64k = [d["size_mb"] * 1024 * 1024 // (64 * 1024) for d in crypto_data]

        ax2.set_facecolor('#141414')
        ax2.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
        ax2.plot(sizes, n_tags_4k,  'o-', color=COLOR_PINK,   linewidth=2.5, markersize=8, label='4 KiB EU tags/buf')
        ax2.plot(sizes, n_tags_16k, 's-', color=COLOR_ORANGE, linewidth=2.5, markersize=8, label='16 KiB EU tags/buf')
        ax2.plot(sizes, n_tags_64k, 'D-', color=COLOR_GREEN,  linewidth=2.5, markersize=8, label='64 KiB EU tags/buf')
        ax2.set_ylabel('Number of AEAD Tags Generated', fontsize=12, fontweight='bold', color='#ffffff')
        ax2.set_title('Tag Count Reduction with Larger Encryption Unit', fontsize=13, fontweight='bold', pad=15, color='#ffffff')
        ax2.set_xticks(range(len(sizes)))
        ax2.set_xticklabels(sizes, fontsize=10)
        ax2.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=10)
        ax2.set_yscale('log')
        ax2.yaxis.set_major_formatter(ticker.FuncFormatter(lambda v, _: f'{int(v):,}'))

    else:
        # Fallback: old format with single aead column
        aead_enc = [d.get("aead_enc_mbps", 0) for d in crypto_data]
        aead_dec = [d.get("aead_dec_mbps", 0) for d in crypto_data]
        ov_enc   = [d.get("overhead_enc_pct", 0) for d in crypto_data]
        ov_dec   = [d.get("overhead_dec_pct", 0) for d in crypto_data]

        fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(16, 6), facecolor='#0d0d0d')
        ax1.set_facecolor('#141414')
        ax1.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
        width = 0.2
        ax1.bar([i - 1.5*width for i in x], xts_enc,  width, label='XTS Encrypt',        color=COLOR_CYAN,   zorder=3)
        ax1.bar([i - 0.5*width for i in x], xts_dec,  width, label='XTS Decrypt',        color=COLOR_BLUE,   zorder=3)
        ax1.bar([i + 0.5*width for i in x], aead_enc, width, label='XTS+AEAD Encrypt',   color=COLOR_PINK,   zorder=3)
        ax1.bar([i + 1.5*width for i in x], aead_dec, width, label='XTS+AEAD Decrypt',   color=COLOR_ORANGE, zorder=3)
        ax1.set_ylabel('Throughput (MiB/s)', fontsize=12, fontweight='bold', color='#ffffff')
        ax1.set_title('AES-256-XTS vs. AEAD Engine Throughput', fontsize=13, fontweight='bold', pad=15, color='#ffffff')
        ax1.set_xticks(x); ax1.set_xticklabels(sizes, fontsize=10)
        ax1.legend(loc='upper left', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=9)
        ax1.set_ylim(bottom=0)
        ax2.set_facecolor('#141414')
        ax2.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)
        width_ov = 0.35
        rects_oe = ax2.bar([i - width_ov/2 for i in x], ov_enc, width_ov, label='Encrypt Overhead (%)', color=COLOR_GOLD,   zorder=3)
        rects_od = ax2.bar([i + width_ov/2 for i in x], ov_dec, width_ov, label='Decrypt Overhead (%)', color=COLOR_PURPLE, zorder=3)
        ax2.set_ylabel('Performance Overhead (%)', fontsize=12, fontweight='bold', color='#ffffff')
        ax2.set_title('AEAD Tag Computation Overhead', fontsize=13, fontweight='bold', pad=15, color='#ffffff')
        ax2.set_xticks(x); ax2.set_xticklabels(sizes, fontsize=10)
        ax2.legend(loc='upper right', frameon=True, facecolor='#1f1f1f', edgecolor='#333333', fontsize=10)
        ax2.set_ylim(bottom=0, top=max(max(ov_enc, default=0), max(ov_dec, default=0)) * 1.25 + 5)
        for rect in rects_oe + rects_od:
            h = rect.get_height()
            if h > 0:
                ax2.annotate(f'{h:.1f}%', xy=(rect.get_x() + rect.get_width() / 2, h),
                             xytext=(0, 3), textcoords="offset points",
                             ha='center', va='bottom', fontsize=9, color='#e0e0e0', fontweight='bold')

    plt.tight_layout()
    save_fig(fig, "crypto_speed.png")


# -----------------------------------------------------------------------------
# Graph 4: KDF Derivation / Unlock Latency
# -----------------------------------------------------------------------------
kdf_data = data.get("kdf_unlock", [])
if kdf_data:
    labels = [d["algo"] for d in kdf_data]
    times = [d["time_ms"] for d in kdf_data]

    y = range(len(labels))

    fig, ax = plt.subplots(figsize=(10, 6), facecolor='#0d0d0d')
    ax.set_facecolor('#141414')
    ax.grid(True, linestyle='--', alpha=0.2, color='#555555', zorder=0)

    # Reverse order for horizontal bars so first is at top
    labels_rev = list(reversed(labels))
    times_rev = list(reversed(times))
    y_rev = range(len(labels_rev))

    colors = [COLOR_CYAN if 'Argon2id' in l else (COLOR_GREEN if 'SHA256' in l else COLOR_GOLD) for l in labels_rev]

    rects = ax.barh(y_rev, times_rev, color=colors, alpha=0.9, height=0.6, zorder=3)

    ax.set_xlabel('Derivation / Unlock Latency (ms)', fontsize=12, fontweight='bold', color='#ffffff')
    ax.set_title('CFS Volume Unlock Latency: Key Derivation Functions', fontsize=15, fontweight='bold', pad=20, color='#ffffff')
    ax.set_yticks(y_rev)
    ax.set_yticklabels(labels_rev, fontsize=11)
    ax.set_xlim(left=0, right=max(times, default=100) * 1.15)

    for rect in rects:
        w = rect.get_width()
        if w > 0:
            ax.annotate(f'{w:.1f} ms',
                        xy=(w, rect.get_y() + rect.get_height() / 2),
                        xytext=(5, 0), textcoords="offset points",
                        ha='left', va='center', fontsize=10, color='#e0e0e0', fontweight='bold')

    save_fig(fig, "kdf_latency.png")

print("All benchmark graphs generated and saved successfully!")
