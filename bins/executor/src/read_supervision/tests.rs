use super::*;
use std::io::Cursor;
fn frame(status: u8, body: &[u8]) -> Capture {
    let mut bytes = vec![status];
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(body);
    Capture { bytes, eof: true }
}
#[test]
fn whole_json_and_exact_body_bound_are_required() {
    let mut body = vec![b'x'; REPORT];
    body[0] = b'"';
    body[REPORT - 1] = b'"';
    assert_eq!(
        frame(0, &body)
            .finish(false)
            .unwrap()
            .result()
            .unwrap()
            .as_bytes()
            .len(),
        REPORT
    );
    for body in [b"".as_slice(), b"{broken", b"1 2"] {
        assert!(matches!(
            frame(0, body).finish(false),
            Err(Refusal::InvalidOutput)
        ));
    }
    let mut incomplete = frame(0, b"{}");
    incomplete.eof = false;
    assert!(matches!(
        incomplete.finish(false),
        Err(Refusal::InvalidOutput)
    ));
}
#[test]
fn worker_status_cannot_forge_a_guardian_cleanup_refusal() {
    assert!(matches!(
        frame(4, b"").finish(true),
        Err(Refusal::InvalidOutput)
    ));
    assert!(matches!(
        frame(4, b"").finish(false).unwrap().result(),
        Err(Refusal::InvalidOutput)
    ));
    for status in [7, 255] {
        assert!(matches!(
            frame(status, b"").finish(false),
            Err(Refusal::InvalidOutput)
        ));
    }
    assert!(matches!(
        frame(1, b"{}").finish(false),
        Err(Refusal::InvalidOutput)
    ));
}
#[test]
fn oversized_declared_or_actual_bytes_refuse_before_growth() {
    let mut capture = Capture::new().unwrap();
    let capacity = capture.bytes.capacity();
    assert!(matches!(
        capture.pump(&mut Cursor::new([0, 0, 4, 0, 1])),
        Err(Refusal::InvalidOutput)
    ));
    assert_eq!(capture.bytes.capacity(), capacity);
    let mut capture = Capture::new().unwrap();
    let bytes = vec![b'x'; REPORT + HEADER + 1];
    assert!(matches!(
        capture.pump(&mut Cursor::new(bytes)),
        Err(Refusal::InvalidOutput)
    ));
    assert!(capture.bytes.len() <= REPORT + HEADER);
}
#[test]
fn partial_and_trailing_frames_cannot_be_accepted() {
    let mut capture = frame(0, b"{}");
    capture.bytes.pop();
    assert!(matches!(capture.finish(true), Err(Refusal::InvalidOutput)));
    let mut capture = frame(0, b"{}");
    capture.bytes.push(0);
    assert!(matches!(capture.finish(true), Err(Refusal::InvalidOutput)));
}

#[test]
fn unconfirmed_admission_drop_poisons_before_releasing_the_guard() {
    for confirmed in [false, true] {
        let manager = Supervisor::default();
        let admission = Admission {
            _flight: manager.flight.try_acquire().unwrap(),
            poisoned: manager.poisoned.clone(),
            launched: true,
            confirmed,
        };
        drop(admission);
        assert_eq!(manager.poisoned.load(Ordering::Acquire), !confirmed);
        assert!(manager.flight.try_acquire().is_ok());
    }
}

#[test]
fn queued_reports_cannot_pass_after_ticket_expiry_or_cancellation() {
    for expired in [true, false] {
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(
            sender
                .try_send(Ok(SupervisedReport(b"{}".to_vec())))
                .is_ok()
        );
        let ticket = Ticket {
            receiver,
            cancelled: Arc::new(AtomicBool::new(!expired)),
            started: if expired {
                Instant::now() - WINDOW
            } else {
                Instant::now()
            },
        };
        assert!(matches!(
            ticket.wait(),
            Err(Refusal::Deadline | Refusal::Cancelled)
        ));
    }
}

#[test]
fn preparation_deadline_reaches_the_monitor_without_a_new_window() {
    // This owned sleeper deliberately supplies no guardian acknowledgment.
    // Its later exit must not replace the original, shorter preparation deadline.
    let manager = Supervisor::default();
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    let mut command = Command::new("/usr/bin/sleep");
    command.arg("0.5");
    let deadline = Instant::now() + Duration::from_millis(100);
    let ticket = manager.start_held(command, deadline, lease).unwrap();
    assert_eq!(ticket.started + WINDOW, deadline);
    assert!(matches!(ticket.wait(), Err(Refusal::Deadline)));
    // Leave no live fixture or leased image after the unconfirmed exit.
    let cleanup = Instant::now() + WINDOW;
    while weak.upgrade().is_some() && Instant::now() < cleanup {
        thread::sleep(TICK);
    }
    assert!(weak.upgrade().is_none());
}
