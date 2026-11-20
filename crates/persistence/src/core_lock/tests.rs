use super::*;
use std::{
    os::unix::fs::MetadataExt,
    process::{Child, Command, Stdio},
};

const OPEN_PATH: &str = "LIMEOS_CORE_LOCK_OPEN_PATH";
const HOLD: &str = "LIMEOS_CORE_LOCK_HOLD";
const REFUSED: i32 = 10;
const OPENED: i32 = 11;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn helper(name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", &format!("core_lock::tests::{name}")]);
    command
}
/// Another process's view: the exit status says whether Store::open was refused.
fn open_in_another_process(path: &Path) -> i32 {
    helper("open_helper")
        .env(OPEN_PATH, path)
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .code()
        .unwrap()
}
fn identity(path: &Path) -> (u64, u64) {
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

#[test]
fn open_helper() {
    let Some(path) = std::env::var_os(OPEN_PATH) else {
        return;
    };
    std::process::exit(match Store::open(Path::new(&path)) {
        Err(Error(ErrorCode::Conflict)) => REFUSED,
        Ok(_) => OPENED,
        Err(_) => 12,
    });
}
#[test]
fn hold_helper() {
    if std::env::var_os(HOLD).is_none() {
        return;
    }
    // Stand-in for a child that still shares the Store's lock description.
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn a_child_sharing_the_lock_description_does_not_block_a_reopen_after_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let lock = path.with_extension("lock");
    let store = Store::open(&path).unwrap();
    // A child spawned while the Store is open shares its lock description until
    // it execs. This one keeps it as stdin for its whole life, so the window never
    // closes and no timing is involved.
    let mut command = helper("hold_helper");
    command
        .env(HOLD, "1")
        .stdin(store._lock.0.try_clone().unwrap())
        .stdout(Stdio::null());
    let child = ChildGuard(command.spawn().unwrap());
    // Close the parent's duplicate now: from here the parent's only descriptor
    // for the lock file is the Store's own.
    drop(command);
    drop(store);
    // The child is alive and still holds the description at the reopen.
    let stdin = PathBuf::from(format!("/proc/{}/fd/0", child.0.id()));
    assert_eq!(identity(&stdin), identity(&lock));
    assert!(Store::open(&path).is_ok());
    assert_eq!(identity(&stdin), identity(&lock));
}
#[test]
fn another_process_is_refused_while_a_store_is_open_and_admitted_after_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let store = Store::open(&path).unwrap();
    assert_eq!(open_in_another_process(&path), REFUSED);
    // A second, independently opened description conflicts within one process too,
    // and closing it does not release the first.
    assert_eq!(Store::open(&path).err().unwrap().0, ErrorCode::Conflict);
    assert_eq!(open_in_another_process(&path), REFUSED);
    drop(store);
    assert_eq!(open_in_another_process(&path), OPENED);
    assert!(Store::open(&path).is_ok());
}
#[test]
fn concurrent_opens_in_one_process_admit_exactly_one_and_keep_the_lock_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    let barrier = std::sync::Barrier::new(8);
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    Store::open(&path)
                })
            })
            .collect();
        // Every opened Store stays alive until all threads have tried.
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(outcomes.iter().filter(|o| o.is_ok()).count(), 1);
    assert!(
        outcomes
            .iter()
            .filter_map(|o| o.as_ref().err())
            .all(|e| e.0 == ErrorCode::Conflict)
    );
    // The refused opens closed their own descriptors; the winner still holds the lock.
    assert_eq!(open_in_another_process(&path), REFUSED);
    drop(outcomes);
    assert_eq!(open_in_another_process(&path), OPENED);
}
