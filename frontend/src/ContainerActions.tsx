// Ported from pi-health@80593 container-list's Restart control and pending label.
// The Containers page (src/pages/ContainersPage.tsx) now owns the lifecycle and
// log dialogs and the operations list (src/containers/*). This keeps the same
// row controls for any other resource table that lists a container.
import { useState } from "react";
import type { Resource } from "../../contracts/generated/types";
import { navigate } from "./app/router";
import { buildRows } from "./containers/container-model";
import { RowActions, type RowHandlers } from "./containers/ContainerList";
import { LifecycleDialog, type LifecycleTarget } from "./containers/LifecycleDialog";
import { LogsDialog, type LogsTarget } from "./containers/LogsDialog";

let nonce = 0;

export function ContainerActions({ resource, csrf, changed }: { resource: Resource; csrf: string; changed: () => void }) {
  const [lifecycle, setLifecycle] = useState<LifecycleTarget | null>(null);
  const [logs, setLogs] = useState<LogsTarget | null>(null);
  const [row] = buildRows([resource], [], true);
  const handlers: RowHandlers = {
    canManage: true,
    now: Date.now() / 1000,
    current: true,
    onAction: (_, action, opener) => setLifecycle({ resource, action, opener, nonce: ++nonce }),
    onLogs: (_, opener) => setLogs({ resource, opener, nonce: ++nonce }),
    onDetail: () => navigate(`/containers?container=${encodeURIComponent(resource.name)}`),
    onShowOperation: () => navigate("/containers"),
  };
  return (
    <>
      <RowActions row={row} handlers={handlers} compact={false} />
      <LifecycleDialog target={lifecycle} csrf={csrf} changed={changed} onClosed={() => setLifecycle(null)} />
      <LogsDialog target={logs} onClosed={() => setLogs(null)} />
    </>
  );
}
