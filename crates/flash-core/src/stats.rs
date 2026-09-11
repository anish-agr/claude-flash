//! Summaries of the journal: what needed attention, and how long Claude waited.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::event::Attention;
use crate::journal::{Channel, Reason, Record};
use crate::time;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KindCount {
    pub total: u32,
    /// Raised and delivered through at least one channel.
    pub delivered: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Waits {
    pub count: u32,
    pub total_ms: u64,
    pub median_ms: u64,
    pub p90_ms: u64,
    pub max_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Project {
    pub name: String,
    pub signals: u32,
    pub waited_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub records: u32,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub sessions: u32,
    pub prompts: u32,
    pub signals: BTreeMap<Attention, KindCount>,
    pub channels: BTreeMap<Channel, u32>,
    pub suppressed: BTreeMap<Reason, u32>,
    /// Time Claude spent blocked on a question or an approval.
    pub waits: Waits,
    pub projects: Vec<Project>,
    /// Signals by local hour of day.
    pub hours: [u32; 24],
}

pub fn summarize<'a>(records: impl IntoIterator<Item = &'a Record>) -> Summary {
    let mut s = Summary {
        records: 0,
        first_ts: None,
        last_ts: None,
        sessions: 0,
        prompts: 0,
        signals: BTreeMap::new(),
        channels: BTreeMap::new(),
        suppressed: BTreeMap::new(),
        waits: Waits::default(),
        projects: Vec::new(),
        hours: [0; 24],
    };
    let mut sessions = HashSet::new();
    let mut waits = Vec::new();
    let mut projects: HashMap<String, (u32, u64)> = HashMap::new();
    let (mut first, mut last): (Option<u64>, Option<u64>) = (None, None);

    for r in records {
        s.records += 1;
        if let Some(t) = r.unix_ms() {
            first = Some(first.map_or(t, |f| f.min(t)));
            last = Some(last.map_or(t, |l| l.max(t)));
        }
        if let Some(id) = &r.session {
            sessions.insert(id.clone());
        }
        if r.event == "UserPromptSubmit" {
            s.prompts += 1;
        }
        for channel in &r.delivered {
            *s.channels.entry(*channel).or_default() += 1;
        }
        if let Some(reason) = r.suppressed {
            *s.suppressed.entry(reason).or_default() += 1;
        }
        let project = r.project.clone().unwrap_or_else(|| "unknown".to_owned());
        if let Some(ms) = r.waited_ms {
            waits.push(ms);
            projects.entry(project).or_default().1 += ms;
        } else if let Some(kind) = r.kind
            && r.event != "test"
        {
            let count = s.signals.entry(kind).or_default();
            count.total += 1;
            count.delivered += u32::from(!r.delivered.is_empty());
            projects.entry(project).or_default().0 += 1;
            if let Some(t) = r.unix_ms() {
                s.hours[time::local_hour(t, r.tz) as usize] += 1;
            }
        }
    }

    s.sessions = sessions.len() as u32;
    s.first_ts = first.map(time::rfc3339);
    s.last_ts = last.map(time::rfc3339);
    waits.sort_unstable();
    s.waits = Waits {
        count: waits.len() as u32,
        total_ms: waits.iter().sum(),
        median_ms: percentile(&waits, 50.0),
        p90_ms: percentile(&waits, 90.0),
        max_ms: waits.last().copied().unwrap_or(0),
    };
    s.projects =
        projects.into_iter().map(|(name, (signals, waited_ms))| Project { name, signals, waited_ms }).collect();
    s.projects.sort_by(|a, b| b.signals.cmp(&a.signals).then(b.waited_ms.cmp(&a.waited_ms)).then(a.name.cmp(&b.name)));
    s
}

/// Nearest-rank percentile of already sorted values.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: &str, event: &str, kind: Option<Attention>, project: &str) -> Record {
        Record {
            ts: ts.into(),
            event: event.into(),
            kind,
            session: Some("s1".into()),
            project: Some(project.into()),
            ..Record::default()
        }
    }

    #[test]
    fn counts_signals_prompts_and_channels() {
        let mut shown = rec("2026-09-10T10:00:00Z", "Stop", Some(Attention::Done), "app");
        shown.delivered = vec![Channel::Flash];
        let mut hidden = rec("2026-09-10T11:00:00Z", "PermissionRequest", Some(Attention::Approval), "app");
        hidden.suppressed = Some(Reason::Paused);
        let prompt = rec("2026-09-10T09:59:00Z", "UserPromptSubmit", None, "app");
        let test = rec("2026-09-10T12:00:00Z", "test", Some(Attention::Done), "app");
        let s = summarize([&shown, &hidden, &prompt, &test]);
        assert_eq!(s.records, 4);
        assert_eq!(s.prompts, 1);
        assert_eq!(s.sessions, 1);
        assert_eq!(s.signals[&Attention::Done], KindCount { total: 1, delivered: 1 });
        assert_eq!(s.signals[&Attention::Approval], KindCount { total: 1, delivered: 0 });
        assert_eq!(s.channels[&Channel::Flash], 1);
        assert_eq!(s.suppressed[&Reason::Paused], 1);
        assert_eq!(s.first_ts.as_deref(), Some("2026-09-10T09:59:00.000Z"));
        assert_eq!(s.last_ts.as_deref(), Some("2026-09-10T12:00:00.000Z"));
    }

    #[test]
    fn wait_percentiles_use_nearest_rank() {
        let records: Vec<Record> = [1_000, 2_000, 3_000, 4_000, 60_000]
            .iter()
            .map(|&ms| Record {
                waited_ms: Some(ms),
                kind: Some(Attention::Approval),
                ..rec("2026-09-10T10:00:00Z", "PostToolUse", None, "app")
            })
            .collect();
        let s = summarize(&records);
        assert_eq!(s.waits, Waits { count: 5, total_ms: 70_000, median_ms: 3_000, p90_ms: 60_000, max_ms: 60_000 });
        assert!(s.signals.is_empty(), "resolutions are not signals");
    }

    #[test]
    fn projects_rank_by_signals_then_wait() {
        let s = summarize(&[
            rec("2026-09-10T10:00:00Z", "Stop", Some(Attention::Done), "b"),
            rec("2026-09-10T10:00:00Z", "Stop", Some(Attention::Done), "a"),
            rec("2026-09-10T10:00:00Z", "Stop", Some(Attention::Done), "a"),
        ]);
        assert_eq!(s.projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn hours_use_the_recorded_offset() {
        let mut r = rec("2026-09-10T02:30:00Z", "Stop", Some(Attention::Done), "app");
        r.tz = -420;
        assert_eq!(summarize([&r]).hours[19], 1);
    }

    #[test]
    fn empty_input_is_all_zero() {
        let s = summarize(std::iter::empty());
        assert_eq!((s.records, s.waits.count, s.waits.median_ms), (0, 0, 0));
        assert_eq!(s.first_ts, None);
    }
}
