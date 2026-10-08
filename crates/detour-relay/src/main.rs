//! detour-relay: a small UDP relay for Detour, std only.
//!
//! A client opens a session for one game server. From then on the relay
//! forwards that session's packets to that one address and sends the replies
//! back, rate-limited. It never forwards anywhere else, never to a private
//! address or a port below 1024, and never carries TCP, so it can't be used
//! as an open proxy or for web traffic.
//!
//! A session may have two paths (the client's own route, and another relay).
//! The relay forwards the first copy of each packet and drops the duplicate,
//! and sends each reply down both paths. That is multipath.
//!
//! Usage: detour-relay [/etc/detour-relay.conf]

use detour_core::multipath::Dedup;
use detour_core::relay::{HEADER_LEN, Header, Kind, PORT, RateLimit, Refusal, check_target};
use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
struct Config {
    listen: SocketAddr,
    max_sessions: usize,
    per_client: usize,
    rate_pps: u32,
    rate_kbps: u32,
    idle_secs: u64,
    /// Only for tests on one machine. Never set this on a public relay.
    allow_private: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: SocketAddr::from(([0, 0, 0, 0], PORT)),
            max_sessions: 64,
            per_client: 4,
            rate_pps: 600,
            rate_kbps: 2000,
            idle_secs: 60,
            allow_private: false,
        }
    }
}

/// `key = value` lines; `#` starts a comment.
fn parse_config(text: &str) -> Result<Config, String> {
    let mut c = Config::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or(format!("line {}: expected key = value", n + 1))?;
        let (k, v) = (k.trim(), v.trim());
        let bad = |_| format!("line {}: bad value for {k}", n + 1);
        match k {
            "listen" => {
                c.listen = v
                    .parse()
                    .map_err(|_| format!("line {}: bad address", n + 1))?
            }
            "max_sessions" => c.max_sessions = v.parse().map_err(bad)?,
            "per_client" => c.per_client = v.parse().map_err(bad)?,
            "rate_pps" => c.rate_pps = v.parse().map_err(bad)?,
            "rate_kbps" => c.rate_kbps = v.parse().map_err(bad)?,
            "idle_secs" => c.idle_secs = v.parse().map_err(bad)?,
            _ => return Err(format!("line {}: unknown key {k}", n + 1)),
        }
    }
    Ok(c)
}

struct Session {
    target: SocketAddr,
    /// Where replies go: the client, and at most one other relay.
    paths: Arc<Mutex<Vec<SocketAddr>>>,
    upstream: UdpSocket,
    dedup: Dedup,
    limit: RateLimit,
    last_seen: Instant,
    open: Arc<AtomicBool>,
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/detour-relay.conf".into());
    let config = match std::fs::read_to_string(&path) {
        Ok(text) => parse_config(&text).unwrap_or_else(|e| fail(&format!("{path}: {e}"))),
        Err(e) if e.kind() == ErrorKind::NotFound => Config::default(),
        Err(e) => fail(&format!("{path}: {e}")),
    };
    let socket = UdpSocket::bind(config.listen)
        .unwrap_or_else(|e| fail(&format!("bind {}: {e}", config.listen)));
    eprintln!(
        "detour-relay {} listening on udp {}",
        env!("CARGO_PKG_VERSION"),
        config.listen
    );
    if let Err(e) = serve(socket, config) {
        fail(&e.to_string());
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("detour-relay: {msg}");
    std::process::exit(1);
}

fn serve(socket: UdpSocket, config: Config) -> io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    let started = Instant::now();
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    let mut buf = [0u8; 2048];
    loop {
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                expire(&mut sessions, config.idle_secs);
                continue;
            }
            Err(e) if e.kind() == ErrorKind::ConnectionReset => continue, // Windows: an ICMP from an old send
            Err(e) => return Err(e),
        };
        let Some((head, body)) = Header::decode(&buf[..n]) else {
            continue;
        };
        match head.kind {
            Kind::Probe => {
                let _ = socket.send_to(&buf[..n], from);
            }
            Kind::Open => {
                let answer = match open(&mut sessions, &socket, &config, head.session, body, from) {
                    Ok(()) => "ok".to_string(),
                    Err(why) => format!("refused: {why}"),
                };
                let mut out = [0u8; 128];
                head.encode(&mut out);
                let len = HEADER_LEN + answer.len().min(out.len() - HEADER_LEN);
                out[HEADER_LEN..len].copy_from_slice(&answer.as_bytes()[..len - HEADER_LEN]);
                let _ = socket.send_to(&out[..len], from);
            }
            Kind::Data => {
                let Some(s) = sessions.get_mut(&head.session) else {
                    continue;
                };
                if !s.paths.lock().unwrap().contains(&from) {
                    continue;
                }
                s.last_seen = Instant::now();
                let now_ms = started.elapsed().as_millis() as u64;
                if !s.limit.allow(now_ms, body.len()) || !s.dedup.accept(head.seq) {
                    continue;
                }
                let _ = s.upstream.send(body);
            }
        }
    }
}

