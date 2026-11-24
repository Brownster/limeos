use super::*;

const TTL: i64 = PLAN_TTL_SECONDS;

#[test]
fn bounded_lifetime_is_any_positive_window_up_to_the_ttl() {
    let cases: &[(i64, i64, bool)] = &[
        (0, 1, true),
        (0, TTL, true),
        (100, 100 + TTL, true),
        (100, 101, true),
        (0, TTL + 1, false),
        (100, 100, false),
        (100, 99, false),
        (-1, TTL - 1, false),
        (i64::MAX - 1, i64::MAX, true),
        (i64::MAX, i64::MAX, false),
        (i64::MIN, 0, false),
    ];
    for (created, expires, expected) in cases {
        assert_eq!(
            PlanLifetime::bounded(*created, *expires).is_ok(),
            *expected,
            "{created} {expires}"
        );
    }
}

#[test]
fn exact_lifetime_is_the_ttl_and_nothing_else() {
    let cases: &[(i64, i64, bool)] = &[
        (0, TTL, true),
        (100, 100 + TTL, true),
        (100, 100 + TTL - 1, false),
        (100, 100 + TTL + 1, false),
        (100, 101, false),
        (-1, TTL - 1, false),
        (i64::MAX - TTL, i64::MAX, true),
        (i64::MAX - TTL + 1, i64::MAX, false),
    ];
    for (created, expires, expected) in cases {
        assert_eq!(
            PlanLifetime::exact(*created, *expires).is_ok(),
            *expected,
            "{created} {expires}"
        );
    }
    let issued = PlanLifetime::issue(1_000).unwrap();
    assert_eq!(
        (issued.created_at(), issued.expires_at()),
        (1_000, 1_000 + TTL)
    );
    assert!(PlanLifetime::issue(-1).is_err());
    assert!(PlanLifetime::issue(i64::MAX).is_err());
}

#[test]
fn a_lifetime_is_current_from_creation_until_just_before_expiry() {
    let lifetime = PlanLifetime::bounded(100, 200).unwrap();
    let expired = Err(Error(ErrorCode::Expired));
    assert_eq!(lifetime.current(99), expired);
    assert_eq!(lifetime.current(100), Ok(()));
    assert_eq!(lifetime.current(199), Ok(()));
    assert_eq!(lifetime.current(200), expired);
    assert_eq!(
        PlanLifetime::bounded(100, 99),
        Err(Error(ErrorCode::InvalidInput))
    );
}
