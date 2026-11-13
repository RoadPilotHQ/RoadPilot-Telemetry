//! Are QueryPerformanceCounter values comparable between two processes on this machine?
//!
//! The frame timing in `roadpilot_telemetry_client::qpc` subtracts QPC values taken in the
//! game process (the plugin) from values taken in RoadPilot's process. That is only valid if
//! both processes read one counter with one frequency. These tests check it on the machine
//! they run on:
//!
//! 1. `qpc_is_one_timeline_across_processes` (runs by default): starts this test binary again
//!    as a child process, which reports its QPC frequency and a series of QPC values on
//!    stdout. Each child value must be monotonic, not earlier than the parent's value taken
//!    before the child started, and not later than the parent's value taken right after the
//!    line arrived; the frequencies must be equal.
//! 2. `plugin_stamps_compare_across_processes` (ignored, run explicitly): the real plugin DLL
//!    runs in its own host process (`roadpilot_plugin_host_tests --drive`), and this process
//!    serves the real telemetry pipe and receives its frames. Every frame must carry stamps
//!    that evaluate to a valid timing: same frequency, produced <= sent <= received, small
//!    and plausible durations. It needs the plugin build and a free pipe (RoadPilot and the
//!    console reader closed):
//!
//!    cargo test --test qpc_cross_process -- --ignored --nocapture plugin_stamps
//!
//!    The host executable is found at `../plugin/build/Release/` or through
//!    `ROADPILOT_PLUGIN_HOST`.

#![cfg(windows)]

use roadpilot_telemetry_client::qpc::{self, ticks_to_duration};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "ROADPILOT_QPC_PROBE_CHILD";
const SAMPLES: usize = 20;

/// The child side of test 1. Ignored so it never runs on its own; the parent starts it with
/// `--ignored` and the environment variable set.
#[test]
#[ignore = "child process of qpc_is_one_timeline_across_processes"]
fn qpc_child_probe() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    println!("QPC_FREQUENCY {}", qpc::frequency().expect("frequency"));
    for _ in 0..SAMPLES {
        println!("QPC_VALUE {}", qpc::now().expect("value"));
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn qpc_is_one_timeline_across_processes() {
    let frequency = qpc::frequency().expect("this process has a QPC frequency");
    let before = qpc::now().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "qpc_child_probe",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the child process");
    assert_ne!(child.id(), std::process::id());

    let mut child_frequency = None;
    let mut values = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        // Taken as soon as the line is here: the child's value must not be later.
        let parent_now = qpc::now().unwrap();
        // The test harness may print its own text before ours on the same line.
        let field = |token: &str| {
            line.find(token)
                .map(|at| line[at + token.len()..].trim().to_owned())
        };
        if let Some(f) = field("QPC_FREQUENCY ") {
            child_frequency = Some(f.trim().parse::<u64>().unwrap());
        } else if let Some(v) = field("QPC_VALUE ") {
            values.push((v.trim().parse::<u64>().unwrap(), parent_now));
        }
    }
    assert!(child.wait().unwrap().success(), "the child probe failed");
    let after = qpc::now().unwrap();

    assert_eq!(child_frequency, Some(frequency), "same QPC frequency");
    assert_eq!(values.len(), SAMPLES, "every sample arrived");
    let mut previous = before;
    let mut worst_delivery = Duration::ZERO;
    for &(child_value, parent_now) in &values {
        assert!(
            child_value >= previous,
            "child value {child_value} before the parent's earlier reading {previous}"
        );
        assert!(
            child_value <= parent_now,
            "child value {child_value} after the parent's reading {parent_now} taken later"
        );
        let delivery = ticks_to_duration(parent_now - child_value, frequency).unwrap();
        worst_delivery = worst_delivery.max(delivery);
        previous = child_value;
    }
    assert!(previous <= after);
    // Child stamp to parent reading includes the pipe and scheduling: small, never seconds.
    assert!(
        worst_delivery < Duration::from_millis(500),
        "implausible delivery delay {worst_delivery:?}"
    );
    println!(
        "QPC frequency {frequency} Hz in both processes; {SAMPLES} child values within the \
         parent's timeline, worst child-to-parent delay {worst_delivery:?}"
    );
}

fn plugin_host() -> std::path::PathBuf {
    std::env::var_os("ROADPILOT_PLUGIN_HOST")
        .map(Into::into)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../plugin/build/Release/roadpilot_plugin_host_tests.exe")
        })
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore = "needs the plugin build and a free telemetry pipe; run explicitly"]
fn plugin_stamps_compare_across_processes() {
    use roadpilot_telemetry_client::named_pipe::serve_latest;
    use tokio::sync::watch;

    let host = plugin_host();
    assert!(host.exists(), "plugin host not found at {}", host.display());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let frames = runtime.block_on(async {
        let (tx, mut rx) = watch::channel(None);
        let server = tokio::spawn(serve_latest(tx));
        // Let the server create the pipe before the plugin tries to connect.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !server.is_finished(),
            "the telemetry pipe is in use (close RoadPilot and the console reader)"
        );
        let mut driver = Command::new(&host)
            .args(["--drive", "5"])
            .stdout(Stdio::null())
            .spawn()
            .expect("start the plugin host");
        assert_ne!(driver.id(), std::process::id());
        let mut frames = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() { break; }
                    if let Some(frame) = rx.borrow_and_update().clone() {
                        frames.push(frame);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if driver.try_wait().unwrap().is_some() { break; }
                }
            }
        }
        assert!(driver.wait().unwrap().success(), "the plugin host failed");
        server.abort();
        frames
    });

    assert!(frames.len() >= 100, "only {} frames", frames.len());
    let local = qpc::frequency().unwrap();
    let (mut processing, mut transport, mut age) = (Vec::new(), Vec::new(), Vec::new());
    let mut previous: Option<(u64, u64, u64, u64)> = None;
    for f in &frames {
        assert_eq!(f.frame.qpc_frequency, Some(local), "same QPC frequency");
        let timing = f
            .timing()
            .unwrap_or_else(|e| panic!("frame {}: {e:?}", f.frame.sequence));
        let (p, s, r) = (
            f.frame.produced_qpc.unwrap(),
            f.frame.sent_qpc.unwrap(),
            f.received_qpc.unwrap(),
        );
        assert!(p <= s && s <= r, "frame {}: {p} {s} {r}", f.frame.sequence);
        if let Some((seq, pp, ps, pr)) = previous {
            assert!(f.frame.sequence > seq);
            assert!(
                p > pp && s > ps && r > pr,
                "stamps increase from frame to frame"
            );
        }
        previous = Some((f.frame.sequence, p, s, r));
        // Plausible on an idle host: well below the 150 ms freshness limit.
        assert!(
            timing.age_at_receive < Duration::from_millis(100),
            "{timing:?}"
        );
        processing.push(timing.plugin_processing);
        transport.push(timing.transport);
        age.push(timing.age_at_receive);
    }
    for v in [&mut processing, &mut transport, &mut age] {
        v.sort();
    }
    println!(
        "{} frames from the plugin process; QPC frequency {local} Hz in both processes",
        frames.len()
    );
    for (name, v) in [
        ("plugin processing", &processing),
        ("transport", &transport),
        ("age at receive", &age),
    ] {
        println!(
            "{name}: p50 {:?} p99 {:?} max {:?}",
            percentile(v, 0.5),
            percentile(v, 0.99),
            v[v.len() - 1]
        );
    }
}
