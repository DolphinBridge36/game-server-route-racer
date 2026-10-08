//! Detour's core: route scoring with a switch rule that doesn't flap,
//! multipath de-duplication, the relay allowlist rule and the verdict.
//!
//! No I/O lives here. The app and the relay feed in measurements and packets.

/// Route scoring and the switch rule.
pub mod route {
    /// One route's numbers from the last race.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Stats {
        /// Median round trip, in ms.
        pub ping_ms: f64,
        /// Mean change between consecutive round trips, in ms.
        pub jitter_ms: f64,
        /// Share of probes that never came back, in percent.
        pub loss_pct: f64,
    }

    impl Stats {
        /// From one race's probes, in send order. `None` is a probe that never came back.
        pub fn from_probes(rtts: &[Option<f64>]) -> Option<Stats> {
            let got: Vec<f64> = rtts.iter().flatten().copied().collect();
            if got.is_empty() {
                return None;
            }
            let mut sorted = got.clone();
            sorted.sort_by(f64::total_cmp);
            let mid = sorted.len() / 2;
            let ping_ms = if sorted.len() % 2 == 0 {
                (sorted[mid - 1] + sorted[mid]) / 2.0
            } else {
                sorted[mid]
            };
            let jitter_ms = if got.len() < 2 {
                0.0
            } else {
                got.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (got.len() - 1) as f64
            };
            let loss_pct = (rtts.len() - got.len()) as f64 * 100.0 / rtts.len() as f64;
            Some(Stats {
                ping_ms,
                jitter_ms,
                loss_pct,
            })
        }

        /// One number per route, lower is better, in "ms-like" points.
        /// Jitter counts double, because the game buffers for it. Each percent
        /// of loss counts like 10 ms, because a lost update shows up as a hitch.
        pub fn score(&self) -> f64 {
            self.ping_ms + 2.0 * self.jitter_ms + 10.0 * self.loss_pct
        }
    }

    /// Lag spikes in a run of probes: lost probes, plus round trips more than
    /// `SPIKE_MS` over the median.
    pub fn spikes(rtts: &[Option<f64>]) -> usize {
        let Some(s) = Stats::from_probes(rtts) else {
            return rtts.len();
        };
        rtts.iter()
            .filter(|r| r.is_none_or(|ms| ms > s.ping_ms + SPIKE_MS))
            .count()
    }
    pub const SPIKE_MS: f64 = 25.0;

    /// Picks the route to use after each race. The first race picks the
    /// route. After that, it switches only when another route is clearly
    /// better (by `margin` points) for `rounds` races in a row, so it doesn't
    /// flap mid-round between two routes that are about the same.
    #[derive(Debug, Clone)]
    pub struct Switcher {
        pub margin: f64,
        pub rounds: u32,
        current: Option<String>,
        challenger: Option<(String, u32)>,
    }

    impl Default for Switcher {
        fn default() -> Self {
            Switcher {
                margin: 5.0,
                rounds: 3,
                current: None,
                challenger: None,
            }
        }
    }

    impl Switcher {
        pub fn current(&self) -> Option<&str> {
            self.current.as_deref()
        }

        /// Feeds one race. Routes that lost every probe are left out by the
        /// caller. Returns the new route when it switches.
        pub fn race(&mut self, results: &[(&str, Stats)]) -> Option<String> {
            let (best, best_stats) = results
                .iter()
                .min_by(|a, b| a.1.score().total_cmp(&b.1.score()))?;
            let current = self
                .current
                .as_deref()
                .and_then(|c| results.iter().find(|r| r.0 == c));
            let switch = match current {
                // First race, or the current route went dark: take the best now.
                None => true,
                Some((name, _)) if name == best => {
                    self.challenger = None;
                    false
                }
                Some((_, now)) if now.score() - best_stats.score() >= self.margin => {
                    let n = match &self.challenger {
                        Some((c, n)) if c.as_str() == *best => n + 1,
                        _ => 1,
                    };
                    self.challenger = Some((best.to_string(), n));
                    n >= self.rounds
                }
                Some(_) => {
                    self.challenger = None;
                    false
                }
            };
            if !switch {
                return None;
            }
            self.challenger = None;
            self.current = Some(best.to_string());
            self.current.clone()
        }
    }
}

/// Multipath: every packet goes two ways, and the far end keeps the first copy.
pub mod multipath {
    /// How far back the window remembers sequence numbers.
    pub const WINDOW: u64 = 1024;

    /// Keeps the first copy of each sequence number and drops the late
    /// duplicate, over a sliding window. Anything older than the window is
    /// dropped too: the game has moved on by then.
    #[derive(Debug, Clone)]
    pub struct Dedup {
        top: Option<u64>,
        seen: [u64; (WINDOW / 64) as usize],
    }

