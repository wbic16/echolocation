//! Echolocation — a generic multiversal phext pathway between any two programs.
//!
//! A program places a PROBE scroll at a well-known coordinate in a shared
//! phext medium; any listener may answer with an ECHO scroll carrying its
//! signature, capabilities, and state. The prober measures the round trip:
//! the range. Discovery + identity + ranging in one exchange, with no prior
//! introduction. See SPEC.md for the protocol this library implements.
//!
//! All phext reading/writing goes through libphext (the reference
//! implementation); EE stamps come from `libphext::epoch`.

use libphext::phext;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    LockTimeout(PathBuf),
    InvalidCoordinate(String),
    InvalidUniverse(String),
    Medium(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {}", e),
            Error::LockTimeout(p) => write!(f, "medium lock timeout: {}", p.display()),
            Error::InvalidCoordinate(s) => write!(f, "invalid coordinate: {}", s),
            Error::InvalidUniverse(s) => write!(f, "invalid universe (want L.Sh.S): {}", s),
            Error::Medium(s) => write!(f, "medium error: {}", s),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Time helpers (UTC stamps are second-precision, format YYYY-MM-DDTHH:MM:SSZ)
// ---------------------------------------------------------------------------

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Exocortical Epoch of a unix time, via libphext's epoch module
/// (EE = floor((unix − 1795327200) / 420) + 1; 0 before the anchor).
pub fn ee_at(unix: i64) -> u64 {
    libphext::epoch::epoch_at(unix)
}

pub fn format_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn parse_utc(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------------------
// Coordinates and channels (SPEC §2)
// ---------------------------------------------------------------------------

/// A universe is a Z-arm triple "library.shelf.series".
pub fn valid_universe(u: &str) -> bool {
    let parts: Vec<&str> = u.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && *p != "0")
}

fn coord(s: &str) -> Result<phext::Coordinate> {
    if s.trim().is_empty() {
        return Err(Error::InvalidCoordinate(s.to_string()));
    }
    Ok(phext::to_coordinate(s))
}

/// Probe channel coordinate for slot n in a universe: Z/1.1.1/1.1.n
pub fn probe_coord(universe: &str, slot: usize) -> Result<phext::Coordinate> {
    coord(&format!("{}/1.1.1/1.1.{}", universe, slot))
}

/// Echo channel coordinate: Z/1.1.1/2.<probe slot>.<k>
pub fn echo_coord(universe: &str, probe_slot: usize, k: usize) -> Result<phext::Coordinate> {
    coord(&format!("{}/1.1.1/2.{}.{}", universe, probe_slot, k))
}

/// Signature registry coordinate: Z/1.1.1/3.1.n
pub fn registry_coord(universe: &str, n: usize) -> Result<phext::Coordinate> {
    coord(&format!("{}/1.1.1/3.1.{}", universe, n))
}

// ---------------------------------------------------------------------------
// Signature (SPEC §7) — a claim, not a proof; v1 has no cryptographic signing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    /// Full coordinate the program claims as its own, medium-relative.
    pub home: String,
    pub capabilities: Vec<String>,
}

impl Signature {
    pub fn new(name: &str, home: &str, capabilities: Vec<String>) -> Result<Signature> {
        coord(home)?; // validate the home coordinate parses
        Ok(Signature {
            name: name.to_string(),
            home: home.to_string(),
            capabilities,
        })
    }

    pub fn offers(&self, intent: &str) -> bool {
        intent == "any" || self.capabilities.iter().any(|c| c == intent)
    }

    fn capabilities_csv(&self) -> String {
        self.capabilities.join(", ")
    }
}

/// The signature scroll a program publishes at its home coordinate (SPEC §7).
pub fn signature_scroll(sig: &Signature) -> String {
    let now = now_unix();
    format!(
        "echolocation/signature 1\nname: {}\nhome: {}\ncapabilities: {}\npublished-utc: {}\npublished-ee: {}\n",
        sig.name,
        sig.home,
        sig.capabilities_csv(),
        format_utc(now),
        ee_at(now)
    )
}

