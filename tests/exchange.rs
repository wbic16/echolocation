//! Integration tests for the echolocation exchange (SPEC §6), run over a
//! real shared .phext file medium with separate FileMedium instances per
//! program — the same code paths the two-process demo exercises.

use echolocation::*;
use libphext::phext;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn temp_medium(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "echolocation-test-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("medium.phext")
}

fn sig(name: &str, home: &str, caps: &[&str]) -> Signature {
    Signature::new(name, home, caps.iter().map(|s| s.to_string()).collect()).unwrap()
}

fn spawn_responder(
    path: PathBuf,
    responder: Signature,
    home_universe: &str,
    inhabits: Vec<String>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let home_universe = home_universe.to_string();
    std::thread::spawn(move || {
        let mut medium = FileMedium::new(path);
        publish_signature(&mut medium, &responder).unwrap();
        while !stop.load(Ordering::Relaxed) {
            match answer_once(
                &mut medium,
                &responder,
                &home_universe,
                &inhabits,
                Some("ready"),
                Some("1.1.1/1.4.1/1.1.1"),
                25,
            ) {
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    })
}

#[test]
fn happy_path_discovery_identity_and_range() {
    let path = temp_medium("happy");
    let stop = Arc::new(AtomicBool::new(false));
    let responder = spawn_responder(
        path.clone(),
        sig("ni-knight-runner", "1.1.1/1.1.1/9.4.1", &["time-census", "consensus"]),
        "1.1.1",
        vec!["1.1.1".to_string()],
        stop.clone(),
    );

    let mut medium = FileMedium::new(path.clone());
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &["time-census"]);
    let handle = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "time-census",
        Duration::from_secs(10),
        CollectMode::First,
        25,
    )
    .unwrap();
    let exchange = await_echo(&mut medium, &handle).unwrap();
    stop.store(true, Ordering::Relaxed);
    responder.join().unwrap();

    match exchange.outcome {
        ExchangeOutcome::Echoed => {}
        ExchangeOutcome::Expired => panic!("expected an echo, probe expired"),
    }
    assert_eq!(exchange.echoes.len(), 1);
    let echo = &exchange.echoes[0];
    assert_eq!(echo.from.name, "ni-knight-runner");
    assert_eq!(echo.from.home, "1.1.1/1.1.1/9.4.1");
    assert!(echo.from.capabilities.contains(&"consensus".to_string()));
    assert_eq!(echo.home_universe, "1.1.1");
    assert_eq!(echo.mailbox.as_deref(), Some("1.1.1/1.4.1/1.1.1"));
    let range = exchange.range_ms.expect("range must be measured");
    assert!(range < 10_000, "range {} ms should be well under the TTL", range);
}

#[test]
fn no_echo_expiry_is_a_normal_terminal_state() {
    let path = temp_medium("silence");
    let mut medium = FileMedium::new(path);
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &["time-census"]);
    let handle = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "any",
        Duration::from_secs(2),
        CollectMode::All,
        25,
    )
    .unwrap();
    let started = Instant::now();
    let exchange = await_echo(&mut medium, &handle).unwrap();
    let elapsed = started.elapsed();
    match exchange.outcome {
        ExchangeOutcome::Expired => {}
        ExchangeOutcome::Echoed => panic!("nobody answered; outcome must be Expired"),
    }
    assert!(exchange.echoes.is_empty());
    assert!(exchange.range_ms.is_none());
    assert!(
        elapsed >= Duration::from_millis(1900),
        "expiry came early: {:?}",
        elapsed
    );
}

