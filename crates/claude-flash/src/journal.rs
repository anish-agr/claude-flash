//! Writing and reading the journal files described in [`flash_core::journal`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use flash_core::journal::{self, Record};
use flash_core::time;

const DAY_MS: u64 = 86_400_000;

pub struct JournalWriter {
    dir: PathBuf,
    retain_days: u32,
    open: Option<(String, File)>,
}

impl JournalWriter {
    pub fn new(dir: PathBuf, retain_days: u32) -> Self {
        JournalWriter { dir, retain_days, open: None }
    }

    pub fn set_retention(&mut self, days: u32) {
        self.retain_days = days;
    }

    pub fn append(&mut self, record: &Record, unix_ms: u64, utc_offset_min: i32) -> io::Result<()> {
        let name = journal::file_name(unix_ms, utc_offset_min);
        if self.open.as_ref().is_none_or(|(open, _)| *open != name) {
            fs::create_dir_all(&self.dir)?;
            let file = OpenOptions::new().create(true).append(true).open(self.dir.join(&name))?;
            self.open = Some((name, file));
            self.prune(unix_ms, utc_offset_min);
        }
        let (_, file) = self.open.as_mut().expect("opened above");
        file.write_all(record.to_line().as_bytes())
    }

    /// Deletes day files older than the retention period. Zero keeps everything.
    fn prune(&self, unix_ms: u64, utc_offset_min: i32) {
        if self.retain_days == 0 {
            return;
        }
        let cutoff = time::local_date(unix_ms.saturating_sub(u64::from(self.retain_days) * DAY_MS), utc_offset_min);
        for (date, path) in day_files(&self.dir) {
            if date < cutoff {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Journal files in `dir`, oldest first, with the local date each is named for.
fn day_files(dir: &Path) -> Vec<((i64, u32, u32), PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            Some((journal::parse_file_name(name.to_str()?)?, entry.path()))
        })
        .collect();
    files.sort();
    files
}

/// Every record written at or after `since_unix_ms`, oldest first.
pub fn read(dir: &Path, since_unix_ms: u64) -> Vec<Record> {
    // Files are named for local dates, and the offset can differ between records;
    // starting two days early covers every time zone.
    let floor = time::local_date(since_unix_ms.saturating_sub(2 * DAY_MS), 0);
    let mut records: Vec<Record> = Vec::new();
    for (date, path) in day_files(dir) {
        if date < floor {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        records.extend(
            text.lines().filter_map(Record::parse_line).filter(|r| r.unix_ms().is_some_and(|t| t >= since_unix_ms)),
        );
    }
    records.sort_by_key(Record::unix_ms);
    records
}

/// Total size of the journal on disk, in bytes.
pub fn size(dir: &Path) -> u64 {
    day_files(dir).iter().filter_map(|(_, path)| fs::metadata(path).ok()).map(|m| m.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flash_core::event::Attention;

    const NOON: u64 = 1_789_041_600_000; // 2026-09-10T12:00:00Z

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-flash-{}", std::process::id())).join("journal").join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn record(unix_ms: u64, event: &str) -> Record {
        Record { ts: time::rfc3339(unix_ms), event: event.into(), kind: Some(Attention::Done), ..Record::default() }
    }

    #[test]
    fn records_land_in_one_file_per_local_day() {
        let dir = scratch("days");
        let mut writer = JournalWriter::new(dir.clone(), 30);
        writer.append(&record(NOON, "Stop"), NOON, 0).unwrap();
        writer.append(&record(NOON + DAY_MS, "Stop"), NOON + DAY_MS, 0).unwrap();
        // 23:30 in UTC-7 on the 10th is 06:30 UTC on the 11th.
        let late = NOON + 18 * 3_600_000 + 30 * 60_000;
        writer.append(&record(late, "late"), late, -420).unwrap();
        let names: Vec<String> =
            day_files(&dir).iter().map(|(_, p)| p.file_name().unwrap().to_string_lossy().into()).collect();
        assert_eq!(names, ["2026-09-10.jsonl", "2026-09-11.jsonl"]);
        assert_eq!(fs::read_to_string(dir.join("2026-09-10.jsonl")).unwrap().lines().count(), 2);
    }

    #[test]
    fn reading_filters_by_time_and_skips_damage() {
        let dir = scratch("read");
        let mut writer = JournalWriter::new(dir.clone(), 30);
        for i in 0..5 {
            writer.append(&record(NOON + i * 60_000, &format!("e{i}")), NOON + i * 60_000, 0).unwrap();
        }
        let mut file = OpenOptions::new().append(true).open(dir.join("2026-09-10.jsonl")).unwrap();
        file.write_all(b"{\"truncated\n").unwrap();
        let events: Vec<String> = read(&dir, NOON + 3 * 60_000).into_iter().map(|r| r.event).collect();
        assert_eq!(events, ["e3", "e4"]);
    }

    #[test]
    fn old_days_are_pruned_when_a_new_day_starts() {
        let dir = scratch("prune");
        let mut writer = JournalWriter::new(dir.clone(), 7);
        writer.append(&record(NOON - 10 * DAY_MS, "old"), NOON - 10 * DAY_MS, 0).unwrap();
        writer.append(&record(NOON - 3 * DAY_MS, "recent"), NOON - 3 * DAY_MS, 0).unwrap();
        writer.append(&record(NOON, "today"), NOON, 0).unwrap();
        let events: Vec<String> = read(&dir, 0).into_iter().map(|r| r.event).collect();
        assert_eq!(events, ["recent", "today"]);
    }
}
