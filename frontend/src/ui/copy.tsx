/**
 * A value people may need to quote (an ID, a digest, a path) with a copy
 * button. The value stays selectable text; the button appears only where the
 * browser offers the clipboard (a secure context), and says "Copied" to
 * screen readers.
 */
import { useEffect, useState } from "react";
import { Icon } from "./icons";

export function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);
  const [available, setAvailable] = useState(false);
  useEffect(() => setAvailable(typeof navigator !== "undefined" && typeof navigator.clipboard?.writeText === "function"), []);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timer);
  }, [copied]);
  if (!available) return null;
  return (
    <>
      <button type="button" className="ghost icon-button copy-button" aria-label={`Copy ${label}`} title={`Copy ${label}`}
        onClick={() => { navigator.clipboard.writeText(value).then(() => setCopied(true), () => setCopied(false)); }}>
        <Icon name={copied ? "check" : "copy"} size={14} />
      </button>
      <span className="sr-only" role="status">{copied ? `${label} copied` : ""}</span>
    </>
  );
}

/** `<code>value</code>` plus its copy button. */
export function CopyValue({ value, label }: { value: string; label: string }) {
  return (
    <span className="copy-value">
      <code>{value}</code>
      <CopyButton value={value} label={label} />
    </span>
  );
}
