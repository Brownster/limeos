//! The shared journal's discipline, through a receipt type of its own. The
//! target and mount journals' own tests drive it through their receipts.
use super::*;
use serde::Deserialize;

const TEST_SCHEMA: &str = "BEGIN IMMEDIATE; CREATE TABLE receipts(action TEXT PRIMARY KEY,digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','done')),body TEXT NOT NULL) STRICT; CREATE TABLE locks(resource TEXT PRIMARY KEY,action TEXT NOT NULL REFERENCES receipts(action)) STRICT; PRAGMA user_version=1; COMMIT;";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Plain {
    action: String,
    plan: String,
    digest: String,
    done: bool,
}
impl Receipt for Plain {
    const DATABASE: &'static str = "plain.sqlite";
    const SCHEMA: &'static str = TEST_SCHEMA;
    const BODY_BYTES: usize = 1024;
    const UNSAFE_DIRECTORY: Failure = Failure::UnsafePath;
    fn action(&self) -> &str {
        &self.action
    }
    fn digest(&self) -> &str {
        &self.digest
    }
    fn state_name(&self) -> &'static str {
        if self.done { "done" } else { "prepared" }
    }
    fn holds_claims(&self) -> bool {
        !self.done
    }
    fn claims(&self) -> Vec<String> {
        vec![format!("plain:{}", self.plan)]
    }
    fn plan_digest(&self) -> Result<String> {
        Ok(limeos_identity::digest(&self.plan))
    }
    fn admit(&self) -> bool {
        true
    }
    fn same_subject(&self, next: &Self) -> bool {
        self.action == next.action && self.plan == next.plan && self.digest == next.digest
    }
}
type PlainJournal = Journal<Plain>;

