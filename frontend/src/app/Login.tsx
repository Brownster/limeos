import type { FormEvent } from "react";

export function Login({ pending, error, onSubmit }: { pending: boolean; error: string; onSubmit: (event: FormEvent<HTMLFormElement>) => void }) {
  return (
    <main id="main" className="login">
      <a className="skip-link" href="#content">Skip to content</a>
      <div className="login-card">
        <div className="brand">
          <span className="mark" aria-hidden="true">L</span>
          <span className="brand-name">lime<span>os</span><small>Your home, in order.</small></span>
        </div>
        <section id="content" className="card" aria-labelledby="heading" tabIndex={-1}>
          <p className="eyebrow">LOCAL CONTROL</p>
          <h1 id="heading">Welcome home.</h1>
          <p className="intro">Sign in to your LimeOS host.</p>
          {error && <p className="error" role="alert">{error}</p>}
          <form className="login-form" onSubmit={onSubmit} aria-busy={pending}>
            <label htmlFor="username">Username</label>
            <input id="username" name="username" autoComplete="username" required maxLength={64} disabled={pending} autoCapitalize="none" spellCheck={false} />
            <label htmlFor="password">Password</label>
            <input id="password" name="password" type="password" autoComplete="current-password" required maxLength={1024} disabled={pending} />
            <button disabled={pending} type="submit">
              {pending ? "Connecting…" : "Sign in"}
              <span aria-hidden="true">↗</span>
            </button>
          </form>
        </section>
        <footer>LimeOS · Secure host management</footer>
      </div>
    </main>
  );
}