    impl Default for Dedup {
        fn default() -> Self {
            Dedup {
                top: None,
                seen: [0; (WINDOW / 64) as usize],
            }
        }
    }

    impl Dedup {
        /// True for the first copy of `seq`, false for a duplicate or a packet too old to matter.
        pub fn accept(&mut self, seq: u64) -> bool {
            let Some(top) = self.top else {
                self.top = Some(seq);
                self.mark(seq, true);
                return true;
            };
            if seq > top {
                if seq - top >= WINDOW {
                    self.seen = [0; (WINDOW / 64) as usize];
                } else {
                    (top + 1..seq).for_each(|s| self.mark(s, false));
                }
                self.top = Some(seq);
                self.mark(seq, true);
                return true;
            }
            if top - seq >= WINDOW || self.has(seq) {
                return false;
            }
            self.mark(seq, true);
            true
        }

        fn slot(seq: u64) -> (usize, u64) {
            let bit = seq % WINDOW;
            ((bit / 64) as usize, 1 << (bit % 64))
        }
        fn has(&self, seq: u64) -> bool {
            let (i, m) = Self::slot(seq);
            self.seen[i] & m != 0
        }
        fn mark(&mut self, seq: u64, on: bool) {
            let (i, m) = Self::slot(seq);
            if on {
                self.seen[i] |= m
            } else {
                self.seen[i] &= !m
            }
        }
    }

    /// What multipath costs: one extra copy of every packet, with Detour's
    /// header and the IP and UDP headers. Bytes per second.
    pub fn extra_bytes_per_sec(packets_per_sec: f64, avg_payload: f64) -> f64 {
        packets_per_sec * (avg_payload + crate::relay::HEADER_LEN as f64 + 28.0)
    }
}

/// The relay's rules and wire format. A relay forwards only UDP, only to the
/// one game server a client registered for that session, and rate-limited.
pub mod relay {
    use std::net::{IpAddr, SocketAddr};

    pub const PORT: u16 = 47820;
    pub const MAGIC: [u8; 2] = *b"DT";
    /// Magic (2), kind (1), session (8), sequence (8).
    pub const HEADER_LEN: usize = 19;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        /// Opens a session. The body is `target=<ip>:<port>`.
        Open = 1,
        /// A game packet. The body is the packet.
        Data = 2,
        /// Echoed back as is. The route race times these.
        Probe = 3,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Header {
        pub kind: Kind,
        pub session: u64,
        pub seq: u64,
    }

    impl Header {
        pub fn encode(&self, out: &mut [u8]) {
            out[..2].copy_from_slice(&MAGIC);
            out[2] = self.kind as u8;
            out[3..11].copy_from_slice(&self.session.to_be_bytes());
            out[11..19].copy_from_slice(&self.seq.to_be_bytes());
        }

        pub fn decode(buf: &[u8]) -> Option<(Header, &[u8])> {
            if buf.len() < HEADER_LEN || buf[..2] != MAGIC {
                return None;
            }
            let kind = match buf[2] {
                1 => Kind::Open,
                2 => Kind::Data,
                3 => Kind::Probe,
                _ => return None,
            };
            let session = u64::from_be_bytes(buf[3..11].try_into().ok()?);
            let seq = u64::from_be_bytes(buf[11..19].try_into().ok()?);
            Some((Header { kind, session, seq }, &buf[HEADER_LEN..]))
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Refusal {
        /// Loopback, private, link-local, multicast and the like. A relay never
        /// sends into its own network.
        NotPublic,
        /// Game servers use high ports. Below 1024 live DNS, mail and the web.
        LowPort,
        TooManySessions,
    }

    /// The allowlist rule for a session's one target.
    pub fn check_target(target: SocketAddr, allow_private: bool) -> Result<(), Refusal> {
        if target.port() < 1024 {
            return Err(Refusal::LowPort);
        }
        if !allow_private && !is_public(target.ip()) {
            return Err(Refusal::NotPublic);
        }
        Ok(())
    }

    pub fn is_public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                !(v4.is_unspecified()
                    || v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_multicast()
                    || v4.is_broadcast()
                    || a == 0
                    || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
                    || a >= 240)
            }
            IpAddr::V6(v6) => {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    return is_public(IpAddr::V4(v4));
                }
                let first = v6.segments()[0];
                !(v6.is_unspecified()
                    || v6.is_loopback()
                    || v6.is_multicast()
                    || (first & 0xfe00) == 0xfc00 // unique local
                    || (first & 0xffc0) == 0xfe80) // link-local
            }
        }
    }

    /// A token bucket over packets and bytes. Time comes in from the caller.
    #[derive(Debug, Clone)]
    pub struct RateLimit {
        pps: f64,
        bps: f64,
        packets: f64,
        bytes: f64,
        last_ms: u64,
    }

    impl RateLimit {
        /// Allows `pps` packets and `kbps` kilobits a second, with up to a
        /// quarter of a second of burst.
        pub fn new(pps: u32, kbps: u32) -> Self {
            let (pps, bps) = (pps as f64, kbps as f64 * 1000.0 / 8.0);
            RateLimit {
                pps,
                bps,
                packets: pps / 4.0,
                bytes: bps / 4.0,
                last_ms: 0,
            }
        }

        pub fn allow(&mut self, now_ms: u64, len: usize) -> bool {
            let dt = now_ms.saturating_sub(self.last_ms) as f64 / 1000.0;
            self.last_ms = self.last_ms.max(now_ms);
            self.packets = (self.packets + dt * self.pps).min(self.pps / 4.0);
            self.bytes = (self.bytes + dt * self.bps).min(self.bps / 4.0);
            if self.packets < 1.0 || self.bytes < len as f64 {
                return false;
            }
            self.packets -= 1.0;
            self.bytes -= len as f64;
            true
        }
    }
}