fn plain(n: u32) -> Plain {
    let plan = format!("plan-{n}");
    Plain {
        action: format!("{n:064x}"),
        digest: limeos_identity::digest(&plan),
        plan,
        done: false,
    }
}
fn directory(temp: &tempfile::TempDir) -> OwnedFd {
    rustix::fs::open(
        temp.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap()
}
fn me() -> u32 {
    rustix::process::geteuid().as_raw()
}
fn writer(temp: &tempfile::TempDir) -> Result<PlainJournal> {
    PlainJournal::from_directory(directory(temp), me())
}
fn reader(temp: &tempfile::TempDir) -> Result<PlainJournal> {
    PlainJournal::readonly_from(directory(temp), me())
}

#[test]
fn a_reader_needs_an_existing_journal_and_shares_the_lock_only_with_readers() {
    let temp = tempfile::tempdir().unwrap();
    assert!(reader(&temp).is_err());
    assert!(!temp.path().join("plain.sqlite").exists());
    let mut journal = writer(&temp).unwrap();
    journal.insert(&plain(1)).unwrap();
    // A writer holds the lock exclusively, so no reader opens beside it.
    assert_eq!(reader(&temp).err(), Some(Failure::Conflict));
    drop(journal);
    let first = reader(&temp).unwrap();
    let second = reader(&temp).unwrap();
    assert_eq!(
        first.stored(&plain(1).action, None).unwrap(),
        Some(plain(1))
    );
    assert_eq!(writer(&temp).err(), Some(Failure::Conflict));
    drop((first, second));
    writer(&temp).unwrap();
}

#[test]
fn a_reader_refuses_linked_or_shared_auxiliary_files() {
    use std::os::unix::fs::PermissionsExt;
    for name in [
        "plain.sqlite-journal",
        "plain.sqlite-wal",
        "plain.sqlite-shm",
    ] {
        let temp = tempfile::tempdir().unwrap();
        drop(writer(&temp).unwrap());
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"preserve").unwrap();
        let _ = std::fs::remove_file(temp.path().join(name));
        std::os::unix::fs::symlink(&outside, temp.path().join(name)).unwrap();
        assert_eq!(reader(&temp).err(), Some(Failure::StateNotDurable));
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserve");
    }
    let temp = tempfile::tempdir().unwrap();
    drop(writer(&temp).unwrap());
    std::fs::set_permissions(
        temp.path().join("plain.sqlite"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    assert_eq!(reader(&temp).err(), Some(Failure::StateNotDurable));
}

#[test]
fn the_receipt_cap_refuses_the_next_receipt_and_an_overfull_journal() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = writer(&temp).unwrap();
    {
        let tx = journal.conn.transaction().unwrap();
        for n in 0..MAX_RECEIPTS {
            let receipt = Plain {
                done: true,
                ..plain(n)
            };
            tx.execute(
                "INSERT INTO receipts VALUES(?,?,'done',?)",
                params![
                    receipt.action,
                    receipt.digest,
                    serde_json::to_string(&receipt).unwrap()
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let next = plain(MAX_RECEIPTS);
    assert_eq!(journal.insert(&next), Err(Failure::StateNotDurable));
    assert_eq!(journal.stored(&next.action, None).unwrap(), None);
    drop(journal);
    // At the cap the journal still opens, for lookups and settling.
    let journal = writer(&temp).unwrap();
    let extra = Plain { done: true, ..next };
    journal
        .conn
        .execute(
            "INSERT INTO receipts VALUES(?,?,'done',?)",
            params![
                extra.action,
                extra.digest,
                serde_json::to_string(&extra).unwrap()
            ],
        )
        .unwrap();
    drop(journal);
    assert_eq!(writer(&temp).err(), Some(Failure::StateNotDurable));
}

#[test]
fn only_a_prepared_receipt_holding_its_claims_is_inserted() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = writer(&temp).unwrap();
    let settled = Plain {
        done: true,
        ..plain(1)
    };
    assert_eq!(journal.insert(&settled), Err(Failure::InvalidPlan));
    assert_eq!(journal.stored(&settled.action, None).unwrap(), None);
}

#[test]
fn a_replacement_must_match_the_stored_receipt_and_releases_claims_when_settled() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = writer(&temp).unwrap();
    let prepared = plain(1);
    journal.insert(&prepared).unwrap();
    let done = Plain {
        done: true,
        ..prepared.clone()
    };
    // Refused by the caller's rule, or for another plan: nothing changes.
    assert_eq!(
        journal.replace(&prepared, &done, |_, _| false),
        Err(Failure::Conflict)
    );
    let other = Plain {
        plan: "plan-2".into(),
        ..done.clone()
    };
    assert_eq!(
        journal.replace(&prepared, &other, |_, _| true),
        Err(Failure::Conflict)
    );
    assert_eq!(
        journal.stored(&prepared.action, None).unwrap(),
        Some(prepared.clone())
    );
    // The claim is still held: another receipt for the plan conflicts.
    let rival = Plain {
        action: format!("{:064x}", 99),
        ..prepared.clone()
    };
    assert_eq!(journal.insert(&rival), Err(Failure::Conflict));
    journal.replace(&prepared, &done, |_, _| true).unwrap();
    // A settled receipt cannot be replaced again; its claim is free.
    assert_eq!(
        journal.replace(&done, &done, |_, _| true),
        Err(Failure::Conflict)
    );
    journal.insert(&rival).unwrap();
    drop(journal);
    let journal = writer(&temp).unwrap();
    assert_eq!(journal.stored(&prepared.action, None).unwrap(), Some(done));
}

#[test]
fn a_replacement_whose_commit_fails_keeps_the_prepared_receipt_and_its_claims() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = writer(&temp).unwrap();
    let prepared = plain(1);
    journal.insert(&prepared).unwrap();
    journal
        .conn
        .execute_batch(
            "CREATE TRIGGER fail BEFORE DELETE ON locks BEGIN SELECT RAISE(ABORT,'injected'); END;",
        )
        .unwrap();
    let done = Plain {
        done: true,
        ..prepared.clone()
    };
    assert_eq!(
        journal.replace(&prepared, &done, |_, _| true),
        Err(Failure::StateNotDurable)
    );
    journal.conn.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        journal.stored(&prepared.action, None).unwrap(),
        Some(prepared)
    );
    drop(journal);
    // Reopening checks the claims against the receipt: still held.
    writer(&temp).unwrap();
}

#[test]
fn a_stored_body_over_the_bound_is_refused_before_it_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = writer(&temp).unwrap();
    let prepared = plain(1);
    journal.insert(&prepared).unwrap();
    journal
        .conn
        .execute(
            "UPDATE receipts SET body=? WHERE action=?",
            params!["x".repeat(Plain::BODY_BYTES + 1), prepared.action],
        )
        .unwrap();
    assert_eq!(
        journal.stored(&prepared.action, None),
        Err(Failure::StateNotDurable)
    );
}
