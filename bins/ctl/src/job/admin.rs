//! `job list` and `job cancel`: every user's jobs, and cancelling a queued
//! one on its owner's behalf, from the machine itself. Root-only like
//! `job resolve`: core serves both to a root peer only, and `--admin` names
//! the administrator a cancellation is recorded under. Both go through core,
//! which holds the store; limeosctl never opens `core.sqlite`.
use limeos_contracts::{CoreRequest, CoreResponse, VERSION};
use limeos_domain::{
    CancelOutcome, CancelRefusal, ErrorCode, JobFamily, JobState, MAX_OPERATOR_JOBS,
    OperatorCancel, OperatorJob, OperatorJobFilter, OperatorJobPage,
};

pub const USAGE: &str = "job list [--state STATES] [--owner USERNAME] [--family FAMILY] [--limit N] [--after POSITION] [--json]\n  root-only; every user's jobs, oldest first: ID, owner, family and operation,\n  resource, state, when queued, deadline (plan expiry), open resolution request.\n  STATES is unsettled (the default: queued, running, verifying, outcome_unknown,\n  needs_intervention, planned, waiting_approval), all, or a comma-separated list.\n  FAMILY is container, stack, storage-targets, storage-mount,\n  automation-observation, health-probe or backup-preparation. Shows at most N\n  jobs (default 32, at most 64), then how many more match and the --after\n  POSITION that lists them. --json prints one JSON object per job, then one\n  line of totals with that position as \"next\".\njob cancel JOB_ID --admin USERNAME [--yes]\n  root-only; cancels a queued job of any user, recorded under that administrator.\n  Shows the job first; without --yes it asks you to type cancel, so it needs a\n  terminal. Refuses a job that is not queued (settle a held one with job\n  resolve) and an automation observation. Never contacts an executor.";
const DEFAULT_LIMIT: u32 = 32;
use JobState::*;
const UNSETTLED: [JobState; 7] = [
    Planned,
    WaitingApproval,
    Queued,
    Running,
    Verifying,
    OutcomeUnknown,
    NeedsIntervention,
];
const ALL: [JobState; 12] = [
    Planned,
    WaitingApproval,
    Queued,
    Running,
    Verifying,
    OutcomeUnknown,
    NeedsIntervention,
    Succeeded,
    Failed,
    Canceled,
    PreconditionChanged,
    Resolved,
];
/// Held: dispatched and not settled. Only receipts or `job resolve` settle these.
const HELD: [JobState; 4] = [Running, Verifying, OutcomeUnknown, NeedsIntervention];
/// What to do when core cannot be reached.
const CORE_DOWN: &str = "job list and job cancel go through core, which holds the store; with core stopped nothing can cancel a job. To see what is queued meanwhile, use the read-only listing in docs/p03-operations.md (restoring the pre-upgrade snapshot, step 5), and close it before core starts: an open core.sqlite blocks a schema upgrade.";
/// Core could not parse the request.
const OLDER_CORE: &str = "core refused the request as malformed: a core older than this limeosctl does not know job list or job cancel. Use the limeosctl installed with that core, or the read-only listing in docs/p03-operations.md (restoring the pre-upgrade snapshot, step 5).";

/// Why a cancel without `--yes` and without a terminal is refused.
const CONFIRM: &str = "job cancel without --yes needs a terminal to show the job and ask for confirmation; nothing was sent. Check the job with `limeosctl job list`, then run `limeosctl job cancel JOB_ID --admin USERNAME --yes`.";