/// The verdict: one plain sentence on why you lag, and one line of advice.
pub mod verdict {
    use crate::route::Stats;

    /// What Detour measured before racing any path.
    #[derive(Debug, Clone)]
    pub struct Checks {
        /// `None` on a cable.
        pub wifi_ghz: Option<f32>,
        /// Loss and jitter to your own router.
        pub router_loss_pct: f64,
        pub router_jitter_ms: f64,
        /// Extra latency while the line is busy (bufferbloat).
        pub bufferbloat_ms: f64,
        /// An app downloading in the background, by name.
        pub download: Option<String>,
        /// The game runs its own relay network (CS2, Dota 2, Deadlock).
        pub valve_routed: bool,
        pub direct: Stats,
        /// The race winner other than direct, with the city it goes through.
        pub best: Option<(String, Stats)>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Verdict {
        WiFi,
        Bufferbloat,
        Download(String),
        ValveRouted,
        RouteFixed { via: String, saved_ms: i64 },
        RouteSteadier { via: String },
        DirectBest,
    }

    /// Problems at home come first: no path can fix them.
    pub fn verdict(c: &Checks) -> Verdict {
        let shaky_home = c.router_loss_pct >= 1.0 || c.router_jitter_ms >= 15.0;
        if shaky_home && c.wifi_ghz.is_some() {
            return Verdict::WiFi;
        }
        if let Some(app) = &c.download {
            return Verdict::Download(app.clone());
        }
        if c.bufferbloat_ms >= 60.0 {
            return Verdict::Bufferbloat;
        }
        if c.valve_routed {
            return Verdict::ValveRouted;
        }
        match &c.best {
            Some((via, b)) if c.direct.score() - b.score() >= 5.0 => {
                let saved_ms = (c.direct.ping_ms - b.ping_ms).round() as i64;
                if saved_ms >= 3 {
                    Verdict::RouteFixed {
                        via: via.clone(),
                        saved_ms,
                    }
                } else {
                    Verdict::RouteSteadier { via: via.clone() }
                }
            }
            _ => Verdict::DirectBest,
        }
    }

    impl Verdict {
        /// Whether Detour sends the game's traffic down a new path.
        pub fn detour_on(&self) -> bool {
            matches!(
                self,
                Verdict::RouteFixed { .. } | Verdict::RouteSteadier { .. }
            )
        }

        /// The big sentence. Plain words only.
        pub fn headline(&self) -> String {
            match self {
                Verdict::WiFi => "Your Wi-Fi is the problem.".into(),
                Verdict::Download(app) => format!("{app} is filling your connection."),
                Verdict::Bufferbloat => "Your router slows down when the line is busy.".into(),
                Verdict::ValveRouted => "This game already has a fast path.".into(),
                Verdict::RouteFixed { via, .. } => format!("Faster path found through {via}."),
                Verdict::RouteSteadier { via } => format!("Steadier path found through {via}."),
                Verdict::DirectBest => "Your normal path is already the fastest.".into(),
            }
        }

