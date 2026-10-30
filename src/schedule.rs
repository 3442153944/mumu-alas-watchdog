//! 重启计划计算（纯函数，可单测）。

use chrono::{DateTime, Duration, Local, TimeZone};

pub fn compute_next_restart(
    now: DateTime<Local>,
    restart_times: &[(u32, u32)],
    interval_hours: f64,
    emulator_start: Option<DateTime<Local>>,
) -> DateTime<Local> {
    if !restart_times.is_empty() {
        for (hour, minute) in restart_times {
            if let Some(candidate) = now
                .date_naive()
                .and_hms_opt(*hour, *minute, 0)
                .and_then(|naive| naive.and_local_timezone(Local).single())
            {
                if candidate > now + Duration::seconds(30) {
                    return candidate;
                }
            }
        }
        let (hour, minute) = restart_times[0];
        let tomorrow = now.date_naive() + Duration::days(1);
        return tomorrow
            .and_hms_opt(hour, minute, 0)
            .and_then(|naive| naive.and_local_timezone(Local).single())
            .unwrap_or(now + Duration::days(1));
    }
    let interval_seconds = (interval_hours.max(0.1) * 3600.0) as i64;
    let base = emulator_start.unwrap_or(now);
    base + Duration::seconds(interval_seconds)
}

/// 启动宽限：若计划时间过近则顺延。返回 (时间, 是否顺延)。
pub fn apply_startup_grace(
    next: DateTime<Local>,
    now: DateTime<Local>,
    grace_minutes: i64,
) -> (DateTime<Local>, bool) {
    let grace = now + Duration::minutes(grace_minutes.max(0));
    if next < grace {
        (grace, true)
    } else {
        (next, false)
    }
}

pub fn format_local(time: DateTime<Local>) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[allow(dead_code)]
fn _unused(_: DateTime<Local>) -> Option<DateTime<Local>> {
    Local.timestamp_opt(0, 0).single()
}
