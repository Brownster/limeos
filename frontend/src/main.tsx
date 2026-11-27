import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import type { SessionView } from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import "./ui/tokens.css";
import "./ui/primitives.css";
import "./app/shell.css";
import "./style.css";
import "./overview/overview.css";
import { Login } from "./app/Login";
import { Shell } from "./app/Shell";
import { OverviewProvider } from "./app/overview-data";
import { navigate, usePathname } from "./app/router";
import { documentTitle, parseRoute, routeById } from "./app/routes";
import { Pages } from "./pages/Pages";

function App() {
  const [session, setSession] = useState<SessionView | null>(null);
  const [pending, setPending] = useState(true);
  const [checked, setChecked] = useState(false);
  const [error, setError] = useState("");
  const pathname = usePathname();
  const route = parseRoute(pathname);
  const expired = useCallback(() => setSession(null), []);
  const reauthenticate = useCallback(() => {
    setSession(null);
    setError("Health scheduling enabled. Sign in again to use the updated permission.");
  }, []);
  useEffect(() => {
    if (route.redirect) navigate(route.path, { replace: true });
  }, [route.redirect, route.path]);
  useEffect(() => {
    document.title = session ? documentTitle(route.id) : "LimeOS · Sign in";
  }, [session, route.id]);
  // After an in-app navigation, move focus to the new page's heading so
  // keyboard and screen-reader users start at the top of the new content.
  const previous = useRef(pathname);
  useEffect(() => {
    if (previous.current === pathname) return;
    previous.current = pathname;
    window.scrollTo(0, 0);
    const frame = window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-page-title]")?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [pathname]);
  useEffect(() => {
    void api
      .session()
      .then(setSession)
      .catch((e: unknown) => {
        if (!(e instanceof ApiError && e.detail.code === "unauthenticated"))
          setError(e instanceof Error ? e.message : "LimeOS is unavailable.");
      })
      .finally(() => {
        setPending(false);
        setChecked(true);
      });
  }, []);
  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const input = new FormData(form);
    setPending(true);
    setError("");
    try {
      setSession(
        await api.login({
          username: String(input.get("username")),
          password: String(input.get("password")),
        }),
      );
      form.reset();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Sign in failed.");
    } finally {
      setPending(false);
    }
  }
  async function logout() {
    if (!session) return;
    setPending(true);
    setError("");
    try {
      await api.logout(session.csrf_token);
      setSession(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Sign out failed.");
    } finally {
      setPending(false);
    }
  }
  if (!session) {
    // Avoid flashing the sign-in form while the existing session is checked.
    if (!checked) return <main id="main" className="login" aria-busy="true"><p className="sr-only" role="status">Connecting to LimeOS…</p></main>;
    return <Login pending={pending} error={error} onSubmit={(e) => void login(e)} />;
  }
  const canManage = ["operator", "administrator"].includes(session.principal.role);
  const section = route.id === "not-found" ? null : routeById(route.id).section;
  return (
    <OverviewProvider key={session.csrf_token} expired={expired}>
      <Shell session={session} section={section} canManage={canManage} pending={pending} logout={() => void logout()}>
        {error && <p className="error app-error" role="alert">{error}</p>}
        <Pages route={route.id} session={session} expired={expired} reauthenticate={reauthenticate} />
      </Shell>
    </OverviewProvider>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
