import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type { Counts, FileStatus, RuntimeStatus, ServiceView, StackInventory } from "./stack-inventory-client";
import { StackInventoryController } from "./stack-inventory-controller";

const runtimeLabels: Record<RuntimeStatus, string> = { unknown: "Unknown", no_containers: "No service containers", stopped: "Stopped", running: "Running", partial: "Partial", conflict: "Identity conflict" };
const relationLabels = { not_registered: "Not registered", catalog_unknown: "Registration unknown", engine_mismatch: "Different Engine", identity_conflict: "Registration identity conflict", overlapping: "Overlaps a registered stack" };
const sample = (at: number | null) => at === null ? "No sample" : new Date(at * 1000).toISOString();

export function StackManagement({ value, loading = false, stale = false, error = "", refresh }: {
  value: StackInventory | null; loading?: boolean; stale?: boolean; error?: string; refresh: () => void;
}) {
  return <section aria-labelledby="stack-management-heading" aria-busy={loading}>
    <p className="eyebrow">DEPLOYMENT</p><h2 id="stack-management-heading">Stacks and services</h2>
    <p>Registered stacks stay visible when their containers are stopped or have not been created.</p>
    <button onClick={refresh} disabled={loading}>Refresh stacks</button>
    {loading && <p role="status">Reading stack inventory…</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {stale && value && <p className="notice" role="status">This is a previous or incomplete observation. Current runtime counts are unavailable.</p>}
    {!value && !loading && <p>Stack inventory is unavailable.</p>}
    {value && <>
      <div className="source-list" aria-label="Stack observation sources">
        <p>Catalog · {value.catalog.state} · {sample(value.catalog.sampled_at)}</p>
        <p>Engine · {value.engine.state} · {sample(value.engine.sampled_at)}</p>
        {value.engine_matches_catalog === false && <p className="notice" role="status">The Engine identity differs from the registration. Runtime counts cannot be matched to these stacks.</p>}
      </div>
      <p className="notice">Stack actions are unavailable while deployment execution is being integrated. Container membership does not verify Compose source files or storage dependencies.</p>
      {value.stacks.length === 0 && <p>{value.catalog.state === "fresh" ? "No stacks are registered." : "Registered stacks could not be confirmed."}</p>}
      {value.stacks.map(stack => <article key={stack.stack} aria-label={`Stack ${stack.stack}`}>
        <h3>{stack.stack} <span className={`badge ${stale || stack.runtime !== "running" ? "warning" : ""}`}>{stale ? "Unknown" : runtimeLabels[stack.runtime]}</span></h3>
        <p>Project · {stack.project}<br /><code style={{ overflowWrap: "anywhere" }}>{stack.working_directory}</code></p>
        <CountsView counts={stale ? null : stack.counts} />
        {!stale && stack.missing_services !== null && stack.missing_services > 0 && <p className="notice">{stack.missing_services} declared {stack.missing_services === 1 ? "service has" : "services have"} no ordinary container.</p>}
        <FileDiagnostic value={stack.files} />
        {stack.issues.includes("identity_conflict") && <p className="error">Engine project labels differ from the registered directory or ordered Compose files.</p>}
        {stack.issues.includes("overlapping_project") && <p className="error">Another Engine project overlaps this stack. Membership counts are withheld.</p>}
        {stack.issues.includes("undeclared_services") && <p className="notice">Additional services are absent from the registration and are shown below.</p>}
        <details><summary>Services and members · {stack.services.length}</summary>
          <p>Expected configuration files</p><ul>{stack.config_files.map(file => <li key={file}><code style={{ overflowWrap: "anywhere" }}>{file}</code></li>)}</ul>
          <Services value={stack.services} stale={stale} />
        </details>
      </article>)}
      {value.other_projects.length > 0 && <><h3>Other observed projects</h3><p>Engine labels identify these projects. They have not been adopted as managed stacks.</p>
        {value.other_projects.map(row => <details key={row.project.name}><summary>{row.project.name} · {relationLabels[row.relation]}</summary>
          <p><code style={{ overflowWrap: "anywhere" }}>{row.project.working_directory}</code></p>
          <ul>{row.project.config_files.map(file => <li key={file}><code style={{ overflowWrap: "anywhere" }}>{file}</code></li>)}</ul>
          <Services value={row.services} stale={stale} />
        </details>)}</>}
      <p>{stale || value.standalone_containers === null ? "Current standalone-container count is unavailable." : `${value.standalone_containers} standalone containers observed outside Compose projects.`}</p>
    </>}
  </section>;
}

function CountsView({ counts }: { counts: Counts | null }) {
  return <p>{counts ? `${counts.running} of ${counts.ordinary} service containers running · ${counts.one_offs} one-offs (${counts.one_off_running} running)` : "Current container counts are unknown."}</p>;
}
function FileDiagnostic({ value }: { value: FileStatus }) {
  if (value.state === "conflict") return <p className="error" role="status">Multiple Compose files: {value.candidates.join(", ")}. Resolve the conflict before edits or actions.</p>;
  if (value.state === "unsupported") return <p className="notice">Source interpretation is unsupported ({value.reason}). The source is preserved for review.</p>;
  if (value.state === "unavailable") return <p className="notice">Compose source inspection is unavailable.</p>;
  return <p>Compose sources have not been inspected.</p>;
}
function Services({ value, stale }: { value: ServiceView[]; stale: boolean }) {
  return <>{value.map(service => <div key={service.name}><h4>{service.name} {!service.declared && <span className="badge warning">Not in registration</span>}</h4>
    <p>{stale ? "Previous observation" : runtimeLabels[service.runtime]}</p><CountsView counts={stale ? null : service.counts} />
    {service.members.length > 0 && <div className="table-wrap"><table><caption>{service.name} · {stale ? "previously observed members" : "observed members"}</caption>
      <thead><tr><th scope="col">Container</th><th scope="col">Role</th><th scope="col">Observed state</th><th scope="col">Image and start time</th></tr></thead>
      <tbody>{service.members.map(member => <tr key={member.container.resource}><th scope="row"><code>{member.container.resource.slice(-12)}</code><small>{member.container.resource}</small></th>
        <td>{member.one_off ? "One-off" : "Service / replica"}</td><td>{stale ? "Previously " : ""}{member.container.running ? "running" : "not running"}</td>
        <td><code>{member.container.image}</code><small>{member.container.started_at}</small></td></tr>)}</tbody>
    </table></div>}
  </div>)}</>;
}

/** Mount with the shared authenticated principal/grant identity. */
export function StackPanel({ principal, grantRevision, expired }: { principal: string; grantRevision: number; expired: () => void }) {
  const expiry = useRef(expired); expiry.current = expired;
  const controller = useMemo(() => new StackInventoryController(() => expiry.current()), [principal, grantRevision]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => {
    void controller.start(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  return <StackManagement value={state.value} loading={state.loading} stale={state.stale} error={state.error} refresh={() => { void controller.refresh(); }} />;
}