// ---------------------------------------------------------------------------
// Probe and echo scrolls (SPEC §3, §4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectMode {
    First,
    All,
}

impl CollectMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            CollectMode::First => "first",
            CollectMode::All => "all",
        }
    }
    pub fn parse(s: &str) -> CollectMode {
        if s == "first" {
            CollectMode::First
        } else {
            CollectMode::All
        }
    }
}

#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub from: Signature,
    pub universe: String,
    pub intent: String,
    /// Full coordinate of echo index 1 (in the prober's universe).
    pub reply_at: String,
    pub reply_universe: String,
    pub slot: usize,
    pub mode: CollectMode,
    pub issued_unix: i64,
    pub expires_unix: i64,
    pub poll_ms: u64,
}

impl Probe {
    pub fn render(&self) -> String {
        format!(
            "echolocation/probe 1\nprobe-id: {}\nfrom-name: {}\nfrom-home: {}\nfrom-capabilities: {}\nuniverse: {}\nintent: {}\nreply-at: {}\nmode: {}\nissued-utc: {}\nissued-ee: {}\nexpires-utc: {}\npoll-ms: {}\n",
            self.id,
            self.from.name,
            self.from.home,
            self.from.capabilities_csv(),
            self.universe,
            self.intent,
            self.reply_at,
            self.mode.as_str(),
            format_utc(self.issued_unix),
            ee_at(self.issued_unix),
            format_utc(self.expires_unix),
            self.poll_ms
        )
    }

    pub fn parse(text: &str) -> Option<Probe> {
        let fields = parse_fields(text, "echolocation/probe 1")?;
        let from = Signature {
            name: fields.get("from-name")?.clone(),
            home: fields.get("from-home")?.clone(),
            capabilities: split_csv(fields.get("from-capabilities").map(|s| s.as_str()).unwrap_or("")),
        };
        let reply_at = fields.get("reply-at")?.clone();
        let reply_coord = phext::to_coordinate(&reply_at);
        let slot = reply_coord.x.section;
        Some(Probe {
            id: fields.get("probe-id")?.clone(),
            from,
            universe: fields.get("universe")?.clone(),
            intent: fields.get("intent").cloned().unwrap_or_else(|| "any".to_string()),
            reply_at,
            reply_universe: format!(
                "{}.{}.{}",
                reply_coord.z.library, reply_coord.z.shelf, reply_coord.z.series
            ),
            slot,
            mode: CollectMode::parse(fields.get("mode").map(|s| s.as_str()).unwrap_or("all")),
            issued_unix: parse_utc(fields.get("issued-utc")?)?,
            expires_unix: parse_utc(fields.get("expires-utc")?)?,
            poll_ms: fields.get("poll-ms")?.parse().ok()?,
        })
    }

    pub fn expired(&self, at_unix: i64) -> bool {
        at_unix > self.expires_unix
    }
}

#[derive(Debug, Clone)]
pub struct Echo {
    pub probe_id: String,
    pub from: Signature,
    pub home_universe: String,
    pub state: Option<String>,
    pub mailbox: Option<String>,
    pub issued_unix: i64,
    pub poll_ms: u64,
    /// Echo index k within the probe's echo channel.
    pub index: usize,
}

impl Echo {
    pub fn render(&self) -> String {
        let mut out = format!(
            "echolocation/echo 1\nprobe-id: {}\nfrom-name: {}\nfrom-home: {}\nhome-universe: {}\ncapabilities: {}\n",
            self.probe_id,
            self.from.name,
            self.from.home,
            self.home_universe,
            self.from.capabilities_csv()
        );
        if let Some(state) = &self.state {
            out += &format!("state: {}\n", state);
        }
        out += &format!(
            "mailbox: {}\nissued-utc: {}\nissued-ee: {}\npoll-ms: {}\n",
            self.mailbox.as_deref().unwrap_or("none"),
            format_utc(self.issued_unix),
            ee_at(self.issued_unix),
            self.poll_ms
        );
        out
    }

