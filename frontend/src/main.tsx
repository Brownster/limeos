import { useCallback, useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import type { SessionView } from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import "./style.css";
import { Dashboard } from "./Dashboard";
function App() {
  const [session, setSession] = useState<SessionView | null>(null);
  const [pending, setPending] = useState(true);
  const [error, setError] = useState("");
  const expired = useCallback(() => setSession(null), []);
  useEffect(() => {
    document.title = session ? "LimeOS · Observations" : "LimeOS · Sign in";
  }, [session]);
  useEffect(() => {
    void api
      .session()
      .then(setSession)
      .catch((e: unknown) => {
        if (!(e instanceof ApiError && e.detail.code === "unauthenticated"))
          setError(e instanceof Error ? e.message : "LimeOS is unavailable.");
      })
      .finally(() => setPending(false));
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
  return (
    <main id="main" className={session ? "signed-in" : ""}>
      <a className="skip-link" href="#content">
        Skip to content
      </a>
      <div className="brand">
        <span className="mark" aria-hidden="true">
          L
        </span>
        <span>
          LimeOS<small>Your home, in order.</small>
        </span>
      </div>
      {session ? (
        <>
          <div className="account">
            <span>
              {session.principal.id} ·{" "}
              {session.principal.role.replace("_", " ")}
            </span>
            <button onClick={() => void logout()} disabled={pending}>
              Sign out
            </button>
          </div>
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}
          <div id="content" tabIndex={-1}>
            <Dashboard expired={expired} session={session} />
          </div>
        </>
      ) : (
        <section id="content" aria-labelledby="heading">
          <p className="eyebrow">LOCAL CONTROL</p>
          <h1 id="heading">Welcome home.</h1>
          <p className="intro">Sign in to your LimeOS host.</p>
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}
          <form onSubmit={(e) => void login(e)} aria-busy={pending}>
            <label htmlFor="username">Username</label>
            <input
              id="username"
              name="username"
              autoComplete="username"
              required
              maxLength={64}
              disabled={pending}
            />
            <label htmlFor="password">Password</label>
            <input
              id="password"
              name="password"
              type="password"
              autoComplete="current-password"
              required
              maxLength={1024}
              disabled={pending}
            />
            <button disabled={pending} type="submit">
              {pending ? "Connecting…" : "Sign in"}
              <span aria-hidden="true">↗</span>
            </button>
          </form>
        </section>
      )}
      <footer>LimeOS · Secure host management</footer>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
