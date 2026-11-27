import { useEffect, useState } from "react";
import type { HistoryRange, MetricHistory } from "../../../contracts/generated/types";
import { api, ApiError } from "../api";
import { useOverview } from "../app/overview-data";
import { HistoryTable, PartialNotice, SourceList } from "../ObservationView";
import { PageHeader, StateMessage } from "../ui/primitives";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import type { PageProps } from "./Pages";

export function OverviewPage({ expired }: PageProps) {
  const { value, refreshKey } = useOverview();
  const [range] = useState<HistoryRange>("24h");
  const [history, setHistory] = useState<MetricHistory | null>(null);
  const [historyError, setHistoryError] = useState<unknown>(null);
  useEffect(() => {
    let active = true;
    setHistoryError(null);
    void api.history(range).then((h) => { if (active) setHistory(h); }).catch((e: unknown) => {
      if (!active) return;
      if (e instanceof ApiError && e.detail.code === "unauthenticated") expired();
      else setHistoryError(e);
    });
    return () => { active = false; };
  }, [range, refreshKey, expired]);
  return (
    <>
      <PageHeader title="Overview" actions={<OverviewRefresh />} />
      <OverviewPending />
      {value && <div className="panel"><PartialNotice value={value} /><SourceList value={value} /></div>}
      {historyError ? <StateMessage kind="error" error={historyError} subject="History" /> : history && <div className="panel"><HistoryTable history={history} /></div>}
    </>
  );
}
