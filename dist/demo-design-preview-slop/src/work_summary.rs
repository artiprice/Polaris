use std::time::Duration;
pub fn elapsed_label(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    if s >= 3600 {
        format!("{}h {:02}m {:02}s", s / 3600, s / 60 % 60, s % 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}
pub fn completed_line(elapsed: Duration) -> String {
    format!("Worked for {} · {}", elapsed_label(elapsed), clock_label())
}
#[cfg(windows)]
fn clock_label() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct LocalTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut LocalTime);
    }
    let mut time = LocalTime::default();
    // SAFETY: GetLocalTime writes the documented eight-u16 SYSTEMTIME layout to a live output.
    unsafe { GetLocalTime(&mut time) };
    format!("{:02}:{:02}", time.hour, time.minute)
}
#[cfg(not(windows))]
fn clock_label() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:02}:{:02} UTC", s / 3600 % 24, s / 60 % 60)
}
