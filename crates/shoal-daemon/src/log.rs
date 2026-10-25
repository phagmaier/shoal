//! `tracing` to `logs/shoald.log` (M1 spec §4.6): rotated at the first write of
//! each UTC day to `shoald.log.<date>`, keeping the previous seven days. Also to
//! stderr when that is a terminal, as under `just dev-daemon`.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

use jiff::{Timestamp, ToSpan, civil::Date, tz::TimeZone};
use tracing_subscriber::{filter::LevelFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const NAME: &str = "shoald.log";
/// Rotated files kept, one per day.
const KEEP_DAYS: i64 = 7;

/// Install the global subscriber. The level comes from `SHOAL_LOG`, default
/// `info`; an unrecognized level is reported once logging works.
pub fn init(dir: &Path) -> io::Result<()> {
    let requested = std::env::var("SHOAL_LOG").ok();
    let level = requested
        .as_deref()
        .and_then(|level| level.parse::<LevelFilter>().ok());
    let file = DailyFile::open(dir, today())?;
    let stderr = io::stderr()
        .is_terminal()
        .then(|| fmt::layer().with_writer(io::stderr));
    tracing_subscriber::registry()
        .with(level.unwrap_or(LevelFilter::INFO))
        .with(fmt::layer().with_writer(Mutex::new(file)))
        .with(stderr)
        .init();
    if let (Some(requested), None) = (requested, level) {
        tracing::warn!(requested, "SHOAL_LOG is not a log level; using info");
    }
    Ok(())
}

fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::UTC).date()
}

/// `shoald.log`, renamed to `shoald.log.<date>` when a write lands on a later day.
struct DailyFile {
    dir: PathBuf,
    file: File,
    /// The UTC day the current file's lines belong to.
    day: Date,
}

impl DailyFile {
    /// An existing file keeps the day it was last written, so a daemon started
    /// on a later day rotates it on its first line.
    fn open(dir: &Path, today: Date) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let day = file
            .metadata()?
            .modified()
            .ok()
            .and_then(|modified| Timestamp::try_from(modified).ok())
            .map_or(today, |modified| modified.to_zoned(TimeZone::UTC).date());
        Ok(Self {
            dir: dir.to_owned(),
            file,
            day,
        })
    }

    fn write_on(&mut self, today: Date, buf: &[u8]) -> io::Result<usize> {
        if today > self.day {
            self.rotate(today)?;
        }
        self.file.write(buf)
    }

    fn rotate(&mut self, today: Date) -> io::Result<()> {
        let path = self.dir.join(NAME);
        // Another shoald, such as one that found the daemon already running,
        // may have rotated first; renaming its new file would overwrite ours.
        let ours = self.file.metadata()?;
        let current = fs::metadata(&path).ok();
        if current.is_some_and(|current| (current.dev(), current.ino()) == (ours.dev(), ours.ino()))
        {
            fs::rename(&path, self.dir.join(format!("{NAME}.{}", self.day)))?;
        }
        self.file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.day = today;
        let oldest = today.saturating_sub(KEEP_DAYS.days());
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let name = entry.file_name();
            let expired = name
                .to_str()
                .and_then(|name| name.strip_prefix(NAME)?.strip_prefix('.'))
                .and_then(|date| date.parse::<Date>().ok())
                .is_some_and(|date| date < oldest);
            if expired {
                // Another daemon may have removed it first.
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
}

impl Write for DailyFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_on(today(), buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .into_string()
                    .expect("UTF-8")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn rotates_on_a_new_day_and_keeps_seven_days() {
        let dir = tempfile::tempdir().expect("temp dir");
        for day in 10..=19 {
            fs::write(dir.path().join(format!("{NAME}.2026-10-{day}")), "old\n").expect("old log");
        }
        fs::write(dir.path().join("other.log"), "").expect("unrelated file");
        let mut log = DailyFile::open(dir.path(), date(2026, 10, 20)).expect("open");
        log.day = date(2026, 10, 20);
        log.write_on(date(2026, 10, 20), b"tuesday\n")
            .expect("write");
        log.write_on(date(2026, 10, 21), b"wednesday\n")
            .expect("write");

        let read = |name: &str| fs::read_to_string(dir.path().join(name)).expect("read");
        assert_eq!(read("shoald.log.2026-10-20"), "tuesday\n");
        assert_eq!(read(NAME), "wednesday\n");
        // Seven days before the 21st is the 14th.
        assert_eq!(
            names(dir.path()),
            [
                "other.log",
                "shoald.log",
                "shoald.log.2026-10-14",
                "shoald.log.2026-10-15",
                "shoald.log.2026-10-16",
                "shoald.log.2026-10-17",
                "shoald.log.2026-10-18",
                "shoald.log.2026-10-19",
                "shoald.log.2026-10-20",
            ]
        );
    }

    #[test]
    fn a_file_from_an_earlier_day_rotates_on_the_first_line() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(NAME), "before\n").expect("old log");
        let written = Timestamp::from_second(1_792_800_000).expect("timestamp"); // 2026-10-24
        File::options()
            .write(true)
            .open(dir.path().join(NAME))
            .and_then(|file| file.set_modified(written.into()))
            .expect("set mtime");
        let mut log = DailyFile::open(dir.path(), date(2026, 10, 25)).expect("open");
        log.write_on(date(2026, 10, 25), b"after\n").expect("write");
        assert_eq!(
            fs::read_to_string(dir.path().join("shoald.log.2026-10-24")).expect("rotated"),
            "before\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(NAME)).expect("current"),
            "after\n"
        );
    }

    #[test]
    fn a_rotation_by_another_process_is_not_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut first = DailyFile::open(dir.path(), date(2026, 10, 24)).expect("open");
        first.day = date(2026, 10, 24);
        first
            .write_on(date(2026, 10, 24), b"first, friday\n")
            .expect("write");
        // A second process starts on Saturday and rotates Friday's file.
        let mut second = DailyFile::open(dir.path(), date(2026, 10, 25)).expect("open");
        second.day = date(2026, 10, 24);
        second
            .write_on(date(2026, 10, 25), b"second, saturday\n")
            .expect("write");
        first
            .write_on(date(2026, 10, 25), b"first, saturday\n")
            .expect("write");

        let read = |name: &str| fs::read_to_string(dir.path().join(name)).expect("read");
        assert_eq!(read("shoald.log.2026-10-24"), "first, friday\n");
        assert_eq!(read(NAME), "second, saturday\nfirst, saturday\n");
    }

    #[test]
    fn the_same_day_appends() {
        let dir = tempfile::tempdir().expect("temp dir");
        let today = today();
        for line in ["one\n", "two\n"] {
            let mut log = DailyFile::open(dir.path(), today).expect("open");
            log.write_on(today, line.as_bytes()).expect("write");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(NAME)).expect("read"),
            "one\ntwo\n"
        );
        assert_eq!(names(dir.path()), [NAME]);
    }
}
