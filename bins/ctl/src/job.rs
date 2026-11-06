//! `job show-resolution` and `job resolve`: settle a job stuck in
//! `needs_intervention` from the machine itself, for when the web UI cannot be
//! reached. Root-only like `enroll`: the core accepts these requests only from a
//! root peer, so being root on the host is the re-authentication, and `--admin`
//! names the administrator the decision is recorded under. Same evidence, same
//! checks and same crash-safe order as the HTTP route.
use limeos_contracts::{
    CoreRequest, CoreResponse, ResolutionView, ResolveJobResponse, ResolveStatus, VERSION,
};
use limeos_domain::DeclaredOutcome;
use std::io::Read;

pub const USAGE: &str = "job show-resolution JOB_ID --admin USERNAME\n  root-only; fresh read-only evidence for a stuck container or stack job.\njob resolve JOB_ID --admin USERNAME --outcome verified-done|not-done|accepted-divergence [--reason TEXT] [--evidence-digest HEX]\n  root-only; records the decision under that administrator. The reason is read from\n  stdin when --reason is absent. Shows the evidence first and decides on its digest;\n  --evidence-digest from show-resolution refuses if the state has changed since.\n  Never starts, stops or restarts anything.";
const REASON_LIMIT: u64 = 2048;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Show {
        admin: String,
        job: String,
    },
    Resolve {
        admin: String,
        job: String,
        declared: DeclaredOutcome,
        reason: Option<String>,
        digest: Option<String>,
    },
}

fn outcome(text: &str) -> Result<DeclaredOutcome, String> {
    match text {
        "verified-done" => Ok(DeclaredOutcome::VerifiedDone),
        "not-done" => Ok(DeclaredOutcome::NotDone),
        "accepted-divergence" => Ok(DeclaredOutcome::AcceptedDivergence),
        _ => Err("--outcome must be verified-done, not-done or accepted-divergence".into()),
    }
}

/// `args` follow the word `job`. Flags may come in any order; each is given once.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let usage = || format!("usage:\n{USAGE}");
    let (sub, rest) = args.split_first().ok_or_else(usage)?;
    let (job, mut flags) = match rest.split_first() {
        Some((job, flags)) if !job.starts_with('-') => (job.clone(), flags.iter()),
        _ => return Err(usage()),
    };
    if !limeos_domain::opaque_id(&job) {
        return Err("JOB_ID must be the 64-character job identifier".into());
    }
    let (mut admin, mut declared, mut reason, mut digest) = (None, None, None, None);
    while let Some(flag) = flags.next() {
        let value = flags
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        let taken = match flag.as_str() {
            "--admin" => admin.replace(value.clone()).is_some(),
            "--outcome" => declared.replace(outcome(value)?).is_some(),
            "--reason" => reason.replace(value.clone()).is_some(),
            "--evidence-digest" => digest.replace(value.clone()).is_some(),
            _ => return Err(format!("unknown argument {flag}; run limeosctl --help")),
        };
        if taken {
            return Err(format!("{flag} given more than once"));
        }
    }
    let admin = admin.ok_or("--admin USERNAME is required")?;
    match sub.as_str() {
        "show-resolution" if declared.is_none() && reason.is_none() && digest.is_none() => {
            Ok(Command::Show { admin, job })
        }
        "show-resolution" => Err("show-resolution takes only --admin".into()),
        "resolve" => Ok(Command::Resolve {
            admin,
            job,
            declared: declared.ok_or("--outcome is required")?,
            reason,
            digest,
        }),
        _ => Err(usage()),
    }
}

fn yes(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}
/// The wire spelling of a plain enum, as the API and contracts show it.
fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
fn name(declared: DeclaredOutcome) -> String {
    declared.as_str().replace('_', "-")
}

