import { useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import type { SessionView } from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import "./style.css";
function App() {
  const [session, setSession] = useState<SessionView | null>(null);
  const [pending, setPending] = useState(true);
  const [error, setError] = useState("");
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
    <main id="main">
      <div className="brand">
        <span className="mark" aria-hidden="true">
          L
        </span>
        <span>
          LimeOS<small>Your home, in order.</small>
        </span>
      </div>
      <section aria-labelledby="heading">
        <p className="eyebrow">LOCAL CONTROL</p>
        <h1 id="heading">{session ? "You’re signed in." : "Welcome home."}</h1>
        <p className="intro">
          {session
            ? `Connected as ${session.principal.role.replace("_", " ")}.`
            : "Sign in to your LimeOS host."}
        </p>
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        {session ? (
          <button onClick={() => void logout()} disabled={pending}>
            Sign out
          </button>
        ) : (
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
        )}
      </section>
      <footer>LimeOS · Secure host management</footer>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
