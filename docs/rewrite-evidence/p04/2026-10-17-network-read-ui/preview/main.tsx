import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { NetworkGroups } from "../../../frontend/src/NetworkGroups";
import "../../../frontend/src/style.css";
import wire from "../../../tests/fixtures/deployment/networks/wire.json";
import type { NetworkInventory } from "../../../frontend/src/network-inventory-client";

function Preview() {
  const [mode, setMode] = useState("fresh");
  const value = mode === "unknown" ? { ...wire, source: "unavailable", sampled_at: null, observed_containers: null, independent_containers: null, unknown_mode_containers: null, groups: [], members: [] } : wire;
  return <><p className="eyebrow">LOCAL FIXTURE PREVIEW</p><h1>limeOS · Network groups</h1>
    <div className="resource-nav" aria-label="Preview fixture"><button onClick={() => setMode("fresh")}>Fresh fixture</button><button onClick={() => setMode("unknown")}>Unknown fixture</button><button onClick={() => setMode("expired")}>Expired fixture</button></div>
    <NetworkGroups value={value as NetworkInventory} now={mode === "expired" ? 191 : 105} refresh={() => setMode("fresh")} />
  </>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
