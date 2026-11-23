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
    fn stack_request(
        &mut self,
        operation: ContainerAction,
        now: i64,
        expires_at: i64,
    ) -> StackRequest {
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
            expires_at,
        };
        StackRequest {
            action: self.action(),
            plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
            plan,
        }
    }
    async fn stack(&mut self, operation: ContainerAction, now: i64) -> Result<StackReceipt> {
        let request = self.stack_request(operation, now, now + PLAN_TTL_SECONDS);
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
async fn every_charge_counts_whatever_its_outcome() {
    let mut f = Fixture::new();
    // a's effect fails: the stack stops there, b and c are never sent.
    *f.world.fail.lock().unwrap() = Some('a');
    let request = f.stack_request(ContainerAction::Restart, T, T + PLAN_TTL_SECONDS);
    let store = f.store.as_mut().unwrap();
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
    // The unknown outcome counts, and so do the members never sent: a charge
    // is never refunded, so no row escapes the budget.
    let resources = [resource('a'), resource('b'), resource('c')];
    let charged = |f: &mut Fixture| {
        let resources: Vec<&str> = resources.iter().map(String::as_str).collect();
        let one = [Window {
            seconds: 600,
            effects: 1,
        }];
        spent(&f.store().conn, &one, &resources, T + 1)
            .unwrap()
            .into_iter()
            .map(|(r, _)| r)
            .collect::<Vec<_>>()
    };
    assert_eq!(charged(&mut f), resources);
    // So does an outcome never recorded (a crash between prepare and the
    // Engine call).
    f.store()
        .conn
        .execute("UPDATE effect_log SET outcome=NULL", [])
        .unwrap();
    assert_eq!(charged(&mut f), resources);
}

/// A clock core can aim at: the plan is valid for every check before prepare
/// (the first three readings) and expired at the re-check after it, as for
/// a plan whose expiry core put just past the moment it sends it.
fn aimed(valid: i64, expired: i64) -> impl Fn() -> i64 {
    let readings = std::cell::Cell::new(0);
    move || {
        let n = readings.get();
        readings.set(n + 1);
        if n < 3 { valid } else { expired }
    }
}

/// The attack the reviewers found: plans whose expiry falls between prepare
/// and the Engine call leave rows that were charged but never sent, single
/// and as stacks. A day of it every 61 s: every row is still admitted by the
/// budget, no container holds more than a day's budget of effect rows and
/// refusal rows, nothing reaches the Engine, and another container's counted
/// charges stay in the log and keep counting.
#[tokio::test]
async fn charges_never_sent_cannot_flood_the_log_or_evict_another_containers_charges() {
    let mut f = Fixture::new();
    for i in 0..5 {
        f.restart('e', T + i).await.unwrap();
    }
    let sent = f.world.effects();
    let (mut admitted, mut refusals) = (0, 0);
    let mut now = T + 10;
    while now < T + DAILY.seconds {
        let outcome = if now % 2 == 0 {
            let mut request = f.container('a', ContainerAction::Restart, now);
            request.plan.expires_at = now + 1;
            request.plan_digest = limeos_identity::digest(&encode(&request.plan).unwrap());
            let store = f.store.as_mut().unwrap();
            store
                .execute(&f.world, &ceiling(), 1001, &request, aimed(now, now + 1))
                .await
                .map(|r| r.state)
        } else {
            let request = f.stack_request(ContainerAction::Restart, now, now + 1);
            let store = f.store.as_mut().unwrap();
            store
                .execute_stack(&f.world, &ceiling(), 1001, &request, aimed(now, now + 1))
                .await
                .map(|r| r.state)
        };
        match outcome {
            Ok(state) => {
                assert_eq!(state, ExecutionState::PreconditionChanged);
                admitted += 1;
            }
            Err(Error(ErrorCode::RateLimited)) => refusals += 1,
            Err(other) => panic!("{other:?} at {now}"),
        }
        now += 61;
    }
    assert_eq!(f.world.effects(), sent);
    assert!(admitted > 0 && refusals > 0, "{admitted} {refusals}");
    for c in ['a', 'b', 'c'] {
        let r = resource(c);
        let effects = f.rows(&format!(
            "SELECT count(*) FROM effect_log WHERE resource='{r}' AND kind='effect'"
        ));
        let refused = f.rows(&format!(
            "SELECT count(*) FROM effect_log WHERE resource='{r}' AND kind='refused'"
        ));
        assert!(
            effects <= i64::from(DAILY.effects),
            "{c}: {effects} effect rows"
        );
        assert!(refused <= REFUSAL_ROWS, "{c}: {refused} refusal rows");
    }
    assert!(
        f.rows("SELECT count(*) FROM effect_log")
            <= 5 + 3 * (i64::from(DAILY.effects) + REFUSAL_ROWS)
    );
    // e's five charges are all still there and still count.
    let e = resource('e');
    assert_eq!(
        f.rows(&format!(
            "SELECT count(*) FROM effect_log WHERE resource='{e}' AND kind='effect'"
        )),
        5
    );
    let five = [Window {
        seconds: DAILY.seconds,
        effects: 5,
    }];
    assert_eq!(
        spent(&f.store().conn, &five, &[e.as_str()], T + 3600)
            .unwrap()
            .len(),
        1
    );
}

/// Inserts `count` charges for `resource` at `at` straight into the log, as
/// earlier executes would have left them.
fn charges(f: &mut Fixture, resource: &str, at: i64, count: i64) {
    f.store()
        .conn
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{count}) \
             INSERT INTO effect_log(at,kind,resource,operation,action,peer,principal,outcome) \
             SELECT {at},'effect','{resource}','restart',printf('%064x',i),1001,'alice','effect_accepted' FROM n"
        ))
        .unwrap();
}