/// Plan against measurement, one block per target.
pub fn render_view(view: &ResolutionView) -> String {
    let mut out = format!(
        "job: {}\nstate: {}\nkind: {} {}\n",
        view.job,
        json(&view.state),
        json(&view.kind),
        json(&view.operation)
    );
    for t in &view.targets {
        let (e, m) = (&t.expected, &t.measured);
        out.push_str(&format!("target: {}\n", m.container));
        out.push_str(&format!(
            "  planned:  running {} started {} image {}\n",
            yes(e.running),
            e.started_at,
            e.image
        ));
        if m.is_absent() {
            out.push_str("  measured: container no longer exists\n");
        } else {
            out.push_str(&format!(
                "  measured: running {} started {} image {}\n",
                yes(m.running),
                m.started_at,
                m.image
            ));
        }
        out.push_str(&format!(
            "  unchanged from plan: {}   intended state reached: {}\n",
            yes(m.matches_expected),
            yes(m.intended_state_reached)
        ));
    }
    if let Some(digest) = &view.evidence_digest {
        out.push_str(&format!("evidence digest: {digest}\n"));
        let allowed: Vec<_> = view.allowed.iter().map(|d| name(*d)).collect();
        out.push_str(&format!("outcomes available: {}\n", allowed.join(", ")));
    }
    for m in view.members.iter().flatten() {
        out.push_str(&format!(
            "executor outcome: {} {}{}\n",
            m.container,
            json(&m.state),
            m.error
                .map(|e| format!(" ({})", json(&e)))
                .unwrap_or_default()
        ));
    }
    if let Some(p) = &view.pending {
        out.push_str(&format!(
            "pending: {} by {} (not yet confirmed; the job stays held)\n",
            name(p.declared),
            p.resolved_by
        ));
    }
    if let Some(r) = &view.resolution {
        out.push_str(&format!(
            "resolved: {} by {} at {}\n  reason: {}\n",
            name(r.declared),
            r.resolved_by,
            r.resolved_at,
            r.reason
        ));
    }
    out
}

/// The outcome of a decision: text for the operator and whether it settled.
pub fn render_response(response: &ResolveJobResponse) -> (String, bool) {
    let mut out = String::new();
    if let Some(view) = &response.view {
        out.push_str(&render_view(view));
    }
    if !response.blocking.is_empty() {
        out.push_str(&format!("blocking: {}\n", response.blocking.join(", ")));
    }
    out.push_str(&response.message);
    out.push('\n');
    (out, response.status == ResolveStatus::Resolved)
}

fn reason_from(
    given: Option<String>,
    stdin: &mut impl Read,
) -> Result<String, Box<dyn std::error::Error>> {
    let text = match given {
        Some(text) => text,
        None => {
            let mut input = String::new();
            stdin.take(REASON_LIMIT + 1).read_to_string(&mut input)?;
            if input.len() as u64 > REASON_LIMIT {
                return Err("the reason is too long".into());
            }
            input
        }
    };
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("a reason is required: pass --reason TEXT or write it on stdin".into());
    }
    Ok(text)
}

async fn read(
    socket: &str,
    admin: &str,
    job: &str,
) -> Result<ResolutionView, Box<dyn std::error::Error>> {
    match crate::rpc(
        socket,
        &CoreRequest::ReadResolution {
            version: VERSION,
            admin: admin.into(),
            job: job.into(),
        },
    )
    .await?
    {
        CoreResponse::Resolution(view) => Ok(view),
        _ => Err("unexpected response from the core".into()),
    }
}

pub async fn run(socket: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let command = parse(args)?;
    if rustix::process::geteuid().as_raw() != 0 {
        return Err("job resolution over the local socket is root-only".into());
    }
    execute(socket, command, &mut std::io::stdin()).await
}

async fn execute(
    socket: &str,
    command: Command,
    stdin: &mut impl Read,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Show { admin, job } => {
            print!("{}", render_view(&read(socket, &admin, &job).await?));
            Ok(())
        }
        Command::Resolve {
            admin,
            job,
            declared,
            reason,
            digest,
        } => {
            let reason = reason_from(reason, stdin)?;
            let view = read(socket, &admin, &job).await?;
            print!("{}", render_view(&view));
            let shown = view
                .evidence_digest
                .clone()
                .ok_or("the job is not awaiting resolution")?;
            if digest.is_some_and(|expected| expected != shown) {
                return Err("the state changed since that evidence digest; review the evidence above and decide again".into());
            }
            let CoreResponse::Resolved(response) = crate::rpc(
                socket,
                &CoreRequest::ResolveJob {
                    version: VERSION,
                    admin,
                    job,
                    declared,
                    reason,
                    evidence_digest: shown,
                },
            )
            .await?
            else {
                return Err("unexpected response from the core".into());
            };
            let (text, settled) = render_response(&response);
            if settled {
                print!("{text}");
                Ok(())
            } else {
                Err(text.trim_end().to_owned().into())
            }
        }
    }
}

#[cfg(test)]
mod tests;
