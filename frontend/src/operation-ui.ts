import type { KeyboardEvent } from "react";
import { ApiError } from "./api";
import { wrapTarget } from "./focus-ring";

export function message(error: unknown): string {
  return error instanceof ApiError
    ? `${error.message} · reference ${error.detail.audit_id}`
    : "The host is unavailable. Check recent operations before trying again.";
}
/** Keep keyboard focus inside a modal operation dialog. */
export function containFocus(event: KeyboardEvent<HTMLDialogElement>) {
  if (event.key !== "Tab") return;
  const controls = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      "button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex='0']",
    ),
  ).filter((element) => element.getClientRects().length > 0);
  const first = controls[0],
    last = controls.at(-1);
  if (!first || !last) {
    event.preventDefault();
    event.currentTarget.focus();
    return;
  }
  const target = wrapTarget(controls, document.activeElement, event.shiftKey);
  if (target) {
    event.preventDefault();
    target.focus();
  }
}