/// The reviewers' clock scenario: 64 containers take a day's budget; the
/// clock steps back 30 hours, so those charges sit in the future, beyond the
/// window, and each container takes another day's budget; then refusal rows
/// for nine hours. No charge that can still count is pruned to make room:
/// once the clock is corrected, the first day's charges are all there and
/// every container is refused, as its 60 charges say.
#[tokio::test]
async fn charges_that_can_still_count_survive_a_clock_stepped_back_and_corrected() {
    let mut f = Fixture::new();
    let containers: Vec<String> = std::iter::once(resource('a'))
        .chain((1..MANAGED).map(|i| format!("container:{i:064x}")))
        .collect();
    let back = T - 30 * 3600;
    for r in &containers {
        charges(&mut f, r, T, i64::from(DAILY.effects));
        if *r == resource('a') {
            // An hour earlier, out of the burst window of the charge below.
            charges(&mut f, r, back - 3600, i64::from(DAILY.effects) - 1);
        } else {
            charges(&mut f, r, back, i64::from(DAILY.effects));
        }
    }
    for hour in 0..9 {
        f.store()
            .conn
            .execute_batch(&format!(
                "INSERT INTO effect_log(at,kind,resource,operation,action,peer,principal,outcome,refusals) \
                 SELECT {at},'refused',resource,'restart','{x}',1001,'alice','rate_limited',1 FROM (SELECT DISTINCT resource FROM effect_log)",
                at = back + hour * 3600,
                x = "f".repeat(64)
            ))
            .unwrap();
    }
    let before = f.rows("SELECT count(*) FROM effect_log");
    assert!(before > LOG_ROWS, "{before}");
    // a's last charge of the stepped-back day: admitted (59 of 60 count; the
    // first day's are more than a day ahead), and the insert prunes refusal
    // rows only.
    f.restart('a', back).await.unwrap();
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), LOG_ROWS);
    assert_eq!(
        f.rows(&format!(
            "SELECT count(*) FROM effect_log WHERE kind='effect' AND at={T}"
        )),
        MANAGED * i64::from(DAILY.effects)
    );
    // Corrected: the first day's charges count again, and a is refused.
    assert_eq!(
        refused(f.restart('a', T + 3600).await),
        ErrorCode::RateLimited
    );
}

/// A log full of charges that can still count refuses every effect rather
/// than evict one, says when room comes back, and writes no refusal row.
#[tokio::test]
async fn a_log_full_of_charges_that_can_count_refuses_instead_of_evicting() {
    let mut f = Fixture::new();
    // 8,192 charges for containers the ceiling no longer lists, the oldest
    // a little under a day old.
    for i in 0..LOG_ROWS / 64 {
        charges(
            &mut f,
            &format!("container:{i:064x}"),
            T - HOLD + 10 + i,
            64,
        );
    }
    let rows = f.rows("SELECT count(*) FROM effect_log");
    assert_eq!(rows, LOG_ROWS);
    let effects = f.world.effects();
    assert_eq!(refused(f.restart('e', T).await), ErrorCode::RateLimited);
    // Room comes when the oldest of them can no longer count.
    assert_eq!(f.store().take_frees_at(), Some(T + 10));
    assert_eq!(f.world.effects(), effects);
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), rows);
    assert_eq!(
        f.rows("SELECT count(*) FROM effect_log WHERE kind='refused'"),
        0
    );
    // Once it is a day old, the effect is admitted and that charge pruned.
    f.restart('e', T + 10).await.unwrap();
    assert_eq!(f.rows("SELECT count(*) FROM effect_log"), LOG_ROWS);
}

