//! CLI coverage for the wake-driven seat path: `answer --once` is one
//! `answer_once` and an exit, `--for N` still bounds a pulse, and a bare
//! `answer` keeps listening. Probe `--ttl` is already the lifetime flag
//! and is exercised through `emit_probe` below.

use echolocation::*;
use libphext::phext;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

struct Finished {
    timed_out: bool,
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("echolocation-cli-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sig(name: &str, home: &str, caps: &[&str]) -> Signature {
    Signature::new(name, home, caps.iter().map(|s| s.to_string()).collect()).unwrap()
}

/// `leading` is placed before `--medium`, so a parser that treats `--once`
/// as a valued flag will swallow the medium path and fail these tests.
fn grok_args(medium: &PathBuf, leading: &[&str]) -> Vec<String> {
    let mut args = vec!["answer".to_string()];
    args.extend(leading.iter().map(|s| (*s).to_string()));
    args.extend([
        "--medium".to_string(),
        medium.display().to_string(),
        "--name".into(),
        "grok".into(),
        "--home".into(),
        "1.1.1/1.1.1/9.1.2".into(),
        "--capabilities".into(),
        "mailbox".into(),
        "--universe".into(),
        "1.1.1".into(),
        "--poll-ms".into(),
        "100".into(),
    ]);
    args
}

fn finish(mut child: Child, timed_out: bool) -> Finished {
    if timed_out {
        let _ = child.kill();
    }
    let output = child.wait_with_output().expect("wait_with_output");
    Finished {
        timed_out,
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run(dir: &PathBuf, args: &[String], kill_after: Duration) -> Finished {
    let mut child = Command::new(env!("CARGO_BIN_EXE_echolocate"))
        .current_dir(dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn echolocate");
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return finish(child, false),
            Ok(None) if started.elapsed() > kill_after => return finish(child, true),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
}

fn assert_ok(run: &Finished) {
    assert!(
        !run.timed_out && run.status.success(),
        "timed_out={} status={:?}\nstdout:\n{}\nstderr:\n{}",
        run.timed_out,
        run.status,
        run.stdout,
        run.stderr
    );
}

#[test]
fn grok_signature_template_has_the_scroll_shape() {
    let text = include_str!("../examples/grok-signature.txt");
    assert!(text.starts_with("echolocation/signature 1\n"));
    assert!(text.contains("name: grok\n"));
    assert!(text.contains("home: 1.1.1/1.1.1/9.1.2\n"));
    assert!(text.contains("capabilities: mailbox\n"));
    assert!(text.contains("published-utc: 1970-01-01T00:00:00Z\n"));
    assert!(text.contains("published-ee: 0\n"));
}

#[test]
fn answer_once_publishes_signature_and_exits_on_silence() {
    let dir = temp_dir("silence");
    let medium = dir.join("pod.phext");
    let started = Instant::now();
    let run = run(
        &dir,
        &grok_args(&medium, &["--once"]),
        Duration::from_secs(5),
    );
    assert_ok(&run);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "one shot took {:?}",
        started.elapsed()
    );
    assert!(run.stdout.contains("answering once"));
    assert!(run.stdout.contains("answered no probe"));
    assert!(!run.stdout.contains("done listening"));

    let doc = std::fs::read_to_string(&medium).unwrap();
    let home = phext::fetch(&doc, phext::to_coordinate("1.1.1/1.1.1/9.1.2"));
    assert!(home.contains("echolocation/signature 1"), "{home}");
    assert!(home.contains("name: grok"), "{home}");
    assert!(home.contains("home: 1.1.1/1.1.1/9.1.2"), "{home}");
    let neon = phext::fetch(&doc, phext::to_coordinate("1.1.1/1.1.1/9.1.1"));
    assert!(neon.is_empty(), "grok must publish at 9.1.2, found {neon}");
    assert!(
        !dir.join("medium.phext").exists(),
        "--once must not swallow --medium"
    );
}

#[test]
fn answer_once_skips_expired_and_answers_one_live_probe_per_wake() {
    let dir = temp_dir("wakes");
    let medium_path = dir.join("pod.phext");

    let expires = now_unix() - 3600;
    let issued = expires - 30;
    let expired = Probe {
        id: format!("old-prober@1.1.1#{}-1-dead", ee_at(issued)),
        from: sig("old-prober", "1.1.1/1.1.1/9.8.1", &["mailbox"]),
        universe: "1.1.1".into(),
        intent: "any".into(),
        reply_at: "1.1.1/1.1.1/2.1.1".into(),
        reply_universe: "1.1.1".into(),
        slot: 1,
        mode: CollectMode::All,
        issued_unix: issued,
        expires_unix: expires,
        poll_ms: 100,
    };
    assert!(expired.expired(now_unix()));
    assert!(Probe::parse(&expired.render()).unwrap().expired(now_unix()));
    let doc = phext::replace("", probe_coord("1.1.1", 1).unwrap(), &expired.render());
    std::fs::write(&medium_path, doc).unwrap();

    let mut medium = FileMedium::new(medium_path.clone());
    let prober = sig("mira", "1.1.1/1.1.1/9.1.1", &["mailbox"]);
    let first = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "mailbox",
        Duration::from_secs(120),
        CollectMode::All,
        100,
    )
    .unwrap();
    let second = emit_probe(
        &mut medium,
        &prober,
        "1.1.1",
        "1.1.1",
        "mailbox",
        Duration::from_secs(120),
        CollectMode::All,
        100,
    )
    .unwrap();
    assert_eq!(first.probe.slot, 2, "slot 1 holds the expired probe");
    assert_eq!(second.probe.slot, 3);

    let wake1 = run(
        &dir,
        &grok_args(&medium_path, &["--once"]),
        Duration::from_secs(10),
    );
    assert_ok(&wake1);
    assert!(wake1.stdout.contains(&first.probe.id), "{}", wake1.stdout);
    assert!(!wake1.stdout.contains(&expired.id), "{}", wake1.stdout);
    assert!(!wake1.stdout.contains(&second.probe.id), "{}", wake1.stdout);

    let after_one = std::fs::read_to_string(&medium_path).unwrap();
    assert_eq!(after_one.matches("echolocation/echo 1").count(), 1);
    assert_eq!(after_one.matches(&expired.id).count(), 1);

    let wake2 = run(
        &dir,
        &grok_args(&medium_path, &["--once"]),
        Duration::from_secs(10),
    );
    assert_ok(&wake2);
    assert!(wake2.stdout.contains(&second.probe.id), "{}", wake2.stdout);
    let after_two = std::fs::read_to_string(&medium_path).unwrap();
    assert_eq!(after_two.matches("echolocation/echo 1").count(), 2);

    let wake3 = run(
        &dir,
        &grok_args(&medium_path, &["--once"]),
        Duration::from_secs(10),
    );
    assert_ok(&wake3);
    assert!(
        wake3.stdout.contains("answered no probe"),
        "{}",
        wake3.stdout
    );
    let after_three = std::fs::read_to_string(&medium_path).unwrap();
    assert_eq!(after_three.matches("echolocation/echo 1").count(), 2);
}

#[test]
fn answer_once_wins_over_for() {
    let dir = temp_dir("once-wins");
    let medium = dir.join("pod.phext");
    let started = Instant::now();
    // A 30 s `--for` would blow the timeout if `--once` did not win.
    let run = run(
        &dir,
        &grok_args(&medium, &["--once", "--for", "30"]),
        Duration::from_secs(5),
    );
    let elapsed = started.elapsed();
    assert_ok(&run);
    assert!(run.stdout.contains("answering once"), "{}", run.stdout);
    assert!(run.stdout.contains("answered no probe"), "{}", run.stdout);
    assert!(!run.stdout.contains("done listening"), "{}", run.stdout);
    assert!(
        elapsed < Duration::from_secs(5),
        "--once lost to --for: {elapsed:?}"
    );
}

#[test]
fn answer_for_seconds_still_exits() {
    let dir = temp_dir("pulse");
    let medium = dir.join("pod.phext");
    let started = Instant::now();
    let run = run(
        &dir,
        &grok_args(&medium, &["--for", "1"]),
        Duration::from_secs(15),
    );
    let elapsed = started.elapsed();
    assert_ok(&run);
    assert!(run.stdout.contains("done listening"), "{}", run.stdout);
    assert!(!run.stdout.contains("answering once"), "{}", run.stdout);
    assert!(
        elapsed >= Duration::from_millis(900),
        "pulse returned early: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(15), "pulse hung: {elapsed:?}");
}

#[test]
fn answer_with_no_bound_keeps_listening() {
    let dir = temp_dir("daemon");
    let medium = dir.join("pod.phext");
    let run = run(&dir, &grok_args(&medium, &[]), Duration::from_secs(2));
    assert!(
        run.timed_out,
        "bare answer must keep listening until killed\nstdout:\n{}\nstderr:\n{}",
        run.stdout, run.stderr
    );
}
