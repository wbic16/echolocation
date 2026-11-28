//! echolocate — the echolocation reference CLI (SPEC.md).
//!
//! Two roles, one binary, no prior introduction between them:
//!
//!   echolocate answer --medium M --name N --home COORD --capabilities a,b
//!                     [--state "..."] [--mailbox COORD] [--universe U]
//!                     [--listen U1,U2] [--for SECONDS] [--poll-ms 100]
//!   echolocate probe  --medium M --name N --home COORD --capabilities a,b
//!                     [--universe U] [--target U] [--intent CAP|any]
//!                     [--ttl SECONDS] [--mode first|all] [--poll-ms 100]
//!
//! `answer` publishes its signature scroll at its home coordinate, then
//! scans the probe channels of the universes it inhabits and echoes.
//! `probe` emits a probe and reports whoever answers, with the range.

use echolocation::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

fn opts(args: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") && i + 1 < args.len() {
            map.insert(args[i][2..].to_string(), args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    map
}

fn get(o: &HashMap<String, String>, key: &str, default: &str) -> String {
    o.get(key).cloned().unwrap_or_else(|| default.to_string())
}

fn csv(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        Vec::new()
    } else {
        s.split(',').map(|p| p.trim().to_string()).collect()
    }
}

fn stamp() -> String {
    let now = now_unix();
    format!("{} EE {}", format_utc(now), ee_at(now))
}

fn run_answer(o: HashMap<String, String>) -> Result<()> {
    let medium_path = PathBuf::from(get(&o, "medium", "medium.phext"));
    let name = get(&o, "name", "responder");
    let home = get(&o, "home", "1.1.1/1.1.1/9.1.1");
    let universe = get(&o, "universe", "1.1.1");
    let listen: Vec<String> = csv(&get(&o, "listen", &universe));
    let poll_ms: u64 = get(&o, "poll-ms", "100").parse().unwrap_or(100);
    let state = o.get("state").cloned();
    let mailbox = o.get("mailbox").cloned();
    let for_secs: u64 = get(&o, "for", "0").parse().unwrap_or(0);

    let sig = Signature::new(&name, &home, csv(&get(&o, "capabilities", "")))?;
    let mut medium = FileMedium::new(medium_path);
    publish_signature(&mut medium, &sig)?;
    println!(
        "[{}] {} answering in universe {} (listening: {}), home {}, capabilities: {}",
        stamp(),
        name,
        universe,
        listen.join(", "),
        home,
        sig.capabilities.join(", ")
    );
    let started = std::time::Instant::now();
    loop {
        match answer_once(
            &mut medium,
            &sig,
            &universe,
            &listen,
            state.as_deref(),
            mailbox.as_deref(),
            poll_ms,
        )? {
            Some(info) => {
                println!(
                    "[{}] {} echoed probe {} from {} at {}",
                    stamp(),
                    name,
                    info.probe_id,
                    info.prober_name,
                    info.echo_coord
                );
            }
            None => std::thread::sleep(Duration::from_millis(poll_ms.max(10))),
        }
        if for_secs > 0 && started.elapsed() >= Duration::from_secs(for_secs) {
            println!("[{}] {} done listening", stamp(), name);
            return Ok(());
        }
    }
}

fn run_probe(o: HashMap<String, String>) -> Result<()> {
    let medium_path = PathBuf::from(get(&o, "medium", "medium.phext"));
    let name = get(&o, "name", "prober");
    let home = get(&o, "home", "1.1.1/1.1.1/9.1.1");
    let universe = get(&o, "universe", "1.1.1");
    let target = get(&o, "target", &universe);
    let intent = get(&o, "intent", "any");
    let ttl: u64 = get(&o, "ttl", "30").parse().unwrap_or(30);
    let poll_ms: u64 = get(&o, "poll-ms", "100").parse().unwrap_or(100);
    let mode = CollectMode::parse(&get(&o, "mode", "all"));

    let sig = Signature::new(&name, &home, csv(&get(&o, "capabilities", "")))?;
    let mut medium = FileMedium::new(medium_path);
    let handle = emit_probe(
        &mut medium,
        &sig,
        &universe,
        &target,
        &intent,
        Duration::from_secs(ttl),
        mode,
        poll_ms,
    )?;
    println!(
        "[{}] {} emitted probe {} into universe {} (intent: {}, reply at {}, expires {})",
        stamp(),
        name,
        handle.probe.id,
        target,
        intent,
        handle.probe.reply_at,
        format_utc(handle.probe.expires_unix)
    );
    let exchange = await_echo(&mut medium, &handle)?;
    match exchange.outcome {
        ExchangeOutcome::Expired => {
            println!(
                "[{}] {} heard silence — probe expired with no echo (absence, not assent)",
                stamp(),
                name
            );
        }
        ExchangeOutcome::Echoed => {
            for echo in &exchange.echoes {
                println!(
                    "[{}] {} got echo #{} from {} (home {}, home universe {}): capabilities [{}], state: {}, mailbox: {}",
                    stamp(),
                    name,
                    echo.index,
                    echo.from.name,
                    echo.from.home,
                    echo.home_universe,
                    echo.from.capabilities.join(", "),
                    echo.state.as_deref().unwrap_or("-"),
                    echo.mailbox.as_deref().unwrap_or("none")
                );
            }
            let responder_poll = exchange
                .echoes
                .first()
                .map(|e| format!("{} ms", e.poll_ms))
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "[{}] {} range: {} ms (poll: prober {} ms, responder {}; an upper bound — see SPEC §5)",
                stamp(),
                name,
                exchange.range_ms.unwrap_or(0),
                poll_ms,
                responder_poll
            );
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: echolocate <probe|answer> [--medium PATH] [--name N] [--home COORD] ...");
        std::process::exit(2);
    }
    let o = opts(&args[1..]);
    let result = match args[0].as_str() {
        "probe" => run_probe(o),
        "answer" => run_answer(o),
        other => {
            eprintln!("unknown role: {} (want probe|answer)", other);
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("echolocate: {}", e);
        std::process::exit(1);
    }
}
