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
