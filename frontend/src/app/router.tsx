/**
 * A minimal history-API router: one source of truth (location.pathname), a
 * hook to read it and a Link that navigates without reloading. Route matching
 * lives in routes.ts. Modified clicks (new tab, etc.) keep browser behaviour.
 */
import { useSyncExternalStore, type AnchorHTMLAttributes, type MouseEvent } from "react";

const listeners = new Set<() => void>();
const notify = () => listeners.forEach((listener) => listener());

function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener("popstate", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("popstate", listener);
  };
}

export function usePathname(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname, () => "/");
}

export function navigate(to: string, { replace = false } = {}) {
  if (to === window.location.pathname) return;
  if (replace) window.history.replaceState(null, "", to);
  else window.history.pushState(null, "", to);
  notify();
}

export function Link({ to, current = false, onClick, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { to: string; current?: boolean }) {
  const follow = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || props.target === "_blank") return;
    event.preventDefault();
    navigate(to);
  };
  return <a href={to} aria-current={current ? "page" : undefined} onClick={follow} {...props} />;
}