#[derive(Debug, PartialEq, Eq)]
pub struct List {
    pub filter: OperatorJobFilter,
    pub limit: u32,
    pub json: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Cancel {
    pub admin: String,
    pub job: String,
    /// Act without asking: the operator has already checked the job.
    pub yes: bool,
}

fn state(name: &str) -> Result<JobState, String> {
    serde_json::from_value(serde_json::Value::String(name.replace('-', "_"))).map_err(|_| {
        format!("--state: unknown state {name}; use unsettled, all or a list of job states")
    })
}
fn states(value: &str) -> Result<Vec<JobState>, String> {
    let mut states = match value {
        "unsettled" => UNSETTLED.to_vec(),
        "all" => ALL.to_vec(),
        _ => value.split(',').map(state).collect::<Result<Vec<_>, _>>()?,
    };
    // Listing order, without repeats.
    states.sort_by_key(|s| ALL.iter().position(|a| a == s));
    states.dedup();
    Ok(states)
}

fn family(name: &str) -> Result<JobFamily, String> {
    serde_json::from_value(serde_json::Value::String(name.replace('-', "_")))
        .map_err(|_| format!("--family: unknown family {name}; run limeosctl --help"))
}

/// `args` follow `job list`. Each flag at most once, in any order.
pub fn parse_list(args: &[String]) -> Result<List, String> {
    let mut filter = OperatorJobFilter {
        states: UNSETTLED.to_vec(),
        owner: None,
        family: None,
        job: None,
        after: None,
    };
    let (mut chosen, mut limit, mut json) = (false, None, false);
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        if flag == "--json" {
            if std::mem::replace(&mut json, true) {
                return Err(format!("{flag} given more than once"));
            }
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--state" | "--limit" | "--owner" | "--family" | "--after"
        ) {
            return Err(format!("unknown argument {flag}; run limeosctl --help"));
        }
        // A value that looks like a flag (`--state --version`) is refused.
        let value = crate::args::value(flag, flags.next()).map_err(|usage| usage.0)?;
        let taken = match flag.as_str() {
            "--state" => {
                filter.states = states(value)?;
                std::mem::replace(&mut chosen, true)
            }
            "--owner" => {
                if !limeos_domain::identifier(value) {
                    return Err(format!("--owner: {value} is not a username"));
                }
                filter.owner.replace(value.clone()).is_some()
            }
            "--family" => filter.family.replace(family(value)?).is_some(),
            "--after" => {
                let position = value
                    .parse::<i64>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| {
                        format!("--after takes a position a listing printed, not {value}")
                    })?;
                filter.after.replace(position).is_some()
            }
            _ => {
                let n = value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=MAX_OPERATOR_JOBS).contains(n))
                    .ok_or_else(|| {
                        format!("--limit takes 1 to {MAX_OPERATOR_JOBS}, not {value}")
                    })?;
                limit.replace(n).is_some()
            }
        };
        if taken {
            return Err(format!("{flag} given more than once"));
        }
    }
    Ok(List {
        filter,
        limit: limit.unwrap_or(DEFAULT_LIMIT),
        json,
    })
}

/// `args` follow `job cancel`: the job, then `--admin USERNAME` and
/// optionally `--yes`, in either order.
pub fn parse_cancel(args: &[String]) -> Result<Cancel, String> {
    let usage = || format!("usage:\n{USAGE}");
    let (job, mut flags) = match args.split_first() {
        Some((job, flags)) if !job.starts_with('-') => (job, flags.iter()),
        _ => return Err(usage()),
    };
    if !limeos_domain::opaque_id(job) {
        return Err("JOB_ID must be the 64-character job identifier".into());
    }
    let (mut admin, mut yes) = (None, false);
    while let Some(flag) = flags.next() {
        let taken = match flag.as_str() {
            "--yes" => std::mem::replace(&mut yes, true),
            "--admin" => {
                let value = crate::args::value(flag, flags.next()).map_err(|usage| usage.0)?;
                admin.replace(value.clone()).is_some()
            }
            _ => return Err(format!("unknown argument {flag}; run limeosctl --help")),
        };
        if taken {
            return Err(format!("{flag} given more than once"));
        }
    }
    Ok(Cancel {
        admin: admin.ok_or("--admin USERNAME is required")?,
        job: job.clone(),
        yes,
    })
}

/// `seconds` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn names(states: &[JobState]) -> String {
    states
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the filter names, for the totals line.
fn described(filter: &OperatorJobFilter) -> String {
    let mut out = format!("states: {}", names(&filter.states));
    if let Some(owner) = &filter.owner {
        out.push_str(&format!("; owner: {owner}"));
    }
    if let Some(family) = filter.family {
        out.push_str(&format!("; family: {}", family.as_str()));
    }
    if let Some(after) = filter.after {
        out.push_str(&format!("; after position {after}"));
    }
    out
}

