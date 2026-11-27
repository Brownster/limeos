import { Link, usePathname } from "../app/router";
import { navRoutes } from "../app/routes";
import { PageHeader, StateMessage } from "../ui/primitives";

export function NotFoundPage() {
  const path = usePathname();
  return (
    <>
      <PageHeader title="Page not found" />
      <StateMessage kind="empty" title={`There's no page at ${path.slice(0, 120)}`}>
        <p>It may be a bookmark from the previous app. Choose where to go:</p>
        <ul className="link-list">
          {navRoutes(false).map((r) => <li key={r.id}><Link to={r.path}>{r.nav!.label}</Link></li>)}
        </ul>
      </StateMessage>
    </>
  );
}
