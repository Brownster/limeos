import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type Ref,
} from "react";
import type {
  ContainerAction,
  ContainerSnapshot,
  DeclaredOutcome,
  NeedsAttentionJob,
  ResolveKind,
  ResolutionTargetView,
  ResolutionView,
} from "../../contracts/generated/types";
import { containFocus } from "./operation-ui";
import { resolutionClient } from "./resolution-client";
import {
  declarationLabels,
  declarations,
  disabledReason,
  MAX_REASON,
  MIN_DIVERGENCE_REASON,
  ResolutionController,
  type ResolutionState,
} from "./resolution-controller";

/** A job that is waiting for an administrator, as listed by the job panels. */
export interface AttentionJob {
  id: string;
  title: string;
  /** Username of another principal's job; absent for the viewer's own. */
  owner?: string;
}

const operationLabels: Record<ContainerAction, string> = {
  restart: "Restart",
  start: "Start",
  stop: "Stop",
};
/** Same wording as the job rows, so one job reads the same from either list. */
function listedTitle(job: NeedsAttentionJob): string {
  return job.kind === "stack"
    ? `${operationLabels[job.operation]} stack · ${job.resource.slice(6)}`
    : `${operationLabels[job.operation]} · ${job.resource.slice(10, 22)}`;
}
/**
 * The viewer's own stuck jobs plus the administrator listing for this kind,
 * once each. An own job keeps its own entry and no owner label; every other
 * entry names its owner.
 */
export function mergeAttention(
  own: AttentionJob[],
  listed: NeedsAttentionJob[],
  kind: ResolveKind,
): AttentionJob[] {
  const seen = new Set(own.map((job) => job.id));
  const others: AttentionJob[] = [];
  for (const job of listed) {
    if (job.kind !== kind || seen.has(job.job)) continue;
    seen.add(job.job);
    others.push({ id: job.job, title: listedTitle(job), owner: job.owner });
  }
  return [...own, ...others];
}

const meaning: Record<DeclaredOutcome, string> = {
  verified_done:
    "Every container is already in the state the plan intended. The lock is released and the job is recorded as done.",
  not_done:
    "Nothing took effect: every container still equals the plan. Only the lock is released. Plan again separately if you still want the change.",
  accepted_divergence:
    "The present state differs from both the plan and the intended result, and you accept it as it is. The lock is released and the next plan starts from this state.",
};
/**
 * Give focus back when the dialog closes: to the control that opened it, or,
 * when that control left the page (its job was resolved and its banner entry
 * went away), to the fallback, so focus is never dropped on the document body.
 */
export function restoreFocus(
  opener: { isConnected: boolean; focus(): void } | null,
  fallback: () => HTMLElement | null,
): "opener" | "fallback" | "none" {
  if (opener && opener.isConnected && typeof opener.focus === "function") {
    opener.focus();
    return "opener";
  }
  const target = fallback();
  if (!target) return "none";
  if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
  target.focus();
  return "fallback";
}
const outcome = (state: string) => state.replaceAll("_", " ");
const shortId = (id: string) => id.slice(0, 12);

/**
 * Banner listing jobs that hold resources until an administrator settles them.
 * Everyone who sees the job panel sees it; only administrators get the button.
 */