    pub fn parse(text: &str, index: usize) -> Option<Echo> {
        let fields = parse_fields(text, "echolocation/echo 1")?;
        let from = Signature {
            name: fields.get("from-name")?.clone(),
            home: fields.get("from-home")?.clone(),
            capabilities: split_csv(fields.get("capabilities").map(|s| s.as_str()).unwrap_or("")),
        };
        let mailbox = match fields.get("mailbox").map(|s| s.as_str()) {
            Some("none") | None => None,
            Some(m) => Some(m.to_string()),
        };
        Some(Echo {
            probe_id: fields.get("probe-id")?.clone(),
            from,
            home_universe: fields.get("home-universe")?.clone(),
            state: fields.get("state").cloned(),
            mailbox,
            issued_unix: parse_utc(fields.get("issued-utc")?)?,
            poll_ms: fields.get("poll-ms")?.parse().ok()?,
            index,
        })
    }
}

fn parse_fields(text: &str, header: &str) -> Option<HashMap<String, String>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != header {
        return None;
    }
    let mut map = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    Some(map)
}

fn split_csv(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    s.split(',').map(|p| p.trim().to_string()).collect()
}

// ---------------------------------------------------------------------------
// The medium (SPEC §2). A Medium is one phext document, read and written at
// scroll granularity. FileMedium serializes writes with a lock file and
// commits by rename, so two processes can share one .phext file safely.
// ---------------------------------------------------------------------------

pub trait Medium {
    /// Whole-document read. A missing medium reads as an empty document.
    fn read(&mut self) -> Result<String>;
    /// Locked read-modify-write. The closure receives the current document
    /// and returns the new document plus a value.
    fn update<T>(&mut self, f: &mut dyn FnMut(&str) -> Result<(String, T)>) -> Result<T>;
}

pub struct FileMedium {
    pub path: PathBuf,
    pub lock_timeout: Duration,
}

impl FileMedium {
    pub fn new(path: PathBuf) -> FileMedium {
        FileMedium {
            path,
            lock_timeout: Duration::from_secs(10),
        }
    }

