//! Authenticated native message acknowledgements measure the real stream RTT.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
#[derive(Default)]
pub(super) struct Latency {
    sent: VecDeque<(u32, Instant)>,
    ack: u32,
    smoothed: Option<u32>,
    published: Option<(Instant, i32)>,
}
impl Latency {
    pub fn sent(&mut self, sequence: u32, now: Instant) {
        self.sent.push_back((sequence, now));
        while self.sent.len() > 128 {
            self.sent.pop_front();
        }
    }
    pub fn acknowledge(&mut self, sequence: u32, now: Instant) {
        if sequence <= self.ack {
            return;
        }
        self.ack = sequence;
        let sample = self
            .sent
            .iter()
            .rev()
            .find(|(seq, _)| *seq <= sequence)
            .map(|(_, sent)| *sent);
        self.sent.retain(|(seq, _)| *seq > sequence);
        if let Some(sent) = sample
            && let Some(age) = now.checked_duration_since(sent)
        {
            let millis = age.as_millis().clamp(1, 9999) as u32;
            self.smoothed = Some(self.smoothed.map_or(millis, |old| (old * 7 + millis) / 8));
        }
    }
    pub fn ping(&self) -> i32 {
        self.smoothed.map_or(-1, |v| v as i32)
    }
    pub fn publish(&mut self, now: Instant) -> Option<i32> {
        let ping = self.ping();
        if ping < 0
            || self.published.is_some_and(|(last, value)| {
                value == ping || now.duration_since(last) < Duration::from_secs(1)
            })
        {
            return None;
        }
        self.published = Some((now, ping));
        Some(ping)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_ack_rtt_ignores_repeated_acks_and_limits_publication_rate() {
        let mut rtt = Latency::default();
        let now = Instant::now();
        assert_eq!(rtt.ping(), -1);
        rtt.sent(10, now);
        rtt.acknowledge(10, now + Duration::from_millis(80));
        assert_eq!(rtt.ping(), 80);
        assert_eq!(rtt.publish(now + Duration::from_millis(80)), Some(80));
        rtt.acknowledge(10, now + Duration::from_secs(2));
        assert_eq!(rtt.ping(), 80);
        rtt.sent(11, now + Duration::from_millis(100));
        rtt.acknowledge(11, now + Duration::from_millis(260));
        assert_eq!(rtt.ping(), 90);
        assert_eq!(rtt.publish(now + Duration::from_millis(260)), None);
        assert_eq!(rtt.publish(now + Duration::from_secs(2)), Some(90));
    }
    #[test]
    fn connecting_and_missing_history_have_no_fabricated_sample_and_history_is_bounded() {
        let mut rtt = Latency::default();
        let now = Instant::now();
        rtt.acknowledge(1, now);
        assert_eq!(rtt.ping(), -1);
        for seq in 2..200 {
            rtt.sent(seq, now);
        }
        assert_eq!(rtt.sent.len(), 128);
        rtt.acknowledge(2, now + Duration::from_secs(1));
        assert_eq!(rtt.ping(), -1);
        rtt.acknowledge(199, now + Duration::from_millis(35));
        assert_eq!(rtt.ping(), 35);
        assert!(rtt.sent.is_empty());
    }
}
