use super::*;
use limeos_domain::{PLAN_TTL_SECONDS, RestartPlan, StackPlan, StackSnapshot};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const T: i64 = 1_800_000_000;

fn private_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn resource(c: char) -> String {
    format!("container:{}", c.to_string().repeat(64))
}
fn snap(c: char, running: bool, generation: u32) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: resource(c),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: format!(
            "2026-10-05T{:02}:{:02}:{:02}Z",
            7 + generation / 3600,
            (generation / 60) % 60,
            generation % 60
        ),
        running,
    }
}
fn ceiling() -> ContainerCeiling {
    ContainerCeiling {
        version: 1,
        core_uid: 1001,
        allow_restart: true,
        allow_start: true,
        allow_stop: true,
        allow_stack: true,
        managed_containers: ["a", "b", "c", "e"].map(|c| c.repeat(64)).to_vec(),
        managed_stacks: vec!["media".into()],
    }
}

/// Containers a, b and c form the `media` stack; e stands alone. An effect
/// moves the start time forward, as a real Engine would.
struct World {
    state: Mutex<BTreeMap<char, (bool, u32)>>,
    effects: AtomicUsize,
    /// The member whose effect fails (the Engine answers an error, nothing
    /// changes).
    fail: Mutex<Option<char>>,
}
impl World {
    fn new() -> Self {
        Self {
            state: Mutex::new(['a', 'b', 'c', 'e'].map(|c| (c, (true, 0))).into()),
            effects: AtomicUsize::new(0),
            fail: Mutex::new(None),
        }
    }
    fn now(&self, c: char) -> ContainerSnapshot {
        let (running, generation) = self.state.lock().unwrap()[&c];
        snap(c, running, generation)
    }
    fn effects(&self) -> usize {
        self.effects.load(Ordering::SeqCst)
    }
    fn effect(&self, id: &str, running: bool, restarts: bool) -> Result<()> {
        let c = id.chars().next().unwrap();
        self.effects.fetch_add(1, Ordering::SeqCst);
        if *self.fail.lock().unwrap() == Some(c) {
            return Err(Error(ErrorCode::Unavailable));
        }
        let mut state = self.state.lock().unwrap();
        let entry = state.get_mut(&c).unwrap();
        entry.0 = running;
        if restarts {
            entry.1 += 1;
        }
        Ok(())
    }
}
impl Engine for World {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        Ok(self.now(id.chars().next().unwrap()))
    }
    async fn restart(&self, id: &str) -> Result<()> {
        self.effect(id, true, true)
    }
    async fn start(&self, id: &str) -> Result<()> {
        self.effect(id, true, true)
    }
    async fn stop(&self, id: &str) -> Result<()> {
        self.effect(id, false, false)
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        Ok(['a', 'b', 'c'].map(|c| self.now(c)).to_vec())
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Option<ReceiptStore>,
    world: World,
    next: u32,
}
impl Fixture {
    fn new() -> Self {
        let dir = private_dir();
        let store = Some(ReceiptStore::open(dir.path()).unwrap());
        Self {
            dir,
            store,
            world: World::new(),
            next: 0,
        }
    }
    fn store(&mut self) -> &mut ReceiptStore {
        self.store.as_mut().unwrap()
    }
    /// An executor restart: the store closed and opened again.
    fn reopen(&mut self) {
        self.store = None;
        self.store = Some(ReceiptStore::open(self.dir.path()).unwrap());
    }
    fn action(&mut self) -> String {
        self.next += 1;
        format!("{:064x}", self.next)
    }
    fn container(&mut self, c: char, operation: ContainerAction, now: i64) -> RestartRequest {
        let plan = RestartPlan {
            operation,
            id: self.action(),
            version: 1,
            principal: "alice".into(),
            grant_revision: 1,
            expected: self.world.now(c),
            created_at: now,
            expires_at: now + PLAN_TTL_SECONDS,
        };
        RestartRequest {
            action: self.action(),
            plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
            plan,
        }
    }
    /// Executes `request` at `now` and, once accepted, verifies it, as core
    /// does, so the container's lock is free for the next action.
    async fn run(&mut self, request: &RestartRequest, now: i64) -> Result<RestartReceipt> {
        let store = self.store.as_mut().unwrap();
        let receipt = store
            .execute(&self.world, &ceiling(), 1001, request, || now)
            .await?;
        if receipt.state == ExecutionState::EffectAccepted {
            let c = request.plan.expected.resource.chars().nth(10).unwrap();
            store
                .verify(
                    &self.world,
                    &ceiling(),
                    1001,
                    &request.action,
                    &request.plan_digest,
                    &self.world.now(c),
                )
                .await
                .unwrap();
        }
        Ok(receipt)
    }
    async fn act(
        &mut self,
        c: char,
        operation: ContainerAction,
        now: i64,
    ) -> Result<RestartReceipt> {
        let request = self.container(c, operation, now);
        self.run(&request, now).await
    }
    async fn restart(&mut self, c: char, now: i64) -> Result<RestartReceipt> {
        self.act(c, ContainerAction::Restart, now).await
    }
    async fn stack(&mut self, operation: ContainerAction, now: i64) -> Result<StackReceipt> {
        let plan = StackPlan {
            id: self.action(),
            version: limeos_domain::STACK_VERSION,
            operation,
            principal: "alice".into(),
            grant_revision: 1,
            catalog_stack: "registered".into(),
            expected: StackSnapshot::new(
                "media",
                ['a', 'b', 'c'].map(|c| self.world.now(c)).to_vec(),
            )
            .unwrap(),
            created_at: now,
            expires_at: now + PLAN_TTL_SECONDS,
        };
        let request = StackRequest {
            action: self.action(),
            plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
            plan,
        };
        let store = self.store.as_mut().unwrap();
        let receipt = store
            .execute_stack(&self.world, &ceiling(), 1001, &request, || now)
            .await?;
        let after: Vec<_> = ['a', 'b', 'c'].map(|c| self.world.now(c)).to_vec();
        store
            .verify_stack(
                &self.world,
                &ceiling(),
                1001,
                &request.action,
                &request.plan_digest,
                &after,
            )
            .await
            .unwrap();
        Ok(receipt)
    }
    /// The action log, oldest first: kind, container letter, outcome and
    /// refusals.
    fn log(&mut self) -> Vec<(String, char, Option<String>, u32)> {
        let store = self.store();
        let mut statement = store
            .conn
            .prepare("SELECT kind,resource,outcome,refusals FROM effect_log ORDER BY seq")
            .unwrap();
        statement
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, String>(1)?.chars().nth(10).unwrap(),
                    r.get(2)?,
                    r.get(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
    fn rows(&mut self, sql: &str) -> i64 {
        self.store().conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }
}
fn refused<T: std::fmt::Debug>(result: Result<T>) -> ErrorCode {
    result.unwrap_err().0
}

#[tokio::test]
async fn a_container_gets_its_burst_then_is_refused_until_the_window_moves_on() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        let receipt = f.restart('a', T + i64::from(i)).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::EffectAccepted, "effect {i}");
    }
    assert_eq!(f.world.effects(), 10);
    let over = f.container('a', ContainerAction::Restart, T + 30);
    assert_eq!(refused(f.run(&over, T + 30).await), ErrorCode::RateLimited);
    // Refused before prepare: no Engine call, no receipt, nothing to close.
    assert_eq!(f.world.effects(), 10);
    assert!(
        f.store()
            .receipt(&over.action, &over.plan_digest)
            .unwrap()
            .is_none()
    );
    // The first effect leaves the window 600 s after it was charged.
    assert_eq!(
        refused(f.restart('a', T + BURST.seconds - 1).await),
        ErrorCode::RateLimited
    );
    f.restart('a', T + BURST.seconds).await.unwrap();
    assert_eq!(f.world.effects(), 11);
}