    fn lock_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".lock");
        PathBuf::from(p)
    }

    fn acquire_lock(&self) -> Result<LockGuard> {
        let lock = self.lock_path();
        let start = Instant::now();
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(_) => return Ok(LockGuard { path: lock }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if start.elapsed() > self.lock_timeout {
                        return Err(Error::LockTimeout(lock));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(Error::Io(e)),
            }
        }
    }

    fn read_unlocked(&self) -> Result<String> {
        match fs::read_to_string(&self.path) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Medium for FileMedium {
    fn read(&mut self) -> Result<String> {
        self.read_unlocked()
    }

    fn update<T>(&mut self, f: &mut dyn FnMut(&str) -> Result<(String, T)>) -> Result<T> {
        let _guard = self.acquire_lock()?;
        let current = self.read_unlocked()?;
        let (new_doc, value) = f(&current)?;
        if new_doc != current {
            let mut tmp = self.path.clone().into_os_string();
            tmp.push(format!(".tmp-{}", std::process::id()));
            let tmp = PathBuf::from(tmp);
            fs::write(&tmp, new_doc.as_bytes())?;
            fs::rename(&tmp, &self.path)?;
        }
        Ok(value)
    }
}

/// An in-memory medium, for tests and embedded use.
pub struct MemoryMedium {
    pub doc: String,
}

impl MemoryMedium {
    pub fn new() -> MemoryMedium {
        MemoryMedium { doc: String::new() }
    }
}

impl Medium for MemoryMedium {
    fn read(&mut self) -> Result<String> {
        Ok(self.doc.clone())
    }

    fn update<T>(&mut self, f: &mut dyn FnMut(&str) -> Result<(String, T)>) -> Result<T> {
        let (new_doc, value) = f(&self.doc)?;
        self.doc = new_doc;
        Ok(value)
    }
}

// ---------------------------------------------------------------------------
// Prober side (SPEC §6.1)
// ---------------------------------------------------------------------------

pub struct ProbeHandle {
    pub probe: Probe,
    pub emitted_at: Instant,
}

pub enum ExchangeOutcome {
    Echoed,
    Expired,
}

pub struct Exchange {
    pub outcome: ExchangeOutcome,
    /// Milliseconds from just before the probe write committed to the first
    /// valid echo observed. An upper bound on the round trip (SPEC §5).
    pub range_ms: Option<u64>,
    pub echoes: Vec<Echo>,
}

/// Emit a probe into the target universe's probe channel.
/// `home_universe` is the prober's own universe — the reply lands there.
pub fn emit_probe<M: Medium>(
    medium: &mut M,
    from: &Signature,
    home_universe: &str,
    target_universe: &str,
    intent: &str,
    ttl: Duration,
    mode: CollectMode,
    poll_ms: u64,
) -> Result<ProbeHandle> {
    if !valid_universe(home_universe) {
        return Err(Error::InvalidUniverse(home_universe.to_string()));
    }
    if !valid_universe(target_universe) {
        return Err(Error::InvalidUniverse(target_universe.to_string()));
    }
    let emitted_at = Instant::now();
    let issued = now_unix();
    let from = from.clone();
    let intent = intent.to_string();
    let target = target_universe.to_string();
    let home = home_universe.to_string();
    let probe = medium.update(&mut |doc| {
        // First free probe slot in the target universe (append-only).
        let mut slot = 1usize;
        loop {
            let existing = phext::fetch(doc, probe_coord(&target, slot)?);
            if existing.is_empty() {
                break;
            }
            slot += 1;
        }
        let nonce = {
            let seed = format!("{}:{}:{}", from.name, issued, slot);
            let h = xxhash_rust::xxh64::xxh64(seed.as_bytes(), 0);
            (h & 0xffff) as u16
        };
        let id = format!(
            "{}@{}#{}-{}-{:04x}",
            from.name,
            target,
            ee_at(issued),
            slot,
            nonce
        );
        let reply_at = echo_coord(&home, slot, 1)?.to_string();
        let probe = Probe {
            id,
            from: from.clone(),
            universe: target.clone(),
            intent: intent.clone(),
            reply_at,
            reply_universe: home.clone(),
            slot,
            mode,
            issued_unix: issued,
            expires_unix: issued + ttl.as_secs() as i64,
            poll_ms,
        };
        let new_doc = phext::replace(doc, probe_coord(&target, slot)?, &probe.render());
        // Read-back: the slot must hold our own probe-id (SPEC §2.3).
        let back = phext::fetch(&new_doc, probe_coord(&target, slot)?);
        if !back.contains(&format!("probe-id: {}", probe.id)) {
            return Err(Error::Medium("probe read-back mismatch".to_string()));
        }
        Ok((new_doc, probe))
    })?;
    Ok(ProbeHandle { probe, emitted_at })
}

/// Poll the echo channel until the first echo (mode=first), until expiry
/// collecting every echo (mode=all), or until expiry with none (Expired).
pub fn await_echo<M: Medium>(medium: &mut M, handle: &ProbeHandle) -> Result<Exchange> {
    let probe = &handle.probe;
    let mut echoes: Vec<Echo> = Vec::new();
    let mut range_ms: Option<u64> = None;
    loop {
        let doc = medium.read()?;
        let mut k = 1usize;
        loop {
            let text = phext::fetch(&doc, echo_coord(&probe.reply_universe, probe.slot, k)?);
            if text.is_empty() {
                break; // echo indices are filled in order; first gap ends the list
            }
            if echoes.iter().all(|e: &Echo| e.index != k) {
                if let Some(echo) = Echo::parse(&text, k) {
                    if echo.probe_id == probe.id {
                        if range_ms.is_none() {
                            range_ms = Some(handle.emitted_at.elapsed().as_millis() as u64);
                        }
                        echoes.push(echo);
                        if probe.mode == CollectMode::First {
                            return Ok(Exchange {
                                outcome: ExchangeOutcome::Echoed,
                                range_ms,
                                echoes,
                            });
                        }
                    }
                }
                // Malformed or foreign scrolls at a channel coordinate are
                // skipped, never fatal (SPEC §6.3).
            }
            k += 1;
        }
        if now_unix() > probe.expires_unix {
            return Ok(Exchange {
                outcome: if echoes.is_empty() {
                    ExchangeOutcome::Expired
                } else {
                    ExchangeOutcome::Echoed
                },
                range_ms,
                echoes,
            });
        }
        std::thread::sleep(Duration::from_millis(probe.poll_ms.max(10)));
    }
}

// ---------------------------------------------------------------------------
// Responder side (SPEC §6.2)
// ---------------------------------------------------------------------------

pub struct AnswerInfo {
    pub probe_id: String,
    pub prober_name: String,
    pub echo_coord: String,
}

/// Publish this program's signature scroll at its home coordinate.
pub fn publish_signature<M: Medium>(medium: &mut M, sig: &Signature) -> Result<()> {
    let home = coord(&sig.home)?;
    let scroll = signature_scroll(sig);
    medium.update(&mut |doc| Ok((phext::replace(doc, home, &scroll), ())))?;
    Ok(())
}

/// Scan the probe channels of the universes this program inhabits and answer
/// at most one outstanding probe. Returns Some when an echo was written.
pub fn answer_once<M: Medium>(
    medium: &mut M,
    sig: &Signature,
    home_universe: &str,
    inhabits: &[String],
    state: Option<&str>,
    mailbox: Option<&str>,
    poll_ms: u64,
) -> Result<Option<AnswerInfo>> {
    let doc = medium.read()?;
    let now = now_unix();
    for universe in inhabits {
        let mut slot = 1usize;
        loop {
            let text = phext::fetch(&doc, probe_coord(universe, slot)?);
            if text.is_empty() {
                break;
            }
            slot += 1;
            let probe = match Probe::parse(&text) {
                Some(p) => p,
                None => continue, // malformed scroll: not a probe (SPEC §10)
            };
            if probe.expired(now) {
                continue; // never answer an expired probe
            }
            if !sig.offers(&probe.intent) {
                continue; // intent does not intersect our capabilities
            }
            // Already answered? The medium is the record (SPEC §6.2):
            // an echo naming this probe-id from us ends the question.
            let mut k = 1usize;
            let mut answered = false;
            loop {
                let etext = phext::fetch(
                    &doc,
                    echo_coord(&probe.reply_universe, probe.slot, k)?,
                );
                if etext.is_empty() {
                    break;
                }
                if let Some(e) = Echo::parse(&etext, k) {
                    if e.probe_id == probe.id && e.from.name == sig.name {
                        answered = true;
                        break;
                    }
                }
                k += 1;
            }
            if answered {
                continue;
            }
            // Write the echo at the first free index, under the medium lock,
            // re-checking freeness inside the lock.
            let sig2 = sig.clone();
            let state2 = state.map(|s| s.to_string());
            let mailbox2 = mailbox.map(|s| s.to_string());
            let home_universe2 = home_universe.to_string();
            let probe2 = probe.clone();
            let info = medium.update(&mut |cur| {
                let mut k = 1usize;
                loop {
                    let existing =
                        phext::fetch(cur, echo_coord(&probe2.reply_universe, probe2.slot, k)?);
                    if existing.is_empty() {
                        break;
                    }
                    k += 1;
                }
                let echo = Echo {
                    probe_id: probe2.id.clone(),
                    from: sig2.clone(),
                    home_universe: home_universe2.clone(),
                    state: state2.clone(),
                    mailbox: mailbox2.clone(),
                    issued_unix: now_unix(),
                    poll_ms,
                    index: k,
                };
                let at = echo_coord(&probe2.reply_universe, probe2.slot, k)?;
                let new_doc = phext::replace(cur, at, &echo.render());
                Ok((
                    new_doc,
                    AnswerInfo {
                        probe_id: probe2.id.clone(),
                        prober_name: probe2.from.name.clone(),
                        echo_coord: at.to_string(),
                    },
                ))
            })?;
            return Ok(Some(info));
        }
    }
    Ok(None)
}