#[test]
fn multi_echo_collects_every_responder() {
    let path = temp_medium("multi");
    let stop = Arc::new(AtomicBool::new(false));
    let r1 = spawn_responder(
        path.clone(),
        sig("alpha-seat", "1.1.1/1.1.1/9.2.1", &["chorus"]),
        "1.1.1",
        vec!["1.1.1".to_string()],
        stop.clone(),
    );
    let r2 = spawn_responder(
        path.clone(),
        sig("beta-seat", "1.1.1/1.1.1/9.3.1", &["chorus"]),
        "1.1.1",
        vec!["1.1.1".to_string()],
        stop.clone(),
    );

    let mut medium = FileMedium::new(path.clone());
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &[]);
    let handle = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "chorus",
        Duration::from_secs(4),
        CollectMode::All,
        25,
    )
    .unwrap();
    let exchange = await_echo(&mut medium, &handle).unwrap();
    stop.store(true, Ordering::Relaxed);
    r1.join().unwrap();
    r2.join().unwrap();

    let mut names: Vec<&str> = exchange.echoes.iter().map(|e| e.from.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["alpha-seat", "beta-seat"]);
    assert!(exchange.range_ms.is_some());
}

#[test]
fn malformed_scrolls_are_skipped_never_fatal() {
    let path = temp_medium("malformed");
    // Pre-seed: garbage at probe slot 1 (not a probe), garbage at the echo
    // coordinate slot 2 / k 1 (not an echo). The real probe takes slot 2;
    // the responder must skip slot 1, answer slot 2 at k=2; the prober
    // must skip the garbage at k=1 and take the real echo at k=2.
    let mut doc = String::new();
    doc = phext::replace(&doc, probe_coord("1.1.1", 1).unwrap(), "this is not a probe at all");
    doc = phext::replace(
        &doc,
        echo_coord("1.1.1", 2, 1).unwrap(),
        "echolocation/echo 1\nbut: nothing else checks out\n",
    );
    std::fs::write(&path, &doc).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let responder = spawn_responder(
        path.clone(),
        sig("ni-knight-runner", "1.1.1/1.1.1/9.4.1", &["time-census"]),
        "1.1.1",
        vec!["1.1.1".to_string()],
        stop.clone(),
    );

    let mut medium = FileMedium::new(path.clone());
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &[]);
    let handle = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "time-census",
        Duration::from_secs(10),
        CollectMode::First,
        25,
    )
    .unwrap();
    assert_eq!(handle.probe.slot, 2, "slot 1 is occupied by garbage; append-only takes 2");
    let exchange = await_echo(&mut medium, &handle).unwrap();
    stop.store(true, Ordering::Relaxed);
    responder.join().unwrap();

    assert_eq!(exchange.echoes.len(), 1);
    assert_eq!(exchange.echoes[0].index, 2, "k=1 is garbage; the real echo lands at k=2");
    assert_eq!(exchange.echoes[0].from.name, "ni-knight-runner");
}

#[test]
fn cross_universe_echo_crosses_back() {
    let path = temp_medium("xuniverse");
    let stop = Arc::new(AtomicBool::new(false));
    // The responder inhabits universe 2.1.1 only. The prober lives in 1.1.1
    // and probes into 2.1.1; the echo must cross back to 1.1.1's channel.
    let responder = spawn_responder(
        path.clone(),
        sig("far-seat", "2.1.1/1.1.1/9.4.1", &["cartography"]),
        "2.1.1",
        vec!["2.1.1".to_string()],
        stop.clone(),
    );

    let mut medium = FileMedium::new(path.clone());
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &[]);
    let handle = emit_probe(
        &mut medium,
        &prober,
        "1.1.1", // prober's home universe — the reply lands here
        "2.1.1", // target universe
        "cartography",
        Duration::from_secs(10),
        CollectMode::First,
        25,
    )
    .unwrap();
    let exchange = await_echo(&mut medium, &handle).unwrap();
    stop.store(true, Ordering::Relaxed);
    responder.join().unwrap();

    assert_eq!(exchange.echoes.len(), 1);
    let echo = &exchange.echoes[0];
    assert_eq!(echo.from.name, "far-seat");
    assert_eq!(echo.home_universe, "2.1.1");

    // Physical placement: the echo scroll sits in the prober's universe
    // (1.1.1), not in the responder's (2.1.1).
    let doc = std::fs::read_to_string(&path).unwrap();
    let at_home = phext::fetch(&doc, echo_coord("1.1.1", handle.probe.slot, 1).unwrap());
    assert!(at_home.contains("from-name: far-seat"), "echo must land in 1.1.1");
    let at_far = phext::fetch(&doc, echo_coord("2.1.1", handle.probe.slot, 1).unwrap());
    assert!(at_far.is_empty(), "no echo should be written in 2.1.1's channel");
    // And the probe itself sits in the target universe's probe channel.
    let probe_there = phext::fetch(&doc, probe_coord("2.1.1", handle.probe.slot).unwrap());
    assert!(probe_there.contains("echolocation/probe 1"));
}

