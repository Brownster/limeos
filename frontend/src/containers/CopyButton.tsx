/**
 * Copies a value to the clipboard and says so. The Clipboard API needs a
 * secure context (the host serves HTTPS); where it is refused, the value is
 * selected instead so the user can copy it by hand.
 */
import { useEffect, useRef, useState } from "react";
import { CIcon } from "./icons";

export function CopyButton({ value, label, target, compact = false }: {
  value: string;
  /** What is copied, for the accessible name: "Copy container ID". */
  label: string;
  /** An element holding the value, selected when copying is refused. */
  target?: () => HTMLElement | null;
  compact?: boolean;
}) {
  const [state, setState] = useState<"idle" | "copied" | "manual">("idle");
  const timer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timer.current), []);
  async function copy() {
    clearTimeout(timer.current);
    try {
      if (!navigator.clipboard || !window.isSecureContext) throw new Error("no clipboard");
      await navigator.clipboard.writeText(value);
      setState("copied");
    } catch {
      const element = target?.();
      if (element) {
        const range = document.createRange();
        range.selectNodeContents(element);
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
      }
      setState("manual");
    }
    timer.current = setTimeout(() => setState("idle"), 4000);
  }
  return (
    <span className="copy">
      <button type="button" className={compact ? "quiet icon-button copy-button" : "quiet copy-button"} onClick={() => void copy()} aria-label={label} title={label}>
        <CIcon name={state === "copied" ? "check" : "copy"} />
        {!compact && <span>{state === "copied" ? "Copied" : "Copy"}</span>}
      </button>
      <span className="copy-status" role="status">
        {state === "copied" ? "Copied" : state === "manual" ? "Selected: press Ctrl+C to copy" : ""}
      </span>
    </span>
  );
}