export function NeedsAttention({
  jobs,
  admin,
  csrf,
  expired,
  changed,
}: {
  jobs: AttentionJob[];
  admin: boolean;
  csrf: string;
  expired: () => void;
  changed: () => void;
}) {
  const [open, setOpen] = useState<AttentionJob | null>(null);
  // Two banners can be on one page (containers and stacks): ids must not repeat.
  const headingId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const anchor = useRef<HTMLSpanElement>(null);
  const fallback = () =>
    heading.current ??
    anchor.current?.parentElement?.querySelector<HTMLElement>("h2, h3") ??
    null;
  // The dialog outlives the list entry so the resolved record can be read.
  const dialog = admin && open && (
    <ResolutionDialog
      key={open.id}
      job={open}
      csrf={csrf}
      expired={expired}
      changed={changed}
      fallback={fallback}
      onClose={() => setOpen(null)}
    />
  );
  return (
    <>
    <span ref={anchor} hidden />
    {jobs.length > 0 && <section className="attention" aria-labelledby={headingId}>
      <h3 id={headingId} tabIndex={-1} ref={heading}>
        Needs attention · {jobs.length}{" "}
        {jobs.length === 1 ? "operation" : "operations"}
      </h3>
      <p>
        {jobs.length === 1 ? "This operation has" : "These operations have"} an
        unknown outcome. Their containers stay locked and nothing is repeated
        until an administrator checks the evidence and resolves them.
        {admin ? "" : " Ask an administrator to review them."}
      </p>
      <ul>
        {jobs.map((job) => (
          <li key={job.id}>
            {job.title}
            {job.owner !== undefined && (
              <span className="badge"> owned by {job.owner}</span>
            )}
            {admin && (
              <button
                className="quiet"
                aria-label={`Review and resolve ${job.title}`}
                onClick={() => setOpen(job)}
              >
                Review and resolve
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>}
    {dialog}
    </>
  );
}

export function ResolutionDialog({
  job,
  csrf,
  expired,
  changed,
  fallback = () => null,
  onClose,
}: {
  job: AttentionJob;
  csrf: string;
  expired: () => void;
  changed: () => void;
  /** Where focus goes when the opener has left the page. */
  fallback?: () => HTMLElement | null;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const opener = useRef<Element | null>(null);
  const latest = useRef({ changed, expired });
  latest.current = { changed, expired };
  const focusFallback = useRef(fallback);
  focusFallback.current = fallback;
  const controller = useMemo(
    () =>
      new ResolutionController(
        job.id,
        csrf,
        resolutionClient,
        () => latest.current.changed(),
        () => latest.current.expired(),
      ),
    [job.id, csrf],
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );
  // Held only until the next submit; cleared whatever the answer.
  const [password, setPassword] = useState("");
  useEffect(() => {
    opener.current = document.activeElement;
    dialog.current?.showModal();
    void controller.load();
    return () => {
      controller.dispose();
      dialog.current?.close();
      restoreFocus(opener.current as HTMLElement | null, () => focusFallback.current());
    };
  }, [controller]);
  // Move focus to the new step's heading so screen readers announce it.
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, [state.step, state.result]);
  const submit = async () => {
    const typed = password;
    setPassword("");
    await controller.submit(typed);
  };
  return (
    <dialog
      ref={dialog}
      className="operation-dialog resolution-dialog"
      aria-labelledby={`resolve-${job.id}`}
      onClose={onClose}
      onKeyDown={containFocus}
    >
      <header className="page-heading">
        <h2 id={`resolve-${job.id}`}>Resolve · {job.title}</h2>
        <button
          className="quiet"
          autoFocus
          aria-label="Close resolution dialog"
          onClick={() => dialog.current?.close()}
        >
          Close
        </button>
      </header>
      <ResolutionBody
        state={state}
        headingRef={heading}
        password={password}
        onPassword={setPassword}
        onChoose={(d) => controller.choose(d)}
        onReason={(r) => controller.setReason(r)}
        onReview={() => controller.review()}
        onBack={() => controller.back()}
        onSubmit={() => void submit()}
        onRefresh={() => void controller.load()}
      />
    </dialog>
  );
}

const noop = () => {};
/** The dialog's content as a pure function of the controller state. */
export function ResolutionBody({
  state,
  headingRef,
  password = "",
  onPassword = noop,
  onChoose = noop,
  onReason = noop,
  onReview = noop,
  onBack = noop,
  onSubmit = noop,
  onRefresh = noop,
}: {
  state: ResolutionState;
  headingRef?: Ref<HTMLHeadingElement>;
  password?: string;
  onPassword?: (value: string) => void;
  onChoose?: (declared: DeclaredOutcome) => void;
  onReason?: (reason: string) => void;
  onReview?: () => void;
  onBack?: () => void;
  onSubmit?: () => void;
  onRefresh?: () => void;
}) {
  const { view } = state;
  const awaiting = view?.evidence_digest != null;
  const done = state.result === "resolved";
  return (
    <div>
      <p className="notice">
        Resolving only records your decision and releases the lock. It never
        starts, stops or restarts anything.
      </p>
      <div aria-live="polite">
        {state.loading && <p role="status">Inspecting the containers…</p>}
        {state.notice && <p className="notice" role="status">{state.notice}</p>}
      </div>
      {state.error && <p className="error" role="alert">{state.error}</p>}
      {state.interrupted && (
        <button className="quiet" onClick={onRefresh}>
          Refresh to check the job
        </button>
      )}
      {view && !done && (
        <>
          <Evidence view={view} blocking={state.blocking} />
          {view.pending && (
            <p className="notice" role="status">
              A decision ({declarationLabels[view.pending.declared]}) is
              recorded and waiting for the executor. The core retries on its
              own; refresh to check.
            </p>
          )}
          {view.resolution && (
            <ResolvedRecord view={view} />
          )}
          {awaiting && state.step === "choose" && (
            <Choose
              view={view}
              state={state}
              onChoose={onChoose}
              onReason={onReason}
              onReview={onReview}
              headingRef={headingRef}
            />
          )}
          {awaiting && state.step === "confirm" && state.declared && (
            <Confirm
              view={view}
              state={state}
              password={password}
              onPassword={onPassword}
              onBack={onBack}
              onSubmit={onSubmit}
              headingRef={headingRef}
            />
          )}
          {!awaiting && !view.resolution && !view.pending && (
            <p>This job is not awaiting resolution.</p>
          )}
          {!state.loading && (
            <button className="quiet" onClick={onRefresh}>
              Refresh evidence
            </button>
          )}
        </>
      )}
      {done && state.resolution && (
        <section aria-labelledby="resolved-heading">
          <h3 id="resolved-heading" tabIndex={-1} ref={headingRef}>Resolved</h3>
          <Record
            declared={state.resolution.declared}
            reason={state.resolution.reason}
            at={state.resolution.resolved_at}
          />
          <p>The containers are unlocked and can be planned again.</p>
        </section>
      )}
    </div>
  );
}

function Snapshot({ value }: { value: ContainerSnapshot }) {
  return (
    <dl className="plan-details">
      <dt>Running</dt>
      <dd>{value.running ? "yes" : "no"}</dd>
      <dt>Image</dt>
      <dd>{value.image || "none"}</dd>
      <dt>Started</dt>
      <dd>{value.started_at || "none"}</dd>
    </dl>
  );
}
function Target({
  target,
  blocked,
}: {
  target: ResolutionTargetView;
  blocked: boolean;
}) {
  const { expected, measured } = target;
  const missing = measured.image === "" && measured.started_at === "";
  return (
    <article
      className={blocked ? "resolution-target blocked" : "resolution-target"}
      aria-label={`Container ${shortId(measured.container)}`}
    >
      <h4>
        <code>{shortId(measured.container)}</code>
        {blocked && <span className="badge warning">Blocks this outcome</span>}
      </h4>
      <div className="side-by-side">
        <div>
          <h5>Plan expected</h5>
          <Snapshot value={expected} />
        </div>
        <div>
          <h5>Measured now</h5>
          {missing ? (
            <p className="notice">
              The Engine no longer has this container. That proves neither
              “done” nor “nothing changed”.
            </p>
          ) : (
            <Snapshot
              value={{
                resource: measured.container,
                image: measured.image,
                started_at: measured.started_at,
                running: measured.running,
              }}
            />
          )}
        </div>
      </div>
      <p>
        {measured.matches_expected
          ? "Same as the plan."
          : "Differs from the plan."}{" "}
        {measured.intended_state_reached
          ? "The intended end state is present."
          : "The intended end state is not present."}
      </p>
    </article>
  );
}
function Evidence({
  view,
  blocking,
}: {
  view: ResolutionView;
  blocking: string[];
}) {
  return (
    <section aria-labelledby="evidence-heading">
      <h3 id="evidence-heading">
        {view.kind === "stack" ? "Stack" : "Container"} {view.operation}{" "}
        <span className="badge warning">{outcome(view.state)}</span>
      </h3>
      {view.members && view.members.length > 0 && (
        <>
          <h4>What the executor recorded</h4>
          <ul className="job-events" aria-label="Per-container outcomes">
            {view.members.map((member) => (
              <li key={member.container}>
                <code>{shortId(member.container)}</code> ·{" "}
                {member.state === "precondition_changed"
                  ? "not attempted"
                  : outcome(member.state)}
                {member.error ? ` (${outcome(member.error)})` : ""}
              </li>
            ))}
          </ul>
        </>
      )}
      {view.targets.map((target) => (
        <Target
          key={target.measured.container}
          target={target}
          blocked={blocking.includes(target.measured.container)}
        />
      ))}
    </section>
  );
}
function Record({
  declared,
  reason,
  at,
}: {
  declared: DeclaredOutcome;
  reason: string;
  at?: number;
}) {
  return (
    <dl className="plan-details">
      <dt>Outcome</dt>
      <dd>{declarationLabels[declared]}</dd>
      <dt>Reason</dt>
      <dd>{reason}</dd>
      {at !== undefined && (
        <>
          <dt>Resolved</dt>
          <dd>{new Date(at * 1000).toISOString()}</dd>
        </>
      )}
    </dl>
  );
}
function ResolvedRecord({ view }: { view: ResolutionView }) {
  const r = view.resolution!;
  return (
    <section aria-labelledby="record-heading">
      <h3 id="record-heading">Already resolved</h3>
      <Record declared={r.declared} reason={r.reason} at={r.resolved_at} />
    </section>
  );
}

function Choose({
  view,
  state,
  onChoose,
  onReason,
  onReview,
  headingRef,
}: {
  view: ResolutionView;
  state: ResolutionState;
  onChoose: (declared: DeclaredOutcome) => void;
  onReason: (reason: string) => void;
  onReview: () => void;
  headingRef?: Ref<HTMLHeadingElement>;
}) {
  const divergent = state.declared === "accepted_divergence";
  const count = [...state.reason].length;
  return (
    <form
      aria-labelledby="choose-heading"
      onSubmit={(event) => {
        event.preventDefault();
        onReview();
      }}
    >
      <h3 id="choose-heading" tabIndex={-1} ref={headingRef}>
        Your decision
      </h3>
      <fieldset className="outcomes">
        <legend>What is true of these containers?</legend>
        {declarations.map((declared) => {
          const why = disabledReason(view, declared);
          return (
            <div
              key={declared}
              className={declared === "accepted_divergence" ? "outcome divergence" : "outcome"}
            >
              <label>
                <input
                  type="radio"
                  name="declared"
                  value={declared}
                  checked={state.declared === declared}
                  disabled={why !== null}
                  aria-describedby={`why-${declared}`}
                  onChange={() => onChoose(declared)}
                />{" "}
                {declarationLabels[declared]}
              </label>
              <p id={`why-${declared}`}>{why ?? meaning[declared]}</p>
            </div>
          );
        })}
      </fieldset>
      {state.declared && (
        <>
          <label htmlFor="resolution-reason">
            Reason (
            {divergent
              ? `at least ${MIN_DIVERGENCE_REASON}, at most ${MAX_REASON} characters`
              : `1 to ${MAX_REASON} characters`}
            )
          </label>
          <textarea
            id="resolution-reason"
            value={state.reason}
            rows={3}
            required
            aria-required="true"
            aria-invalid={state.invalid !== ""}
            aria-describedby="reason-help"
            onChange={(event) => onReason(event.target.value)}
          />
          <p id="reason-help">
            {count} characters. Say what you checked on the host and why this
            outcome is right.
          </p>
        </>
      )}
      {state.invalid && <p className="error" role="alert">{state.invalid}</p>}
      <button type="submit" disabled={!state.declared}>
        Review decision
      </button>
    </form>
  );
}

function Confirm({
  view,
  state,
  password,
  onPassword,
  onBack,
  onSubmit,
  headingRef,
}: {
  view: ResolutionView;
  state: ResolutionState;
  password: string;
  onPassword: (value: string) => void;
  onBack: () => void;
  onSubmit: () => void;
  headingRef?: Ref<HTMLHeadingElement>;
}) {
  const declared = state.declared!;
  const divergent = declared === "accepted_divergence";
  return (
    <form
      aria-labelledby="confirm-heading"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit();
      }}
    >
      <h3 id="confirm-heading" tabIndex={-1} ref={headingRef}>
        Confirm with your password
      </h3>
      <div className={divergent ? "divergence summary" : "summary"}>
        <dl className="plan-details">
          <dt>Outcome</dt>
          <dd>{declarationLabels[declared]}</dd>
          <dt>Reason</dt>
          <dd>{state.reason}</dd>
          <dt>Containers</dt>
          <dd>
            {view.targets
              .map((target) => shortId(target.measured.container))
              .join(", ")}
          </dd>
          <dt>Evidence digest</dt>
          <dd>{view.evidence_digest}</dd>
        </dl>
        <p>
          This decision is bound to the evidence digest above. If anything
          changes first, it is refused and you see the new evidence.
        </p>
        {divergent && (
          <p>
            You are accepting a state that differs from both the plan and the
            intended result.
          </p>
        )}
      </div>
      {state.result === "pending" && (
        <p className="notice">
          Already recorded. Submitting the identical decision again is safe.
        </p>
      )}
      <label htmlFor="resolution-password">Your password</label>
      <input
        id="resolution-password"
        type="password"
        autoComplete="current-password"
        value={password}
        required
        onChange={(event) => onPassword(event.target.value)}
      />
      <div className="container-controls">
        <button
          type="submit"
          className={divergent ? "danger" : ""}
          disabled={state.submitting || password === ""}
        >
          {state.submitting
            ? "Submitting…"
            : state.result === "pending"
              ? "Submit the same decision again"
              : "Resolve job"}
        </button>
        <button
          type="button"
          className="quiet"
          disabled={state.submitting}
          onClick={onBack}
        >
          Back
        </button>
      </div>
    </form>
  );
}
