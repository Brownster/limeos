/**
 * The signed-in frame: a fixed sidebar from 980 px, below that a sticky top
 * bar whose menu opens the same navigation in a modal drawer (native
 * <dialog>: inert background, Escape closes, focus contained and returned).
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { SessionView } from "../../../contracts/generated/types";
import { containFocus } from "../operation-ui";
import { Icon } from "../ui/icons";
import { Link } from "./router";
import { navGroups, navRoutes, type RouteDef } from "./routes";

const roleLabel = (role: string) => role.replace("_", " ");

function Brand() {
  return (
    <span className="brand">
      <span className="mark" aria-hidden="true">L</span>
      <span className="brand-name">lime<span>os</span></span>
    </span>
  );
}

function Navigation({ section, canManage, onNavigate }: { section: RouteDef["section"] | null; canManage: boolean; onNavigate?: () => void }) {
  const items = navRoutes(canManage);
  return (
    <nav className="primary-nav" aria-label="Primary">
      {navGroups.map((group) => {
        const entries = items.filter((r) => r.nav!.group === group);
        if (entries.length === 0) return null;
        return (
          <div className="nav-group" key={group}>
            <p className="nav-group-label" aria-hidden="true">{group}</p>
            <ul>
              {entries.map((r) => (
                <li key={r.id}>
                  <Link className="nav-link" to={r.path} current={section === r.section} onClick={onNavigate}>
                    <Icon name={r.nav!.icon} />
                    <span>{r.nav!.label}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </nav>
  );
}

function Account({ session, pending, logout }: { session: SessionView; pending: boolean; logout: () => void }) {
  const name = session.principal.id;
  return (
    <div className="account">
      <span className="avatar" aria-hidden="true">{(name || "?").charAt(0).toUpperCase()}</span>
      <div className="account-who">
        <p className="account-name" title={name}>{name}</p>
        <p className="account-role">{roleLabel(session.principal.role)}</p>
      </div>
      <button type="button" className="sign-out" onClick={logout} disabled={pending}>
        <Icon name="logout" size={16} />Sign out
      </button>
    </div>
  );
}

export function Shell({ session, section, canManage, pending, logout, children }: {
  session: SessionView; section: RouteDef["section"] | null; canManage: boolean; pending: boolean; logout: () => void; children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const drawer = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = drawer.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  useEffect(() => {
    // Leaving the small-screen layout closes the drawer.
    const desktop = window.matchMedia("(min-width: 980px)");
    const change = (event: MediaQueryListEvent) => { if (event.matches) setOpen(false); };
    desktop.addEventListener("change", change);
    return () => desktop.removeEventListener("change", change);
  }, []);

  const sidebar = (inDrawer: boolean) => (
    <div className="sidebar-inner">
      <div className="sidebar-head">
        <Brand />
        {inDrawer && (
          <button type="button" className="ghost drawer-close" onClick={() => setOpen(false)} aria-label="Close navigation" autoFocus>
            <Icon name="close" size={20} />
          </button>
        )}
      </div>
      <Navigation section={section} canManage={canManage} onNavigate={inDrawer ? () => setOpen(false) : undefined} />
      <Account session={session} pending={pending} logout={logout} />
    </div>
  );

  return (
    <div className="app">
      <a className="skip-link" href="#content">Skip to content</a>
      <aside className="sidebar" aria-label="Sidebar">{sidebar(false)}</aside>
      <header className="topbar">
        <Brand />
        <button type="button" className="quiet menu-button" aria-label="Open navigation" aria-haspopup="dialog"
          aria-expanded={open} aria-controls="navigation-drawer" onClick={() => setOpen(true)}>
          <Icon name="menu" size={20} />
        </button>
      </header>
      <dialog id="navigation-drawer" ref={drawer} className="drawer" aria-label="Navigation"
        onKeyDown={containFocus} onClose={() => setOpen(false)}
        onClick={(event) => { if (event.target === event.currentTarget) setOpen(false); }}>
        {open && sidebar(true)}
      </dialog>
      <main id="content" className="main" tabIndex={-1}>
        <div className="main-inner">{children}</div>
      </main>
    </div>
  );
}