#[tokio::test]
async fn the_daily_budget_holds_after_every_burst_window_and_returns_a_day_later() {
    let mut f = Fixture::new();
    // Ten an hour, spread out: never more than the burst in ten minutes.
    let at = |i: u32| T + i64::from(i) * 360;
    for i in 0..DAILY.effects {
        f.restart('a', at(i)).await.unwrap();
    }
    let last = at(DAILY.effects - 1);
    assert_eq!(
        refused(f.restart('a', last + 3600).await),
        ErrorCode::RateLimited
    );
    assert_eq!(
        refused(f.restart('a', T + DAILY.seconds - 1).await),
        ErrorCode::RateLimited
    );
    // A day after the first effect, it has left the daily window.
    f.restart('a', T + DAILY.seconds).await.unwrap();
    assert_eq!(f.world.effects(), DAILY.effects as usize + 1);
}

#[tokio::test]
async fn containers_have_independent_budgets_and_operations_share_one() {
    let mut f = Fixture::new();
    // Stop and start alternate on one budget.
    for i in 0..BURST.effects {
        let operation = if i % 2 == 0 {
            ContainerAction::Stop
        } else {
            ContainerAction::Start
        };
        f.act('a', operation, T + i64::from(i)).await.unwrap();
    }
    for operation in [ContainerAction::Stop, ContainerAction::Restart] {
        assert_eq!(
            refused(f.act('a', operation, T + 20).await),
            ErrorCode::RateLimited
        );
    }
    // Container e is untouched by a's spent budget, and the reverse.
    for i in 0..BURST.effects {
        f.restart('e', T + 20 + i64::from(i)).await.unwrap();
    }
    assert_eq!(
        refused(f.restart('e', T + 40).await),
        ErrorCode::RateLimited
    );
    assert_eq!(
        refused(f.restart('a', T + 40).await),
        ErrorCode::RateLimited
    );
    assert_eq!(f.world.effects(), 20);
}