/// Shown, matching, and the line that says how to list the rest.
fn totals(page: &OperatorJobPage, filter: &OperatorJobFilter) -> String {
    let shown = page.jobs.len();
    let mut out = if page.matching == 0 {
        format!("no jobs match ({})\n", described(filter))
    } else {
        format!(
            "{shown} shown of {} matching jobs ({})\n",
            page.matching,
            described(filter)
        )
    };
    if let Some(next) = page.next {
        out.push_str(&format!(
            "{} more after these: list them with --after {next}\n",
            page.remaining
        ));
    }
    out
}

/// A table, one row per job, then the totals. `now` marks passed deadlines.
pub fn render_table(page: &OperatorJobPage, filter: &OperatorJobFilter, now: i64) -> String {
    let header = [
        "JOB",
        "OWNER",
        "FAMILY",
        "OPERATION",
        "STATE",
        "RESOURCE",
        "CLAIMS",
        "QUEUED",
        "DEADLINE",
        "RESOLUTION",
    ]
    .map(String::from);
    let mut rows = vec![header];
    for j in &page.jobs {
        rows.push([
            j.job.clone(),
            j.owner.clone(),
            j.family.as_str().into(),
            j.operation.clone(),
            j.state.as_str().into(),
            j.resource.clone(),
            j.claims.to_string(),
            if j.created_at == 0 {
                "-".into()
            } else {
                utc(j.created_at)
            },
            format!(
                "{}{}",
                utc(j.deadline),
                if j.deadline <= now { " (passed)" } else { "" }
            ),
            if j.resolution_open { "open" } else { "-" }.into(),
        ]);
    }
    let mut out = String::new();
    if !page.jobs.is_empty() {
        let widths: Vec<usize> = (0..rows[0].len())
            .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
            .collect();
        for row in &rows {
            let cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, w)| format!("{cell:<w$}"))
                .collect();
            out.push_str(cells.join("  ").trim_end());
            out.push('\n');
        }
    }
    out.push_str(&totals(page, filter));
    out
}

/// One JSON object per job, then `{"shown","matching","more","next"}`.
pub fn render_json(page: &OperatorJobPage) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    for job in &page.jobs {
        out.push_str(&serde_json::to_string(job)?);
        out.push('\n');
    }
    out.push_str(&serde_json::to_string(&serde_json::json!({
        "shown": page.jobs.len(),
        "matching": page.matching,
        "more": page.remaining,
        "next": page.next,
    }))?);
    out.push('\n');
    Ok(out)
}

/// What the operator is told, and whether the job is now canceled.
pub fn render_cancel(answer: &OperatorCancel, admin: &str) -> (String, bool) {
    let (job, family, state) = (&answer.job, answer.family.as_str(), answer.state);
    match (answer.outcome, answer.refusal) {
        (CancelOutcome::Canceled, _) => (
            format!(
                "canceled: job {job} ({family}) was queued and is now canceled, recorded under administrator {admin}. No executor had been sent it and it held no lock; it can never be dispatched.\n"
            ),
            true,
        ),
        (CancelOutcome::AlreadyCanceled, _) => (
            format!("job {job} ({family}) was already canceled; nothing changed.\n"),
            true,
        ),
        (_, Some(CancelRefusal::PauseScheduleInstead)) => (
            format!(
                "refused: job {job} is an automation observation ({}). It is read-only and sends no effect; while the observation runtime is running it settles the job within about a minute (as failed once its 60-second deadline passes, or precondition_changed if its owner's authority changed). Only pausing its schedule cancels one: core refuses to start on a store holding a cancellation that no pause explains. Nothing changed.\n",
                state.as_str()
            ),
            false,
        ),
        _ if HELD.contains(&state) => {
            let next = match answer.family {
                JobFamily::Container | JobFamily::Stack => format!(
                    "Settle it with `limeosctl job show-resolution {job} --admin {admin}`, then `job resolve`."
                ),
                _ => "Core's recovery settles it from the storage executor's receipt; job resolve covers only container and stack jobs.".into(),
            };
            (
                format!(
                    "refused: job {job} ({family}) is {}, not queued: core may already have sent it to an executor, so cancelling could not prove nothing ran. Nothing changed. {next}\n",
                    state.as_str()
                ),
                false,
            )
        }
        _ => (
            format!(
                "refused: job {job} ({family}) is {}, not queued; only a queued job can be cancelled. Nothing changed.\n",
                state.as_str()
            ),
            false,
        ),
    }
}