/// The free time counts charges dated in the future that will be in the
/// window by then: ten charges at each of T, T + 600 and T + 1,200, and the
/// clock corrected back to T + 599. The burst frees when the third ten have
/// left it too, at T + 1,800, not at T + 1,200 when the first ten leave.
#[tokio::test]
async fn the_free_time_counts_future_charges_that_will_enter_the_window() {
    let mut f = Fixture::new();
    for at in [T, T + 600, T + 1200] {
        charges(&mut f, &resource('a'), at, i64::from(BURST.effects));
    }
    assert_eq!(
        refused(f.restart('a', T + 599).await),
        ErrorCode::RateLimited
    );
    assert_eq!(f.store().take_frees_at(), Some(T + 1800));
    assert_eq!(
        refused(f.restart('a', T + 1799).await),
        ErrorCode::RateLimited
    );
    f.restart('a', T + 1800).await.unwrap();
}

#[tokio::test]
async fn a_refusal_says_when_every_container_it_names_has_room_again() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    assert_eq!(
        refused(f.restart('a', T + 30).await),
        ErrorCode::RateLimited
    );
    // The first charge leaves the burst window 600 s after it.
    assert_eq!(f.store().take_frees_at(), Some(T + BURST.seconds));
    assert_eq!(f.store().take_frees_at(), None);
    // A stack names the latest of its spent members.
    for i in 0..BURST.effects {
        f.restart('b', T + 100 + i64::from(i)).await.unwrap();
    }
    assert_eq!(
        refused(f.stack(ContainerAction::Restart, T + 200).await),
        ErrorCode::RateLimited
    );
    assert_eq!(f.store().take_frees_at(), Some(T + 100 + BURST.seconds));
    // A day spent: the charge that brings the count below the limit leaves
    // the daily window.
    let mut g = Fixture::new();
    for i in 0..DAILY.effects {
        g.restart('a', T + i64::from(i) * 360).await.unwrap();
    }
    let at = T + i64::from(DAILY.effects) * 360 + 3600;
    assert_eq!(refused(g.restart('a', at).await), ErrorCode::RateLimited);
    assert_eq!(g.store().take_frees_at(), Some(T + DAILY.seconds));
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
async fn a_clock_step_changes_at_most_one_windows_worth_of_budget() {
    let mut f = Fixture::new();
    for i in 0..BURST.effects {
        f.restart('a', T + i64::from(i)).await.unwrap();
    }
    // Five minutes back: the charges are dated after now, within the burst
    // window of it, and still count.
    assert_eq!(
        refused(f.restart('a', T - 300).await),
        ErrorCode::RateLimited
    );
    // An hour back: further in the future than the burst window, so the
    // burst comes back once; the daily window still counts them (ten of 60).
    f.restart('a', T - 3600).await.unwrap();
    // Forward past the window: the charges have aged out.
    f.restart('a', T + BURST.seconds + 10).await.unwrap();
    // Back again, before those charges: they are within the window of now
    // again, and eleven now count against ten.
    assert_eq!(refused(f.restart('a', T).await), ErrorCode::RateLimited);

    // A day's budget spent while the clock ran three years ahead.
    let mut g = Fixture::new();
    let ahead = T + 3 * 365 * DAILY.seconds;
    for i in 0..DAILY.effects {
        g.restart('a', ahead + i64::from(i) * 360).await.unwrap();
    }
    assert_eq!(
        refused(g.restart('a', ahead + DAILY.seconds - 1).await),
        ErrorCode::RateLimited
    );
    // Corrected: those charges are years in the future and no longer count,
    // so the container is not refused for three years.
    g.restart('a', T).await.unwrap();
    // Corrected by less than a day, they still count until they age out.
    let mut h = Fixture::new();
    for i in 0..DAILY.effects {
        h.restart('a', T + i64::from(i) * 360).await.unwrap();
    }
    assert_eq!(
        refused(h.restart('a', T - 7200).await),
        ErrorCode::RateLimited
    );
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
            "SELECT sum(charges),sum(refusals) FROM effect_days",
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
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{rows}) INSERT INTO effect_log(at,kind,resource,operation,action,peer,principal,outcome) SELECT {old},'effect','{e}','restart','{x}',1001,'alice','effect_accepted' FROM n; WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{days}) INSERT INTO effect_days(day,charges,refusals) SELECT i,1,0 FROM n;",
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