#[tokio::test]
async fn a_stack_charges_every_target_and_is_refused_whole_when_one_is_spent() {
    let mut f = Fixture::new();
    // A restart of the stack charges a, b and c once each.
    let receipt = f.stack(ContainerAction::Restart, T).await.unwrap();
    assert_eq!(receipt.state, ExecutionState::EffectAccepted);
    assert_eq!(
        f.log(),
        ['a', 'b', 'c'].map(|c| (
            "effect".to_owned(),
            c,
            Some("effect_accepted".to_owned()),
            0
        ))
    );
    let stack: String = f
        .store()
        .conn
        .query_row("SELECT DISTINCT stack FROM effect_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stack, "stack:media");
    // Single actions on b use up the rest of b's burst: nine more.
    for i in 1..BURST.effects {
        f.restart('b', T + i64::from(i)).await.unwrap();
    }
    let effects = f.world.effects();
    let rows = f.rows("SELECT count(*) FROM stack_actions");
    // The stack is refused whole: nothing sent to a or c either, no receipt.
    assert_eq!(
        refused(f.stack(ContainerAction::Restart, T + 30).await),
        ErrorCode::RateLimited
    );
    assert_eq!(f.world.effects(), effects);
    assert_eq!(f.rows("SELECT count(*) FROM stack_actions"), rows);
    // b alone is refused too: its budget is one, whichever way it is spent.
    assert_eq!(
        refused(f.restart('b', T + 30).await),
        ErrorCode::RateLimited
    );
    // a still has room: the stack charged it once.
    f.restart('a', T + 31).await.unwrap();
    // The refusal names the spent member only.
    let refusals: Vec<_> = f
        .log()
        .into_iter()
        .filter(|(kind, ..)| kind == "refused")
        .map(|(_, c, ..)| c)
        .collect();
    assert_eq!(refusals, ['b']);
}

#[tokio::test]
async fn members_never_sent_do_not_count_and_a_crash_before_the_outcome_does() {
    let mut f = Fixture::new();
    // a's effect fails: the stack stops there, b and c are never sent.
    *f.world.fail.lock().unwrap() = Some('a');
    let store = f.store.as_mut().unwrap();
    let plan = StackPlan {
        id: "1".repeat(64),
        version: limeos_domain::STACK_VERSION,
        operation: ContainerAction::Restart,
        principal: "alice".into(),
        grant_revision: 1,
        catalog_stack: "registered".into(),
        expected: StackSnapshot::new("media", ['a', 'b', 'c'].map(|c| f.world.now(c)).to_vec())
            .unwrap(),
        created_at: T,
        expires_at: T + PLAN_TTL_SECONDS,
    };
    let request = StackRequest {
        action: "2".repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    };
    store
        .execute_stack(&f.world, &ceiling(), 1001, &request, || T)
        .await
        .unwrap();
    assert_eq!(
        f.log(),
        [
            (
                "effect".to_owned(),
                'a',
                Some("outcome_unknown".to_owned()),
                0
            ),
            ("effect".to_owned(), 'b', Some("not_sent".to_owned()), 0),
            ("effect".to_owned(), 'c', Some("not_sent".to_owned()), 0),
        ]
    );
    let resources = [resource('a'), resource('b')];
    let charged = |f: &mut Fixture, window: &[Window]| {
        let resources: Vec<&str> = resources.iter().map(String::as_str).collect();
        spent(&f.store().conn, window, &resources, T + 1).unwrap()
    };
    // a's unknown outcome counts; b's unsent effect does not.
    let one = [Window {
        seconds: 600,
        effects: 1,
    }];
    assert_eq!(charged(&mut f, &one), [resource('a')]);
    // An outcome never recorded (a crash between prepare and the Engine
    // call) counts.
    f.store()
        .conn
        .execute(
            "UPDATE effect_log SET outcome=NULL WHERE resource=?1",
            [&resource('b')],
        )
        .unwrap();
    assert_eq!(charged(&mut f, &one), resources);
}

#[tokio::test]
async fn the_budget_survives_an_executor_restart() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    f.reopen();
    assert_eq!(
        refused(f.restart('a', T + 20).await),
        ErrorCode::RateLimited
    );
    f.reopen();
    f.restart('a', T + BURST.seconds).await.unwrap();
    assert_eq!(f.world.effects(), 11);
}

