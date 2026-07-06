// Automated test suite for NumberStepper interval snapping and stepping logic
import assert from "node:assert";

/**
 * Snaps a raw number or string input to the closest valid interval multiple within [min, max].
 */
export function snapToInterval(val, min, max, step, fallback = min) {
  if (val === undefined || val === null || String(val).trim() === "" || isNaN(Number(val))) {
    return fallback;
  }
  let num = Number(val);
  // Clamp first
  num = Math.max(min, Math.min(max, num));
  // Snap to interval from min
  let snapped = Math.round((num - min) / step) * step + min;
  // Re-clamp in case of rounding overshoot
  snapped = Math.max(min, Math.min(max, snapped));
  // Clean up floating point precision issues
  return Number(snapped.toFixed(6));
}

/**
 * Steps the current value by delta (e.g. +step or -step), snapping and clamping the result.
 */
export function stepValue(current, delta, min, max, step, fallback = min) {
  const base = snapToInterval(current, min, max, step, fallback);
  const next = base + delta;
  return snapToInterval(next, min, max, step, fallback);
}

// ─── Test Suite ─────────────────────────────────────────────────────────────

console.log("Running NumberStepper math tests...");

// Test 1: Empty or NaN input falls back to fallback
assert.strictEqual(snapToInterval("", 16, 256, 8, 32), 32, "Empty string should return fallback");
assert.strictEqual(snapToInterval("abc", 16, 256, 8, 16), 16, "NaN should return fallback");
assert.strictEqual(snapToInterval(null, 1, 12, 1, 4), 4, "Null should return fallback");

// Test 2: Clamping below min and above max
assert.strictEqual(snapToInterval(-5, 16, 256, 8), 16, "Below min should clamp to min");
assert.strictEqual(snapToInterval(0, 1, 12, 1), 1, "0 should clamp to min 1");
assert.strictEqual(snapToInterval(500, 16, 256, 8), 256, "Above max should clamp to max");
assert.strictEqual(snapToInterval(100, 1, 12, 1), 12, "Above max should clamp to max 12");

// Test 3: Interval snapping (closest multiple)
// Memory: min=16, max=256, step=8
assert.strictEqual(snapToInterval(30, 16, 256, 8), 32, "30 should snap to 32 (closer to 32 than 24)");
assert.strictEqual(snapToInterval(27, 16, 256, 8), 24, "27 should snap to 24 (closer to 24 than 32)");
assert.strictEqual(snapToInterval(20, 16, 256, 8), 24, "20 should snap to 24 (midway rounds up)");
assert.strictEqual(snapToInterval(64, 16, 256, 8), 64, "Exact multiple 64 should stay 64");

// Parallelism: min=1, max=12, step=1
assert.strictEqual(snapToInterval(5.4, 1, 12, 1), 5, "5.4 should snap to 5");
assert.strictEqual(snapToInterval(5.8, 1, 12, 1), 6, "5.8 should snap to 6");

// PBKDF2 Iterations: min=100_000, max=10_000_000, step=50_000
assert.strictEqual(snapToInterval(120_000, 100_000, 10_000_000, 50_000), 100_000, "120k snaps to 100k");
assert.strictEqual(snapToInterval(130_000, 100_000, 10_000_000, 50_000), 150_000, "130k snaps to 150k");
assert.strictEqual(snapToInterval(600_000, 100_000, 10_000_000, 50_000), 600_000, "600k stays 600k");

// Inode ratio: min=1024, max=65536, step=1024
assert.strictEqual(snapToInterval(1500, 1024, 65536, 1024), 1024, "1500 snaps to 1024");
assert.strictEqual(snapToInterval(1600, 1024, 65536, 1024), 2048, "1600 snaps to 2048");

// Test 4: Stepping (increment / decrement)
assert.strictEqual(stepValue(32, 8, 16, 256, 8), 40, "32 + 8 = 40");
assert.strictEqual(stepValue(16, -8, 16, 256, 8), 16, "16 - 8 clamps at min 16");
assert.strictEqual(stepValue(256, 8, 16, 256, 8), 256, "256 + 8 clamps at max 256");
assert.strictEqual(stepValue(6, 1, 1, 12, 1), 7, "6 + 1 = 7");
assert.strictEqual(stepValue(1, -1, 1, 12, 1), 1, "1 - 1 clamps at 1");

// Test 5: Shift stepping (10x step)
assert.strictEqual(stepValue(32, 80, 16, 256, 8), 112, "32 + 80 = 112");
assert.strictEqual(stepValue(200, 80, 16, 256, 8), 256, "200 + 80 clamps at 256");

// Test 6: Whitespace string handling
assert.strictEqual(snapToInterval("   ", 16, 256, 8, 32), 32, "Whitespace string should return fallback");

// Test 7: Floating-point precision (decimal steps)
assert.strictEqual(snapToInterval(0.1 + 0.2, 0, 1, 0.1), 0.3, "Decimal step snapping should avoid floating point precision artifacts");

// Test 8: Non-divisible range upper bound
assert.strictEqual(snapToInterval(10, 0, 10, 3), 9, "Should snap to closest valid multiple below max when range is non-divisible");
assert.strictEqual(stepValue(9, 3, 0, 10, 3), 9, "Stepping above max on non-divisible range should clamp and snap cleanly");

console.log("✔ All 26 NumberStepper math tests passed successfully!");
