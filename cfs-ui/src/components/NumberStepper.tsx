import React, { useState, useEffect } from "react";

export interface NumberStepperProps {
  value: number;
  onChange: (val: number) => void;
  min: number;
  max: number;
  step: number;
  unit?: string;
  disabled?: boolean;
  className?: string;
}

/**
 * Reusable NumberStepper control replacing browser-default number inputs.
 * Features:
 * - Keyboard arrow stepping (Left/Down to decrement, Right/Up to increment, Shift for 10x jumps)
 * - Flanking decrement/increment buttons
 * - Free-form text entry allowing temporary clearing ("") while typing
 * - Automatic snapping to closest interval multiple within [min, max] on blur or Enter
 */
export const NumberStepper: React.FC<NumberStepperProps> = ({
  value,
  onChange,
  min,
  max,
  step,
  unit,
  disabled = false,
  className = "",
}) => {
  const [text, setText] = useState<string>(String(value));

  // Sync external value updates (e.g. preset button clicks) with local text state
  useEffect(() => {
    setText(String(value));
  }, [value]);

  function snap(val: string | number): number {
    if (val === undefined || val === null || String(val).trim() === "" || isNaN(Number(val))) {
      return value; // fallback to current prop value if left completely blank or invalid
    }
    let num = Number(val);
    num = Math.max(min, Math.min(max, num));
    let snapped = Math.round((num - min) / step) * step + min;
    snapped = Math.max(min, Math.min(max, snapped));
    return Number(snapped.toFixed(6));
  }

  function handleBlur() {
    const snapped = snap(text);
    setText(String(snapped));
    if (snapped !== value) {
      onChange(snapped);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (disabled) return;
    if (e.key === "Enter") {
      e.currentTarget.blur();
    } else if (e.key === "ArrowUp" || e.key === "ArrowRight") {
      e.preventDefault();
      const delta = e.shiftKey ? step * 10 : step;
      const next = snap(snap(text) + delta);
      setText(String(next));
      onChange(next);
    } else if (e.key === "ArrowDown" || e.key === "ArrowLeft") {
      e.preventDefault();
      const delta = e.shiftKey ? step * 10 : step;
      const next = snap(snap(text) - delta);
      setText(String(next));
      onChange(next);
    }
  }

  return (
    <div
      className={[
        "flex items-center border border-border bg-bg focus-within:border-border-focus transition-colors",
        disabled ? "opacity-50 pointer-events-none" : "",
        className,
      ].join(" ")}
    >
      <button
        type="button"
        disabled={disabled || value <= min}
        onClick={() => {
          const next = snap(snap(text) - step);
          setText(String(next));
          onChange(next);
        }}
        className="px-3 py-1.5 text-text-muted hover:text-text hover:bg-surface disabled:opacity-20 disabled:hover:bg-transparent border-r border-border transition-colors select-none font-mono text-sm leading-none flex items-center justify-center cursor-pointer"
        title={`Decrease by ${step}`}
      >
        −
      </button>
      <div className="flex-1 flex items-center px-2 py-1 min-w-0">
        <input
          type="text"
          disabled={disabled}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={handleBlur}
          onKeyDown={handleKeyDown}
          className="w-full bg-transparent text-text text-sm font-mono focus:outline-none min-w-0 text-center"
          spellCheck={false}
        />
        {unit && (
          <span className="text-[11px] text-text-muted ml-1 select-none font-sans shrink-0">
            {unit}
          </span>
        )}
      </div>
      <button
        type="button"
        disabled={disabled || value >= max}
        onClick={() => {
          const next = snap(snap(text) + step);
          setText(String(next));
          onChange(next);
        }}
        className="px-3 py-1.5 text-text-muted hover:text-text hover:bg-surface disabled:opacity-20 disabled:hover:bg-transparent border-l border-border transition-colors select-none font-mono text-sm leading-none flex items-center justify-center cursor-pointer"
        title={`Increase by ${step}`}
      >
        +
      </button>
    </div>
  );
};