/// The receipt store with its retention saturated (1,024 standalone
/// receipts, 256 stack receipts of `stack_members` members, every receipt as
/// an execute writes it) and the action log full of rows of the longest
/// values the protocol admits (a 128-byte principal, a 64-character
/// project), plus 400 days of totals: (pages of the receipts alone, pages in
/// all, file bytes, bytes of the log's b-trees). The executor opens it.
fn saturated_with_a_full_log(stack_members: u32) -> (i64, i64, u64, i64) {
    use limeos_domain::{StackMemberReceipt, StackReceipt};
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    let id = |tag: u32, i: u32| format!("{tag:08x}{i:056x}");
    let member = |i: u32| ContainerSnapshot {
        resource: format!("container:{}", id(7, i)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    };
    let tx = store.conn.unchecked_transaction().unwrap();
    for i in 0..crate::retention::RETAINED_ACTIONS {
        let receipt = RestartReceipt {
            action: id(1, i),
            plan_digest: id(2, i),
            operation: ContainerAction::Restart,
            before: member(i),
            state: ExecutionState::Verified,
            error: None,
        };
        tx.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?2,?3,'verified',?4,'restart',?5)",
            params![receipt.action, receipt.plan_digest, receipt.before.resource, encode(&receipt).unwrap(), T],
        )
        .unwrap();
    }
    let members: Vec<_> = (0..stack_members).map(member).collect();
    let project = "p".repeat(64);
    for i in 0..crate::retention::RETAINED_STACKS {
        let receipt = StackReceipt {
            action: id(3, i),
            plan_digest: id(4, i),
            operation: ContainerAction::Restart,
            before: StackSnapshot::new(&project, members.clone()).unwrap(),
            state: ExecutionState::Verified,
            members: members
                .iter()
                .map(|m| StackMemberReceipt {
                    resource: m.resource.clone(),
                    state: ExecutionState::Verified,
                    error: None,
                })
                .collect(),
            error: None,
        };
        tx.execute(
            "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?2,?3,'verified',?4,'restart',?5)",
            params![receipt.action, receipt.plan_digest, receipt.before.resource, encode(&receipt).unwrap(), T],
        )
        .unwrap();
        for m in &receipt.members {
            let row = crate::stack::member_receipt_for_tests(&receipt, m);
            tx.execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?2,?3,'verified',?4,'restart',?5)",
                params![row.action, row.plan_digest, row.before.resource, encode(&row).unwrap(), T],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
    let pages = |store: &ReceiptStore| -> i64 {
        store
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap()
    };
    let receipts = pages(&store);
    store
        .conn
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{rows}) \
             INSERT INTO effect_log(at,kind,resource,operation,stack,action,peer,principal,outcome,refusals) \
             SELECT {T}+i,'effect','container:'||printf('%064x',i),'restart','stack:{project}',printf('%064x',i),4294967295,'{principal}','outcome_unknown',0 FROM n; \
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{days}) \
             INSERT INTO effect_days(day,charges,refusals) SELECT 20000+i,1000000,1000000 FROM n; \
             PRAGMA wal_checkpoint(TRUNCATE);",
            rows = LOG_ROWS,
            days = DAYS_KEPT,
            principal = "x".repeat(128),
        ))
        .unwrap();
    let total = pages(&store);
    let log: i64 = store
        .conn
        .query_row(
            "SELECT sum(pgsize) FROM dbstat WHERE name IN ('effect_log','effect_log_resource','effect_days')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let file = std::fs::metadata(dir.path().join("receipts.sqlite"))
        .unwrap()
        .len();
    drop(store);
    // The executor still opens it (quick_check reads every page).
    ReceiptStore::open(dir.path()).unwrap();
    (receipts, total, file, log)
}

/// With every stack at the protocol's 16 members and every log row as long
/// as allowed, the store stays well inside its page cap (the last-resort
/// stop on every execute); with stacks the size of the soak's household's
/// largest (6), inside half of it, the bound the soak holds. Prints the sizes
/// the documentation quotes.
#[test]
fn a_full_log_beside_saturated_receipts_stays_within_the_store_cap() {
    for members in [limeos_domain::MAX_STACK_MEMBERS as u32, 6] {
        let (receipts, total, file, log) = saturated_with_a_full_log(members);
        eprintln!(
            "stacks of {members}: saturated receipts {receipts} pages; with a full log {total} pages ({file} bytes); the log's b-trees {log} bytes, {} a row",
            log / LOG_ROWS
        );
        assert!(total * 4 < i64::from(crate::MAX_PAGES) * 3, "{total} pages");
        if members <= 6 {
            assert!(
                file <= u64::from(crate::MAX_PAGES) * 4096 / 2,
                "{file} bytes"
            );
        }
    }
}
