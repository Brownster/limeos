/**
 * The route table and path parsing. Pure (no DOM, no React) so it is unit
 * tested directly. Caddy serves index.html for every non-file path
 * (`try_files {path} /index.html`), so each of these paths survives a reload.
 */
export type RouteId =
  | "overview" | "containers" | "stacks" | "storage"
  | "alerts" | "alerts-schedules" | "alerts-policy" | "not-found";

export type NavGroup = "Main" | "Apps" | "System";

export interface RouteDef {
  id: Exclude<RouteId, "not-found">;
  path: string;
  /** Document title suffix and page heading. */
  title: string;
  /** Present when the route has its own sidebar entry. */
  nav?: { label: string; group: NavGroup; icon: "overview" | "containers" | "stacks" | "storage" | "alerts" };
  /** The sidebar entry this route highlights (sub-pages highlight their parent). */
  section: "overview" | "containers" | "stacks" | "storage" | "alerts";
  /** Only operators and administrators can use this page (it edits). */
  manageOnly?: boolean;
}

export const routes: readonly RouteDef[] = [
  { id: "overview", path: "/", title: "Overview", section: "overview", nav: { label: "Overview", group: "Main", icon: "overview" } },
  { id: "containers", path: "/containers", title: "Containers", section: "containers", nav: { label: "Containers", group: "Apps", icon: "containers" } },
  { id: "stacks", path: "/stacks", title: "Stacks", section: "stacks", nav: { label: "Stacks", group: "Apps", icon: "stacks" } },
  { id: "storage", path: "/storage", title: "Storage", section: "storage", nav: { label: "Storage", group: "System", icon: "storage" } },
  { id: "alerts", path: "/alerts", title: "Alerts", section: "alerts", nav: { label: "Alerts", group: "System", icon: "alerts" } },
  { id: "alerts-schedules", path: "/alerts/schedules", title: "Health schedules", section: "alerts", manageOnly: true },
  { id: "alerts-policy", path: "/alerts/policy", title: "Alert policy", section: "alerts", manageOnly: true },
];

export const navGroups: readonly NavGroup[] = ["Main", "Apps", "System"];

/**
 * Old and natural spellings that land on a real page, including the previous
 * app's paths so the household's bookmarks keep working.
 */
const aliases: Record<string, string> = {
  "/overview": "/", "/home": "/", "/index.html": "/", "/dashboard": "/", "/system": "/",
  "/disks": "/storage", "/pools": "/storage", "/mounts": "/storage",
  "/alerts/history": "/alerts", "/notifications": "/alerts",
  "/schedules": "/alerts/schedules", "/policy": "/alerts/policy",
};

export interface ParsedRoute {
  /** The matched route, or "not-found". */
  id: RouteId;
  /** The canonical path for this location. */
  path: string;
  /** Set when the address bar should be replaced with `path`. */
  redirect: boolean;
}

/** Normalise and match a pathname: trailing and repeated slashes, case and aliases. */
export function parseRoute(pathname: string): ParsedRoute {
  let path = (pathname || "/").split(/[?#]/, 1)[0].replace(/\/{2,}/g, "/").toLowerCase();
  if (!path.startsWith("/")) path = `/${path}`;
  if (path.length > 1) path = path.replace(/\/+$/, "") || "/";
  const canonical = aliases[path] ?? path;
  const route = routes.find((r) => r.path === canonical);
  if (!route) return { id: "not-found", path: pathname, redirect: false };
  return { id: route.id, path: canonical, redirect: canonical !== pathname };
}

export function routeById(id: Exclude<RouteId, "not-found">): RouteDef {
  return routes.find((r) => r.id === id)!;
}

/** Sidebar entries visible to a role. */
export function navRoutes(canManage: boolean): RouteDef[] {
  return routes.filter((r) => r.nav && (canManage || !r.manageOnly));
}

export function documentTitle(id: RouteId): string {
  return `LimeOS · ${id === "not-found" ? "Page not found" : routeById(id).title}`;
}