#[test]
fn concurrent_readers_and_writers_on_file_medium() {
    let path = temp_medium("concurrent-io");
    let stop = Arc::new(AtomicBool::new(false));
    let reader_path = path.clone();
    let reader_stop = stop.clone();

    let reader = std::thread::spawn(move || {
        let mut m = FileMedium::new(reader_path);
        let mut reads = 0usize;
        while !reader_stop.load(Ordering::Relaxed) {
            let _ = m.read().expect("concurrent read must not fail");
            reads += 1;
        }
        reads
    });

    let mut writer = FileMedium::new(path.clone());
    for i in 1..=25usize {
        let payload = format!("scroll-{}", i);
        let c = probe_coord("1.1.1", i).unwrap();
        writer
            .update(&mut |doc| Ok((phext::replace(doc, c, &payload), ())))
            .expect("concurrent update must not fail");
    }

    stop.store(true, Ordering::Relaxed);
    let reads = reader.join().unwrap();
    assert!(reads > 0, "reader should have performed concurrent reads");

    let doc = writer.read().unwrap();
    assert_eq!(phext::fetch(&doc, probe_coord("1.1.1", 25).unwrap()), "scroll-25");
}

#[test]
fn cli_range_reports_both_prober_and_responder_poll_ms() {
    let path = temp_medium("cli-poll");
    let bin = env!("CARGO_BIN_EXE_echolocate");

    let mut responder = std::process::Command::new(bin)
        .args([
            "answer",
            "--medium",
            path.to_str().unwrap(),
            "--name",
            "cli-responder",
            "--home",
            "1.1.1/1.1.1/9.4.1",
            "--capabilities",
            "time-census",
            "--poll-ms",
            "75",
            "--for",
            "4",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn echolocate answer");

    std::thread::sleep(Duration::from_millis(250));

    let probe_out = std::process::Command::new(bin)
        .args([
            "probe",
            "--medium",
            path.to_str().unwrap(),
            "--name",
            "cli-prober",
            "--home",
            "1.1.1/1.1.1/9.1.1",
            "--intent",
            "time-census",
            "--ttl",
            "4",
            "--mode",
            "first",
            "--poll-ms",
            "60",
        ])
        .output()
        .expect("run echolocate probe");

    let _ = responder.wait();
    assert!(
        probe_out.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&probe_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&probe_out.stdout);
    assert!(
        stdout.contains("poll: prober 60 ms, responder 75 ms"),
        "expected both poll values per SPEC §5, got:\n{}",
        stdout
    );
}

#[test]
fn sq_medium_exchange_and_auth() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    fn pct_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                }
            }
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock SQ");
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();

    let doc = Arc::new(Mutex::new(String::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let expected_key = "test-sq-key";

    let server_doc = doc.clone();
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 2048];
                    let header_end;
                    loop {
                        let n = match stream.read(&mut tmp) {
                            Ok(0) | Err(_) => {
                                header_end = 0;
                                break;
                            }
                            Ok(n) => n,
                        };
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = pos + 4;
                            break;
                        }
                    }
                    if header_end == 0 {
                        continue;
                    }
                    let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
                    let content_len: usize = headers
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let mut body = buf[header_end..].to_vec();
                    while body.len() < content_len {
                        let n = stream.read(&mut tmp).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&tmp[..n]);
                    }
                    body.truncate(content_len);

                    let auth_ok = headers.lines().any(|l| {
                        l.to_ascii_lowercase()
                            .starts_with("authorization:")
                            && l.contains(expected_key)
                    });
                    if !auth_ok {
                        let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\n\r\nUnauthorized";
                        let _ = stream.write_all(resp.as_bytes());
                        continue;
                    }

                    let req_line = headers.lines().next().unwrap_or("");
                    let target = req_line.split_whitespace().nth(1).unwrap_or("");
                    let (route, query) = target.split_once('?').unwrap_or((target, ""));
                    let mut params = std::collections::HashMap::new();
                    for pair in query.split('&') {
                        if let Some((k, v)) = pair.split_once('=') {
                            params.insert(pct_decode(k), pct_decode(v));
                        }
                    }

                    let reply = if req_line.starts_with("GET ") && route == "/api/v2/get" {
                        server_doc.lock().unwrap().clone()
                    } else if req_line.starts_with("GET ") && route == "/api/v2/select" {
                        let c = phext::to_coordinate(params.get("c").map(|s| s.as_str()).unwrap_or("1.1.1/1.1.1/1.1.1"));
                        phext::fetch(&server_doc.lock().unwrap(), c)
                    } else if req_line.starts_with("POST ") && route == "/api/v2/update" {
                        let c = phext::to_coordinate(params.get("c").map(|s| s.as_str()).unwrap_or("1.1.1/1.1.1/1.1.1"));
                        let text = String::from_utf8_lossy(&body).into_owned();
                        let mut guard = server_doc.lock().unwrap();
                        *guard = phext::replace(&guard, c, &text);
                        "Updated".to_string()
                    } else {
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found";
                        let _ = stream.write_all(resp.as_bytes());
                        continue;
                    };

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        reply.len(),
                        reply
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    let base_url = format!("http://{}", addr);

    // Wrong key must fail loudly.
    let mut bad_sq = SqMedium::new(&base_url, "echolocation", Some("wrong-key".to_string())).unwrap();
    assert!(bad_sq.read().is_err(), "unauthorized SQ read must return Err");

    // HTTPS must fail clearly (zero-dep TcpStream is plaintext HTTP).
    assert!(SqMedium::new("https://example.com", "echolocation", None).is_err());

    // Run responder and prober against the mock SQ host.
    let r_url = base_url.clone();
    let r_stop = stop.clone();
    let responder_thread = std::thread::spawn(move || {
        let mut m = SqMedium::new(&r_url, "echolocation", Some(expected_key.to_string())).unwrap();
        let responder = sig("sq-seat", "1.1.1/1.1.1/9.4.1", &["time-census"]);
        publish_signature(&mut m, &responder).unwrap();
        while !r_stop.load(Ordering::Relaxed) {
            if let Ok(Some(_)) = answer_once(
                &mut m,
                &responder,
                "1.1.1",
                &["1.1.1".to_string()],
                Some("sq-ready"),
                Some("1.1.1/1.4.1/1.1.1"),
                50,
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    });

    let mut prober_sq = SqMedium::new(&base_url, "echolocation", Some(expected_key.to_string())).unwrap();
    let prober = sig("mira-sq", "1.1.1/1.1.1/9.1.1", &["time-census"]);
    let handle = emit_probe(
        &mut prober_sq,
        &prober,
        "1.1.1",
        "1.1.1",
        "time-census",
        Duration::from_secs(6),
        CollectMode::First,
        50,
    )
    .unwrap();
    let exchange = await_echo(&mut prober_sq, &handle).unwrap();
    stop.store(true, Ordering::Relaxed);
    responder_thread.join().unwrap();
    server.join().unwrap();

    match exchange.outcome {
        ExchangeOutcome::Echoed => {}
        ExchangeOutcome::Expired => panic!("expected echo over SqMedium"),
    }
    assert_eq!(exchange.echoes.len(), 1);
    assert_eq!(exchange.echoes[0].from.name, "sq-seat");
    assert_eq!(exchange.echoes[0].poll_ms, 50);
    assert_eq!(exchange.echoes[0].state.as_deref(), Some("sq-ready"));
}