        /// The one line of advice under it.
        pub fn advice(&self) -> String {
            match self {
                Verdict::WiFi => "A faster path won't help. Use a cable or 5 GHz Wi-Fi.".into(),
                Verdict::Download(_) => {
                    "A faster path won't help. Pause the download while you play.".into()
                }
                Verdict::Bufferbloat => {
                    "A faster path won't help. Turn on QoS (sometimes called SQM) in your router."
                        .into()
                }
                Verdict::ValveRouted => {
                    "Valve routes it through its own network. Don't pay anyone for a ping booster."
                        .into()
                }
                Verdict::RouteFixed { saved_ms, .. } => {
                    format!(
                        "Your normal path takes the long way. This one is {saved_ms} ms quicker."
                    )
                }
                Verdict::RouteSteadier { .. } => "Same speed, but no more lost packets.".into(),
                Verdict::DirectBest => "Don't pay anyone for a ping booster.".into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::multipath::*;
    use super::relay::*;
    use super::route::*;
    use super::verdict::*;
    use std::net::SocketAddr;

    // The case this models: Krakow to a Paris server.
    const DIRECT: Stats = Stats {
        ping_ms: 74.0,
        jitter_ms: 18.0,
        loss_pct: 2.1,
    };
    const VIA_FRA: Stats = Stats {
        ping_ms: 46.0,
        jitter_ms: 3.0,
        loss_pct: 0.3,
    };
    const VIA_VIE: Stats = Stats {
        ping_ms: 58.0,
        jitter_ms: 6.0,
        loss_pct: 0.3,
    };

    #[test]
    fn stats_from_probes() {
        let s = Stats::from_probes(&[Some(40.0), Some(50.0), None, Some(44.0)]).unwrap();
        assert_eq!(s.ping_ms, 44.0);
        assert_eq!(s.jitter_ms, 8.0); // |50-40| and |44-50|
        assert_eq!(s.loss_pct, 25.0);
        assert!(Stats::from_probes(&[None, None]).is_none());
    }

    #[test]
    fn spikes_count_losses_and_late_probes() {
        let run = [
            Some(46.0),
            Some(47.0),
            None,
            Some(45.0),
            Some(90.0),
            Some(46.0),
        ];
        assert_eq!(spikes(&run), 2);
    }

    #[test]
    fn scores_rank_the_sample_routes() {
        assert!(VIA_FRA.score() < VIA_VIE.score());
        assert!(VIA_VIE.score() < DIRECT.score());
    }

    #[test]
    fn first_race_picks_the_best_at_once() {
        let mut s = Switcher::default();
        let won = s.race(&[
            ("direct", DIRECT),
            ("vie-1", VIA_VIE),
            ("fra-2 par-1", VIA_FRA),
        ]);
        assert_eq!(won.as_deref(), Some("fra-2 par-1"));
    }

    #[test]
    fn switch_waits_for_a_clear_lead_three_races_in_a_row() {
        let mut s = Switcher::default();
        s.race(&[("vie-1", VIA_VIE)]);
        let race = [("vie-1", VIA_VIE), ("fra-2 par-1", VIA_FRA)];
        assert_eq!(s.race(&race), None);
        assert_eq!(s.race(&race), None);
        assert_eq!(s.race(&race).as_deref(), Some("fra-2 par-1"));
    }

    #[test]
    fn close_routes_do_not_flap() {
        let mut s = Switcher::default();
        s.race(&[("a", VIA_FRA)]);
        let near = Stats {
            ping_ms: 44.0,
            ..VIA_FRA
        }; // 2 points better: not enough
        for _ in 0..10 {
            assert_eq!(s.race(&[("a", VIA_FRA), ("b", near)]), None);
        }
        assert_eq!(s.current(), Some("a"));
    }

    #[test]
    fn a_dead_route_is_left_at_once() {
        let mut s = Switcher::default();
        s.race(&[("fra-2 par-1", VIA_FRA)]);
        assert_eq!(s.race(&[("direct", DIRECT)]).as_deref(), Some("direct"));
    }

    #[test]
    fn dedup_keeps_the_first_copy() {
        let mut d = Dedup::default();
        assert!(d.accept(1));
        assert!(!d.accept(1));
        assert!(d.accept(3));
        assert!(d.accept(2)); // late, but the first copy of 2
        assert!(!d.accept(2));
        assert!(!d.accept(3));
    }

    #[test]
    fn dedup_window_slides() {
        let mut d = Dedup::default();
        assert!(d.accept(10));
        assert!(d.accept(10 + WINDOW + 5)); // a jump clears the window
        assert!(!d.accept(10)); // too old now
        assert!(d.accept(10 + WINDOW + 4));
        // A number that reuses an old slot isn't mistaken for a duplicate.
        let mut d = Dedup::default();
        assert!(d.accept(0));
        assert!(d.accept(WINDOW - 1));
        assert!(d.accept(WINDOW + 1));
        assert!(d.accept(WINDOW)); // same slot as 0, but new
        assert!(!d.accept(WINDOW));
    }

    #[test]
    fn multipath_cost_is_one_extra_copy() {
        let up = extra_bytes_per_sec(128.0, 56.0);
        assert_eq!(up, 128.0 * (56.0 + 19.0 + 28.0));
    }

    #[test]
    fn header_round_trip() {
        let h = Header {
            kind: Kind::Data,
            session: 0xfeed,
            seq: 42,
        };
        let mut buf = [0u8; HEADER_LEN + 3];
        h.encode(&mut buf);
        buf[HEADER_LEN..].copy_from_slice(b"abc");
        assert_eq!(Header::decode(&buf), Some((h, &b"abc"[..])));
        assert_eq!(Header::decode(b"GET / HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn allowlist_refuses_private_and_low_ports() {
        let t = |s: &str| check_target(s.parse::<SocketAddr>().unwrap(), false);
        assert_eq!(t("203.0.113.48:7088"), Ok(()));
        assert_eq!(t("203.0.113.48:443"), Err(Refusal::LowPort));
        assert_eq!(t("203.0.113.48:53"), Err(Refusal::LowPort));
        assert_eq!(t("127.0.0.1:7088"), Err(Refusal::NotPublic));
        assert_eq!(t("10.0.0.5:7088"), Err(Refusal::NotPublic));
        assert_eq!(t("169.254.169.254:8080"), Err(Refusal::NotPublic)); // cloud metadata
        assert_eq!(t("100.64.1.1:7088"), Err(Refusal::NotPublic));
        assert_eq!(t("[fd00::1]:7088"), Err(Refusal::NotPublic));
        assert_eq!(t("[::ffff:192.168.1.1]:7088"), Err(Refusal::NotPublic));
        assert_eq!(t("[2001:db8::1]:7088"), Ok(()));
    }

    #[test]
    fn rate_limit_caps_packets_and_bytes() {
        let mut r = RateLimit::new(400, 2000); // 100 packets of burst
        let sent = (0..1000).filter(|_| r.allow(0, 60)).count();
        assert_eq!(sent, 100);
        assert!(r.allow(10, 60)); // 10 ms later, 4 more packets of room
        let mut r = RateLimit::new(10_000, 80); // 2,500 bytes of burst
        assert!(r.allow(0, 1200));
        assert!(r.allow(0, 1200));
        assert!(!r.allow(0, 1200));
    }

    fn checks() -> Checks {
        Checks {
            wifi_ghz: Some(5.0),
            router_loss_pct: 0.0,
            router_jitter_ms: 1.0,
            bufferbloat_ms: 12.0,
            download: None,
            valve_routed: false,
            direct: DIRECT,
            best: Some(("Frankfurt".into(), VIA_FRA)),
        }
    }

    #[test]
    fn verdict_route_fixed() {
        let v = verdict(&checks());
        assert_eq!(
            v,
            Verdict::RouteFixed {
                via: "Frankfurt".into(),
                saved_ms: 28
            }
        );
        assert_eq!(v.headline(), "Faster path found through Frankfurt.");
        assert_eq!(
            v.advice(),
            "Your normal path takes the long way. This one is 28 ms quicker."
        );
        assert!(v.detour_on());
    }

    #[test]
    fn verdict_wifi_comes_first() {
        let c = Checks {
            wifi_ghz: Some(2.4),
            router_loss_pct: 3.4,
            router_jitter_ms: 38.0,
            ..checks()
        };
        let v = verdict(&c);
        assert_eq!(v, Verdict::WiFi);
        assert!(!v.detour_on());
    }

    #[test]
    fn verdict_valve_and_direct_best() {
        let c = Checks {
            valve_routed: true,
            ..checks()
        };
        assert_eq!(verdict(&c).headline(), "This game already has a fast path.");
        let c = Checks {
            best: Some((
                "Vienna".into(),
                Stats {
                    ping_ms: 38.0,
                    jitter_ms: 2.0,
                    loss_pct: 0.0,
                },
            )),
            direct: Stats {
                ping_ms: 31.0,
                jitter_ms: 2.0,
                loss_pct: 0.0,
            },
            ..checks()
        };
        assert_eq!(verdict(&c), Verdict::DirectBest);
        assert_eq!(verdict(&c).advice(), "Don't pay anyone for a ping booster.");
    }

    #[test]
    fn verdict_names_the_download() {
        let c = Checks {
            download: Some("Steam".into()),
            ..checks()
        };
        assert_eq!(verdict(&c).headline(), "Steam is filling your connection.");
    }
}