/// Opens a session, or adds a second path to one. The body is `target=<ip>:<port>`.
fn open(
    sessions: &mut HashMap<u64, Session>,
    socket: &UdpSocket,
    config: &Config,
    id: u64,
    body: &[u8],
    from: SocketAddr,
) -> Result<(), String> {
    let target: SocketAddr = std::str::from_utf8(body)
        .ok()
        .and_then(|b| b.trim().strip_prefix("target="))
        .and_then(|t| t.parse().ok())
        .ok_or("expected target=<ip>:<port>")?;
    if let Some(s) = sessions.get_mut(&id) {
        // The same session from a second path, for multipath. Never a new target.
        if s.target != target {
            return Err("a session has one target".into());
        }
        let mut paths = s.paths.lock().unwrap();
        if !paths.contains(&from) {
            if paths.len() >= 2 {
                return Err("a session has at most two paths".into());
            }
            paths.push(from);
        }
        s.last_seen = Instant::now();
        return Ok(());
    }
    check_target(target, config.allow_private).map_err(|r| match r {
        Refusal::NotPublic => "not a public address",
        Refusal::LowPort => "ports below 1024 are not relayed",
        Refusal::TooManySessions => "too many sessions",
    })?;
    let mine = sessions
        .values()
        .filter(|s| s.paths.lock().unwrap()[0].ip() == from.ip())
        .count();
    if sessions.len() >= config.max_sessions || mine >= config.per_client {
        return Err("too many sessions".into());
    }
    let any: SocketAddr = if target.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let upstream = UdpSocket::bind(any).map_err(|e| e.to_string())?;
    upstream.connect(target).map_err(|e| e.to_string())?;
    upstream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| e.to_string())?;
    let paths = Arc::new(Mutex::new(vec![from]));
    let open = Arc::new(AtomicBool::new(true));
    replies(
        id,
        upstream.try_clone().map_err(|e| e.to_string())?,
        socket.try_clone().map_err(|e| e.to_string())?,
        paths.clone(),
        open.clone(),
    );
    sessions.insert(
        id,
        Session {
            target,
            paths,
            upstream,
            dedup: Dedup::default(),
            limit: RateLimit::new(config.rate_pps, config.rate_kbps),
            last_seen: Instant::now(),
            open,
        },
    );
    Ok(())
}

/// Replies from the game server go back down every path, numbered so the
/// client can keep the first copy.
fn replies(
    id: u64,
    upstream: UdpSocket,
    socket: UdpSocket,
    paths: Arc<Mutex<Vec<SocketAddr>>>,
    open: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let mut seq = 0u64;
        while open.load(Ordering::Relaxed) {
            let n = match upstream.recv(&mut buf[HEADER_LEN..]) {
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionRefused
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
            };
            seq += 1;
            Header {
                kind: Kind::Data,
                session: id,
                seq,
            }
            .encode(&mut buf);
            for p in paths.lock().unwrap().iter() {
                let _ = socket.send_to(&buf[..HEADER_LEN + n], p);
            }
        }
    });
}

fn expire(sessions: &mut HashMap<u64, Session>, idle_secs: u64) {
    sessions.retain(|_, s| {
        let keep = s.last_seen.elapsed() < Duration::from_secs(idle_secs);
        if !keep {
            s.open.store(false, Ordering::Relaxed);
        }
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(sock: &UdpSocket, to: SocketAddr, kind: Kind, session: u64, seq: u64, body: &[u8]) {
        let mut buf = vec![0u8; HEADER_LEN + body.len()];
        Header { kind, session, seq }.encode(&mut buf);
        buf[HEADER_LEN..].copy_from_slice(body);
        sock.send_to(&buf, to).unwrap();
    }

    fn recv(sock: &UdpSocket) -> Option<(Vec<u8>, SocketAddr)> {
        let mut buf = [0u8; 2048];
        sock.recv_from(&mut buf)
            .ok()
            .map(|(n, a)| (buf[..n].to_vec(), a))
    }

    #[test]
    fn relays_one_session_to_one_server_and_keeps_first_copies() {
        let game = UdpSocket::bind("127.0.0.1:0").unwrap();
        game.set_read_timeout(Some(Duration::from_millis(400)))
            .unwrap();
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = relay.local_addr().unwrap();
        thread::spawn(move || {
            serve(
                relay,
                Config {
                    allow_private: true,
                    ..Config::default()
                },
            )
        });

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(400)))
            .unwrap();
        let target = format!("target={}", game.local_addr().unwrap());
        send(&client, at, Kind::Open, 7, 0, target.as_bytes());
        assert!(recv(&client).unwrap().0.ends_with(b"ok"));

        // Probes come straight back.
        send(&client, at, Kind::Probe, 7, 99, b"");
        assert_eq!(Header::decode(&recv(&client).unwrap().0).unwrap().0.seq, 99);

        // Two copies of packet 1, one of packet 2: the game sees each once.
        send(&client, at, Kind::Data, 7, 1, b"one");
        send(&client, at, Kind::Data, 7, 1, b"one");
        send(&client, at, Kind::Data, 7, 2, b"two");
        let (first, upstream) = recv(&game).unwrap();
        assert_eq!(first, b"one");
        assert_eq!(recv(&game).unwrap().0, b"two");
        assert!(recv(&game).is_none());

        // The reply comes back numbered, on the session.
        game.send_to(b"state", upstream).unwrap();
        let (reply, _) = recv(&client).unwrap();
        let (head, body) = Header::decode(&reply).unwrap();
        assert_eq!(
            (head.kind, head.session, head.seq, body),
            (Kind::Data, 7, 1, &b"state"[..])
        );

        // A stranger can't send on the session.
        let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
        send(&stranger, at, Kind::Data, 7, 3, b"spoof");
        assert!(recv(&game).is_none());

        // And a session can't be pointed at a low port, such as DNS or the web.
        send(&client, at, Kind::Open, 8, 0, b"target=127.0.0.1:53");
        assert!(
            recv(&client)
                .unwrap()
                .0
                .ends_with(b"refused: ports below 1024 are not relayed")
        );
    }

    #[test]
    fn config_file() {
        let c = parse_config("# relay\nlisten = 0.0.0.0:47820\nrate_pps = 300 # per session\n")
            .unwrap();
        assert_eq!(c.rate_pps, 300);
        assert!(!c.allow_private);
        assert!(parse_config("allow_private = true").is_err());
        assert!(parse_config("listen = nowhere").is_err());
    }
}