#[tokio::test]
async fn a_clock_stepped_back_keeps_recent_charges_and_one_stepped_forward_ages_them() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    // An hour back: every charge is dated after now, and still counts.
    assert_eq!(
        refused(f.restart('a', T - 3600).await),
        ErrorCode::RateLimited
    );
    // A day back, the same: a backward step only ever refuses more.
    assert_eq!(
        refused(f.restart('a', T - DAILY.seconds).await),
        ErrorCode::RateLimited
    );
    // Forward past the window: the charges have aged out.
    f.restart('a', T + BURST.seconds + 10).await.unwrap();
    // Back again, before those charges: eleven now count against ten.
    assert_eq!(refused(f.restart('a', T).await), ErrorCode::RateLimited);
}

#[tokio::test]
async fn a_budget_that_cannot_be_read_refuses_the_effect() {
    let mut f = Fixture::new();
    f.store()
        .conn
        .execute_batch("DROP INDEX effect_log_resource; ALTER TABLE effect_log RENAME TO moved")
        .unwrap();
    assert_eq!(refused(f.restart('a', T).await), ErrorCode::StateNotDurable);
    assert_eq!(f.world.effects(), 0);
    assert_eq!(f.rows("SELECT count(*) FROM actions"), 0);
    // A store whose log is missing does not open at all.
    f.store = None;
    assert_eq!(
        ReceiptStore::open(f.dir.path()).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
}

#[tokio::test]
async fn replays_lookups_closures_and_verification_are_never_charged_or_refused() {
    let mut f = Fixture::new();
    let first = f.container('a', ContainerAction::Restart, T);
    let store = f.store.as_mut().unwrap();
    store
        .execute(&f.world, &ceiling(), 1001, &first, || T)
        .await
        .unwrap();
    // Verified later, after the budget is spent.
    for i in 1..BURST.effects {
        // The lock is held by the unverified first action, so the others go
        // to e, then a's budget is spent directly in the log.
        f.restart('e', T + i64::from(i)).await.unwrap();
    }
    f.store()
        .conn
        .execute_batch(&format!(
            "INSERT INTO effect_log(at,kind,resource,operation,action,peer,principal,outcome) SELECT {T},'effect','{}','restart','{}',1001,'alice','effect_accepted' FROM (SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9)",
            resource('a'),
            "f".repeat(64)
        ))
        .unwrap();
    let rows = f.rows("SELECT count(*) FROM effect_log");
    let effects = f.world.effects();
    // A replay of the first action answers its receipt.
    let store = f.store.as_mut().unwrap();
    let replay = store
        .execute(&f.world, &ceiling(), 1001, &first, || T + 1)
        .await
        .unwrap();
    assert_eq!(replay.state, ExecutionState::EffectAccepted);
    // A lookup of it, and a closing lookup of an action never sent.
    let lookup = store
        .lookup(
            &ceiling(),
            1001,
            &first.action,
            &first.plan_digest,
            &first.plan,
            true,
            T + 1,
        )
        .unwrap();
    assert!(
        matches!(lookup, Lookup::Receipt { closed: false, .. }),
        "{lookup:?}"
    );
    let never = f.container('a', ContainerAction::Restart, T + 1);
    let store = f.store.as_mut().unwrap();
    let closed = store
        .lookup(
            &ceiling(),
            1001,
            &never.action,
            &never.plan_digest,
            &never.plan,
            true,
            T + 1,
        )
        .unwrap();
    assert!(
        matches!(closed, Lookup::Receipt { closed: true, .. }),
        "{closed:?}"
    );
    // Verification of the first.
    let verified = store
        .verify(
            &f.world,
            &ceiling(),
            1001,
            &first.action,
            &first.plan_digest,
            &f.world.now('a'),
        )
        .await
        .unwrap();
    assert_eq!(verified.state, ExecutionState::Verified);
    // None of it was charged, and nothing more reached the Engine.
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), rows);
    assert_eq!(f.world.effects(), effects);
    // A new action on a is refused.
    assert_eq!(refused(f.restart('a', T + 2).await), ErrorCode::RateLimited);
}

