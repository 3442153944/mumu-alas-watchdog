use chrono::{Datelike, Local, TimeZone, Timelike};
use mumu_alas_watchdog_rs::schedule::{apply_startup_grace, compute_next_restart};
use mumu_alas_watchdog_rs::util::{fmt_duration, parse_hhmm_list, path_is_under};

#[test]
fn fixed_times_pick_next_future_slot() {
    let now = Local.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    let next = compute_next_restart(now, &[(4, 0), (16, 0)], 6.0, None);
    assert_eq!(next.hour(), 16);
    assert_eq!(next.day(), 1);
}

#[test]
fn fixed_times_roll_to_tomorrow() {
    let now = Local.with_ymd_and_hms(2026, 1, 1, 20, 0, 0).unwrap();
    let next = compute_next_restart(now, &[(4, 0), (16, 0)], 6.0, None);
    assert_eq!(next.day(), 2);
    assert_eq!(next.hour(), 4);
}

#[test]
fn interval_based_on_emulator_start() {
    let now = Local.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
    let started = now - chrono::Duration::hours(2);
    let next = compute_next_restart(now, &[], 6.0, Some(started));
    assert_eq!(next, now + chrono::Duration::hours(4));
}

#[test]
fn startup_grace_pushes_imminent_restart() {
    let now = Local.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
    let (next, graced) = apply_startup_grace(now + chrono::Duration::minutes(1), now, 10);
    assert!(graced);
    assert_eq!(next, now + chrono::Duration::minutes(10));

    let later = now + chrono::Duration::hours(2);
    let (next, graced) = apply_startup_grace(later, now, 10);
    assert!(!graced);
    assert_eq!(next, later);
}

#[test]
fn hhmm_parser_filters_invalid() {
    let value: serde_yaml::Value = serde_yaml::from_str(r#"["04:00", "16:30", "bad", "25:00"]"#).unwrap();
    assert_eq!(parse_hhmm_list(&value), vec![(4, 0), (16, 30)]);

    let value: serde_yaml::Value = serde_yaml::from_str(r#""23:59, 00:00""#).unwrap();
    assert_eq!(parse_hhmm_list(&value), vec![(0, 0), (23, 59)]);
}

#[test]
fn duration_formatting() {
    assert_eq!(fmt_duration(0.0), "0秒");
    assert_eq!(fmt_duration(59.0), "59秒");
    assert_eq!(fmt_duration(60.0), "1分0秒");
    assert_eq!(fmt_duration(3661.0), "1小时1分1秒");
}

#[test]
fn path_prefix_rules() {
    let base = std::path::Path::new(r"C:\__muw_path_test__\Alas");
    assert!(path_is_under(
        std::path::Path::new(r"C:\__muw_path_test__\Alas\toolkit\webapp\alas.exe"),
        base
    ));
    assert!(!path_is_under(
        std::path::Path::new(r"C:\__muw_path_test__\Alas2\a.exe"),
        base
    ));
}

fn _unused(_: chrono::DateTime<Local>) {}
