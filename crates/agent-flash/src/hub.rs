//! Recent journal records held in memory, so `flash watch` can follow along.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use flash_core::journal::Record;

const CAPACITY: usize = 512;

#[derive(Default)]
pub struct Hub {
    inner: Mutex<Inner>,
    arrived: Condvar,
}

#[derive(Default)]
struct Inner {
    /// Sequence number the next record will get.
    next: u64,
    items: VecDeque<(u64, Record)>,
}

impl Hub {
    pub fn publish(&self, record: Record) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let seq = inner.next;
        inner.next += 1;
        if inner.items.len() == CAPACITY {
            inner.items.pop_front();
        }
        inner.items.push_back((seq, record));
        drop(inner);
        self.arrived.notify_all();
    }

    /// Records numbered `since` or later, waiting up to `wait` for the first to
    /// arrive. Returns them with the number to ask for next time.
    pub fn since(&self, since: u64, wait: Duration) -> (Vec<Record>, u64) {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while inner.next <= since {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            inner = self.arrived.wait_timeout(inner, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        let records = inner.items.iter().filter(|(seq, _)| *seq >= since).map(|(_, r)| r.clone()).collect();
        (records, inner.next)
    }

    /// The latest `n` records, for a client that has only just started watching.
    pub fn recent(&self, n: usize) -> (Vec<Record>, u64) {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let skip = inner.items.len().saturating_sub(n);
        (inner.items.iter().skip(skip).map(|(_, r)| r.clone()).collect(), inner.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn record(event: &str) -> Record {
        Record { event: event.into(), ..Record::default() }
    }

    #[test]
    fn readers_resume_where_they_left_off() {
        let hub = Hub::default();
        hub.publish(record("a"));
        hub.publish(record("b"));
        let (records, next) = hub.since(1, Duration::ZERO);
        assert_eq!(records.iter().map(|r| r.event.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(next, 2);
        assert_eq!(hub.recent(1).0[0].event, "b");
    }

    #[test]
    fn a_waiting_reader_wakes_when_a_record_arrives() {
        let hub = Arc::new(Hub::default());
        let publisher = Arc::clone(&hub);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            publisher.publish(record("late"));
        });
        let started = Instant::now();
        let (records, next) = hub.since(0, Duration::from_secs(10));
        assert_eq!(records.len(), 1);
        assert_eq!(next, 1);
        assert!(started.elapsed() < Duration::from_secs(5));
        handle.join().unwrap();
    }

    #[test]
    fn the_buffer_is_bounded() {
        let hub = Hub::default();
        for i in 0..CAPACITY + 10 {
            hub.publish(record(&i.to_string()));
        }
        let (records, next) = hub.since(0, Duration::ZERO);
        assert_eq!(records.len(), CAPACITY);
        assert_eq!(records[0].event, "10");
        assert_eq!(next, (CAPACITY + 10) as u64);
    }
}
