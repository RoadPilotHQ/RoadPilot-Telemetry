//! Small console host for the Windows telemetry pipe reader.

#[cfg(not(windows))]
compile_error!("The RoadPilot telemetry reader currently requires Windows named pipes.");

use roadpilot_telemetry_client::{ReceivedTelemetryFrame, named_pipe::serve_latest};
use std::{
    io::{self, Write},
    time::Duration,
};
use tokio::{runtime::Builder, sync::watch, time::interval};

const STALE_AFTER: Duration = Duration::from_secs(2);

fn main() -> io::Result<()> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> io::Result<()> {
    let (sender, mut latest) = watch::channel(None::<ReceivedTelemetryFrame>);
    let mut server = tokio::spawn(serve_latest(sender));
    let mut refresh = interval(Duration::from_millis(250));

    println!("RoadPilot Telemetry Reader — listening on \\\\.\\pipe\\RoadPilot.Telemetry.v1");
    println!("Press Ctrl+C to stop.");

    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                println!("\nStopping reader...");
                server.abort();
                let _ = server.await;
                return Ok(());
            }
            result = &mut server => {
                return match result {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(io::Error::other(format!("reader task failed: {error}"))),
                };
            }
            _ = refresh.tick() => render_status(&mut latest),
        }
    }
}

fn render_status(latest: &mut watch::Receiver<Option<ReceivedTelemetryFrame>>) {
    let line = match latest.borrow_and_update().as_ref() {
        None => "Waiting for the game plugin…".to_owned(),
        Some(received) => {
            let age = received.received_at.elapsed();
            if age > STALE_AFTER {
                format!(
                    "STALE · last frame {:.1}s ago · {}",
                    age.as_secs_f32(),
                    game_label(received)
                )
            } else {
                let frame = &received.frame;
                let truck = frame.truck.unwrap_or_default();
                let dash = || "—".to_owned();
                let speed = truck
                    .speed_mps
                    .map(|v| format!("{v:.1} m/s ({:.0} km/h)", v * 3.6))
                    .unwrap_or_else(dash);
                let rpm = truck
                    .engine_rpm
                    .map(|v| format!("{v:.0} rpm"))
                    .unwrap_or_else(dash);
                let gear = truck.gear.map(|v| v.to_string()).unwrap_or_else(dash);
                let fuel = truck
                    .fuel_l
                    .map(|v| format!("{v:.0} l"))
                    .unwrap_or_else(dash);
                let position = frame
                    .placement
                    .map(|p| format!("x {:.0} z {:.0}", p.x, p.z))
                    .unwrap_or_else(dash);
                format!(
                    "LIVE · {} · v{} · {} · {} · gear {} · fuel {} · {} · seq {}",
                    game_label(received),
                    frame.schema_version,
                    speed,
                    rpm,
                    gear,
                    fuel,
                    position,
                    frame.sequence
                )
            }
        }
    };
    print!("\r{line:<110}");
    let _ = io::stdout().flush();
}

fn game_label(received: &ReceivedTelemetryFrame) -> &'static str {
    match received.frame.game {
        roadpilot_telemetry_client::GameId::Ets2 => "ETS2",
        roadpilot_telemetry_client::GameId::Ats => "ATS",
    }
}