#[tokio::test]
async fn new_ids_digests_and_spellings_buy_no_extra_effects() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    // A fresh action id and plan id (and so digest) is a new effect, charged
    // like any other.
    let mut fresh = f.container('a', ContainerAction::Restart, T + 20);
    fresh.plan.id = "9".repeat(64);
    fresh.plan.principal = "bob".into();
    fresh.plan_digest = limeos_identity::digest(&encode(&fresh.plan).unwrap());
    assert_eq!(refused(f.run(&fresh, T + 20).await), ErrorCode::RateLimited);
    // Another spelling of the container is not in the ceiling: refused
    // before the budget is even read.
    for spelling in [
        format!("container:{}", "A".repeat(64)),
        format!("container:{}", "a".repeat(12)),
        format!("container: {}", "a".repeat(64)),
    ] {
        let mut other = f.container('a', ContainerAction::Restart, T + 21);
        other.plan.expected.resource = spelling;
        other.plan_digest = limeos_identity::digest(&encode(&other.plan).unwrap());
        assert_ne!(refused(f.run(&other, T + 21).await), ErrorCode::RateLimited);
    }
    // The budget is keyed on the listed id the ceiling matched, which a
    // fresh inspection equals: the stack route charges the same key.
    assert_eq!(
        refused(f.stack(ContainerAction::Restart, T + 22).await),
        ErrorCode::RateLimited
    );
    assert_eq!(f.world.effects(), 10);
}

