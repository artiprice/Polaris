// Unit tests of crates/polaris-log/src/file.rs (`tests`); compiled into that module via #[path].
use super::{DailySizeSink, utc_day};
use crate::init::LogConfig;
use crate::ring::lock;
use std::io::Write as _;

#[test]
fn daily_rollover_archives_and_reopens_the_active_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut config = LogConfig::new(directory.path());
    config.keep_files = 3;
    let sink = DailySizeSink::new(&config)?;
    {
        let mut state = lock(&sink.state);
        if let Some(file) = state.file.as_mut() {
            file.write_all(b"previous day\n")?;
            state.size = 13;
            state.utc_day = utc_day().saturating_sub(1);
        }
        sink.rotate(&mut state)?;
    }
    assert_eq!(
        std::fs::read(directory.path().join("polaris_1.log"))?,
        b"previous day\n"
    );
    assert_eq!(
        std::fs::metadata(directory.path().join("polaris.log"))?.len(),
        0
    );
    Ok(())
}