/// An error from core or the socket, with what it means for these commands.
fn explain(error: Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
    if error.is::<crate::Unreachable>() {
        return format!("{error}\n{CORE_DOWN}").into();
    }
    match error.downcast_ref::<crate::Refused>().map(|r| r.code) {
        Some(ErrorCode::InvalidInput) => format!("{error}\n{OLDER_CORE}").into(),
        Some(ErrorCode::NotFound) => format!(
            "{error}\nNo job has that ID, or no user has the name given to --admin. Nothing changed."
        )
        .into(),
        Some(ErrorCode::Forbidden) => format!(
            "{error}\n--admin must name a current administrator, and the request must come from root. Nothing changed."
        )
        .into(),
        _ => error,
    }
}

pub async fn list(
    socket: &str,
    list: List,
    now: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let page = fetch(socket, list.filter.clone(), list.limit).await?;
    Ok(if list.json {
        render_json(&page)?
    } else {
        render_table(&page, &list.filter, now)
    })
}

async fn fetch(
    socket: &str,
    filter: OperatorJobFilter,
    limit: u32,
) -> Result<OperatorJobPage, Box<dyn std::error::Error>> {
    let response = crate::rpc(
        socket,
        &CoreRequest::ListJobs {
            version: VERSION,
            filter,
            limit,
        },
    )
    .await
    .map_err(explain)?;
    match response {
        CoreResponse::Jobs(page) => Ok(page),
        _ => Err("unexpected response from the core".into()),
    }
}

/// The job as `job cancel` shows it before asking.
pub fn render_subject(job: &OperatorJob) -> String {
    format!(
        "job: {}\nowner: {}\noperation: {} ({})\nresource: {}\nstate: {}\nqueued: {}\ndeadline: {}\n",
        job.job,
        job.owner,
        job.operation,
        job.family.as_str(),
        job.resource,
        job.state.as_str(),
        if job.created_at == 0 {
            "unknown".into()
        } else {
            utc(job.created_at)
        },
        utc(job.deadline)
    )
}

/// Shows the job, then cancels it once the operator has confirmed: `--yes`,
/// or `cancel` typed on the terminal (`confirm`). Without either, nothing is
/// sent at all. A job that is not queued is sent without asking: core
/// refuses it and changes nothing. Returns the text to print, or the refusal
/// as the error.
pub async fn cancel(
    socket: &str,
    cancel: Cancel,
    confirm: Option<super::Confirm<'_>>,
) -> Result<String, Box<dyn std::error::Error>> {
    if !cancel.yes && confirm.is_none() {
        return Err(CONFIRM.into());
    }
    let filter = OperatorJobFilter {
        states: ALL.to_vec(),
        owner: None,
        family: None,
        job: Some(cancel.job.clone()),
        after: None,
    };
    let page = fetch(socket, filter, 1).await?;
    let subject = page
        .jobs
        .first()
        .ok_or("no job has that ID; nothing changed")?;
    print!("{}", render_subject(subject));
    let asks =
        subject.state == JobState::Queued && subject.family != JobFamily::AutomationObservation;
    // Without `--yes` there is a terminal (checked above) to ask on.
    if asks
        && !cancel.yes
        && let Some(confirm) = confirm
    {
        print!("Type cancel to cancel this job on its owner's behalf, anything else to keep it: ");
        std::io::Write::flush(&mut std::io::stdout())?;
        if confirm()?.trim() != "cancel" {
            return Err("not confirmed; nothing was sent".into());
        }
    }
    let response = crate::rpc(
        socket,
        &CoreRequest::CancelJob {
            version: VERSION,
            admin: cancel.admin.clone(),
            job: cancel.job,
        },
    )
    .await
    .map_err(explain)?;
    let CoreResponse::JobCanceled(answer) = response else {
        return Err("unexpected response from the core".into());
    };
    match render_cancel(&answer, &cancel.admin) {
        (text, true) => Ok(text),
        (text, false) => Err(text.trim_end().to_owned().into()),
    }
}
