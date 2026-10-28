use polaris::stats_data::*;
fn fixture() -> Dataset {
    let start = civil_day(2024, 2, 27);
    Dataset {
        start,
        end: start + 4,
        models: vec![
            Model {
                id: "a".into(),
                name: "A".into(),
            },
            Model {
                id: "b".into(),
                name: "B".into(),
            },
        ],
        sessions: vec![
            Session {
                id: 1,
                day: start,
                seconds: Some(10),
                usage: vec![Usage {
                    model: "a".into(),
                    tokens: Tokens::known(10, 20, 30, 40),
                }],
            },
            Session {
                id: 2,
                day: start + 1,
                seconds: Some(20),
                usage: vec![Usage {
                    model: "b".into(),
                    tokens: Tokens::known(100, 200, 300, 400),
                }],
            },
            Session {
                id: 3,
                day: start + 1,
                seconds: Some(30),
                usage: vec![Usage {
                    model: "a".into(),
                    tokens: Tokens::known(1, 2, 3, 4),
                }],
            },
            Session {
                id: 4,
                day: start + 3,
                seconds: Some(40),
                usage: vec![Usage {
                    model: "a".into(),
                    tokens: Tokens::known(5, 5, 5, 5),
                }],
            },
            Session {
                id: 5,
                day: start + 4,
                seconds: Some(50),
                usage: vec![Usage {
                    model: "b".into(),
                    tokens: Tokens::known(0, 0, 0, 0),
                }],
            },
        ],
    }
}
#[test]
fn dates_cover_leap_years_weekdays_and_exact_roundtrips() {
    for (y, m, d) in [
        (1970, 1, 1),
        (2000, 2, 29),
        (2024, 2, 29),
        (2025, 12, 31),
        (2026, 1, 1),
        (2100, 3, 1),
    ] {
        assert_eq!(civil(civil_day(y, m, d)), (y, m, d));
    }
    assert_eq!(weekday(civil_day(2025, 10, 19)), 6);
    assert_eq!(weekday(civil_day(2025, 10, 20)), 0);
    assert_eq!(civil(civil_day(2024, 2, 28) + 1), (2024, 2, 29));
    assert_eq!(civil(civil_day(2100, 2, 28) + 1), (2100, 3, 1));
}
#[test]
fn categories_summaries_models_buckets_and_streaks_reconcile() {
    let d = fixture();
    let s = d.summary(d.start, d.end);
    assert_eq!(s.tokens.total(), Some(1130));
    assert_eq!(s.sessions, 5);
    assert_eq!(s.active_days, 4);
    assert_eq!(s.seconds, Some(150));
    assert_eq!(s.longest_session, Some(50));
    assert_eq!(s.current_streak, 2);
    assert_eq!(s.longest_streak, 2);
    assert_eq!(s.busiest, Some((d.start + 1, 2)));
    assert_eq!(s.models[0].id, "b");
    assert_eq!(
        s.models
            .iter()
            .map(|m| m.tokens.total().unwrap())
            .sum::<u64>(),
        1130
    );
    for weekly in [true, false] {
        let buckets = d.buckets(d.start, d.end, weekly, &chart_models(&s));
        assert_eq!(
            buckets
                .iter()
                .flat_map(|b| b.models.iter())
                .map(|n| n.unwrap())
                .sum::<u64>(),
            1130
        );
        assert_eq!(buckets.first().unwrap().start, d.start);
        assert_eq!(buckets.last().unwrap().end, d.end);
        for pair in buckets.windows(2) {
            assert_eq!(pair[0].end + 1, pair[1].start);
        }
    }
}
#[test]
fn unknown_is_not_zero_large_values_do_not_overflow_and_overflow_is_unavailable() {
    let mut t = Tokens::known(5_000_000_000, 8_000_000_000, 9_000_000_000, 1);
    assert_eq!(t.total(), Some(22_000_000_001));
    t.output = None;
    assert_eq!(t.total(), None);
    let mut t = Tokens::known(u64::MAX, 0, 0, 0);
    t.add(Tokens::known(1, 0, 0, 0));
    assert_eq!(t.total(), None);
    let mut d = fixture();
    d.sessions[0].usage[0].tokens.input = None;
    d.sessions[0].seconds = None;
    let s = d.summary(d.start, d.end);
    assert_eq!(s.tokens.total(), None);
    assert_eq!(s.seconds, None);
    assert_eq!(s.longest_session, None);
    let empty = d.summary(d.end, d.end);
    assert_eq!(empty.tokens.total(), Some(0));
    assert_eq!(empty.sessions, 1);
}
#[test]
fn more_than_five_models_fold_into_other_without_double_counting() {
    let mut d = fixture();
    d.models.clear();
    d.sessions.clear();
    for i in 0..9 {
        d.models.push(Model {
            id: i.to_string(),
            name: format!("Model {i}"),
        });
        d.sessions.push(Session {
            id: i,
            day: d.start,
            seconds: Some(1),
            usage: vec![Usage {
                model: i.to_string(),
                tokens: Tokens::known(i + 1, 0, 0, 0),
            }],
        });
    }
    let s = d.summary(d.start, d.end);
    let m = chart_models(&s);
    assert_eq!(m.len(), 6);
    assert_eq!(m.last().unwrap().name, "Other");
    assert_eq!(m.iter().map(|m| m.tokens.total().unwrap()).sum::<u64>(), 45);
    let b = d.buckets(d.start, d.end, false, &m);
    assert_eq!(b[0].models.iter().map(|n| n.unwrap()).sum::<u64>(), 45);
}
#[test]
fn demo_is_deterministic_varied_and_every_filter_matches_source_records() {
    let d = demo();
    for (start, end) in [(d.start, d.end), (d.end - 6, d.end), (d.end - 29, d.end)] {
        let s = d.summary(start, end);
        let expected = d
            .sessions
            .iter()
            .filter(|s| s.day >= start && s.day <= end)
            .flat_map(|s| &s.usage)
            .map(|u| u.tokens.total().unwrap())
            .sum::<u64>();
        assert_eq!(s.tokens.total(), Some(expected));
        assert!(s.models.iter().all(|m| m.tokens.total().unwrap() > 0));
        let b = d.buckets(start, end, true, &chart_models(&s));
        assert_eq!(
            b.iter()
                .flat_map(|b| &b.models)
                .map(|n| n.unwrap())
                .sum::<u64>(),
            expected
        );
    }
    let a = d.summary(d.start, d.start + 29);
    let b = d.summary(d.end - 29, d.end);
    let ap = a
        .models
        .iter()
        .find(|m| m.id == "super")
        .unwrap()
        .tokens
        .total()
        .unwrap() as f64
        / a.tokens.total().unwrap() as f64;
    let bp = b
        .models
        .iter()
        .find(|m| m.id == "super")
        .unwrap()
        .tokens
        .total()
        .unwrap() as f64
        / b.tokens.total().unwrap() as f64;
    assert!((ap - bp).abs() > 0.01);
    assert_eq!(d.end - d.start + 1, 365);
}

#[test]
fn duplicate_sessions_and_unknown_model_ids_do_not_break_reconciliation() {
    let mut d = fixture();
    d.sessions.push(d.sessions[0].clone());
    let s = d.summary(d.start, d.end);
    assert_eq!(s.sessions, 5);
    assert_eq!(s.tokens.total(), Some(1130));
    d.sessions[0].usage.push(Usage {
        model: "unlisted".into(),
        tokens: Tokens::known(100, 0, 0, 0),
    });
    let s = d.summary(d.start, d.end);
    assert_eq!(s.tokens.total(), Some(1230));
    assert!(s.models.iter().any(|m| m.id == "unlisted"));
    assert_eq!(
        s.models
            .iter()
            .map(|m| m.tokens.total().unwrap())
            .sum::<u64>(),
        1230
    );
}