#[tokio::test]
async fn one_record_per_effect_and_coalesced_records_for_a_flood_of_refusals() {
    // One long window, so the budget stays spent for the whole test.
    static LONG: [Window; 1] = [Window {
        seconds: 30 * 86_400,
        effects: 10,
    }];
    let mut f = Fixture::new();
    f.store().windows = &LONG;
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    let journal = f.store().take_journal();
    assert_eq!(journal.len(), 10);
    assert!(journal.iter().all(|r| r.kind == "effect"
        && r.outcome == "effect_accepted"
        && r.peer == 1001
        && r.principal == "alice"
        && r.resource == resource('a')));
    // A thousand refused requests in the next hour: one row written.
    for i in 0..1000 {
        let _ = f.restart('a', T + 10 + i * 3).await.unwrap_err();
    }
    let journal = f.store().take_journal();
    assert_eq!(journal.len(), 1);
    assert_eq!((journal[0].kind, journal[0].refusals), ("refused", 1));
    let log = f.log();
    assert_eq!(log.len(), 11);
    assert_eq!(
        log[10],
        ("refused".into(), 'a', Some("rate_limited".into()), 1)
    );
    // An hour after that row, the next refusal writes one carrying every
    // refusal counted since.
    let _ = f.restart('a', T + 10 + REFUSAL_INTERVAL).await.unwrap_err();
    assert_eq!(
        f.log()[11],
        ("refused".into(), 'a', Some("rate_limited".into()), 1000)
    );
    // The journal copies the table, row for row.
    let journal = f.store().take_journal();
    let seq = f.rows("SELECT max(seq) FROM effect_log");
    assert_eq!((journal.len(), journal[0].seq), (1, seq));
    // Totals by day.
    let (effects, refusals): (i64, i64) = f
        .store()
        .conn
        .query_row(
            "SELECT sum(effects),sum(refusals) FROM effect_days",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((effects, refusals), (10, 1001));
}

#[tokio::test]
async fn the_log_and_its_daily_totals_stay_within_their_row_bounds() {
    let mut f = Fixture::new();
    // Rows a long flood left, and days of totals.
    f.store()
        .conn
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{rows}) INSERT INTO effect_log(at,kind,resource,operation,action,peer,principal,outcome) SELECT {old},'effect','{e}','restart','{x}',1001,'alice','effect_accepted' FROM n; WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{days}) INSERT INTO effect_days(day,effects,refusals) SELECT i,1,0 FROM n;",
            rows = LOG_ROWS + 500,
            days = DAYS_KEPT + 50,
            old = T - 2 * DAILY.seconds,
            e = resource('e'),
            x = "f".repeat(64),
        ))
        .unwrap();
    let newest = f.rows("SELECT max(seq) FROM effect_log");
    f.restart('a', T).await.unwrap();
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), LOG_ROWS);
    assert_eq!(f.rows("SELECT max(seq) FROM effect_log"), newest + 1);
    assert_eq!(
        f.rows("SELECT min(seq) FROM effect_log"),
        newest + 2 - LOG_ROWS
    );
    assert_eq!(f.rows("SELECT count(*) FROM effect_days"), DAYS_KEPT);
    assert_eq!(
        f.rows("SELECT max(day) FROM effect_days"),
        T.div_euclid(86_400)
    );
    // A refusal row trims too.
    for i in 1..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    let _ = f.restart('a', T + 20).await.unwrap_err();
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), LOG_ROWS);
}

#[test]
fn a_store_from_before_the_log_gains_it_empty() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    store
        .conn
        .execute_batch("DROP TABLE effect_log; DROP TABLE effect_days; PRAGMA user_version=8;")
        .unwrap();
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, RECEIPT_SCHEMA_VERSION);
    let rows: i64 = store
        .conn
        .query_row("SELECT count(*) FROM effect_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}
