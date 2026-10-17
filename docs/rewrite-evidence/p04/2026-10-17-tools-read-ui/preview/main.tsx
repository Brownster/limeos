import { createRoot } from "react-dom/client";
import { ToolsStatus } from "../../../frontend/src/ToolsStatus";
import { decodeToolsStatus } from "../../../frontend/src/tools-status-client";
import wire from "../../../tests/fixtures/deployment/tools/wire.json";
import "../../../frontend/src/style.css";

const value = decodeToolsStatus(wire);
const partial = decodeToolsStatus({ ...wire, payload: { ...wire.payload, sampled_at: 103 }, service: { ...wire.service, sampled_at: 105 } });
createRoot(document.getElementById("root")!).render(<main className="signed-in">
  <h1>Tools observation fixtures</h1>
  <p>Standalone synthetic preview for local review.</p>
  <ToolsStatus value={value} now={105} refresh={() => {}} />
  <ToolsStatus value={partial} now={191} />
  <ToolsStatus value={null} error="Tools status is unavailable. Refresh to try again." />
</main>);
