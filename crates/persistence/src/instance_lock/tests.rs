use super::*;
use std::process::{Child, Command, Stdio};

const OPEN_PATH: &str = "LIMEOS_INSTANCE_LOCK_OPEN_PATH";
const HOLD: &str = "LIMEOS_INSTANCE_LOCK_HOLD";
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
    command.args(["--exact", &format!("instance_lock::tests::{name}")]);
    command
}
/// Another process's view: exit status says whether Store::open was refused.
fn open_in_another_process(path: &Path) -> i32 {
    helper("open_helper")
        .env(OPEN_PATH, path)
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .code()
        .unwrap()
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
    // Stand-in for any child that still has a copy of the Store's lock descriptor.
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn a_child_with_a_copy_of_the_lock_descriptor_does_not_block_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let store = Store::open(&path).unwrap();
    // A child spawned while the Store is open shares its lock descriptor until it
    // execs; this one keeps it as stdin for its whole life, so no timing is involved.
    let copy = store._lock.file.as_ref().unwrap().try_clone().unwrap();
    let _child = ChildGuard(
        helper("hold_helper")
            .env(HOLD, "1")
            .stdin(copy)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    drop(store);
    assert!(Store::open(&path).is_ok());
}
#[test]
fn another_process_is_refused_while_a_store_is_open_even_after_an_in_process_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let store = Store::open(&path).unwrap();
    assert_eq!(open_in_another_process(&path), REFUSED);
    // A second Store in this process is refused without opening, and so without
    // releasing, the first one's record lock.
    assert_eq!(Store::open(&path).err().unwrap().0, ErrorCode::Conflict);
    let respelled = dir.path().join(".").join("core.sqlite");
    assert_eq!(
        Store::open(&respelled).err().unwrap().0,
        ErrorCode::Conflict
    );
    assert_eq!(open_in_another_process(&path), REFUSED);
    drop(store);
    assert_eq!(open_in_another_process(&path), OPENED);
    assert!(Store::open(&path).is_ok());
}
#[test]
fn concurrent_opens_in_one_process_admit_exactly_one_store() {
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
}
