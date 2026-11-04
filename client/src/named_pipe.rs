//! Windows local named-pipe server for framed telemetry JSON.
//!
//! Framing: 4-byte little-endian payload length, then that many bytes of UTF-8 JSON.
//! The plugin keeps one connection open and writes frames continuously; when the connection
//! breaks it reconnects. The server accepts one client at a time and waits for the next
//! client after a disconnect.

use crate::ordering::FrameOrder;
use crate::{DecodeError, DecodedFrame, MAX_FRAME_BYTES, ReceivedTelemetryFrame, decode_frame};
use std::{ffi::c_void, io, mem::size_of, ptr::null_mut, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::watch,
    time::timeout,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SECURITY_ATTRIBUTES,
    },
};

/// Endpoint name. The `.v1` suffix names the transport (framing), which is unchanged in
/// schema 2; the schema version travels inside every frame.
pub const PIPE_NAME: &str = r"\\.\pipe\RoadPilot.Telemetry.v1";
const FRAME_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);

/// Serve [`PIPE_NAME`]. See [`serve_latest_on`].
pub async fn serve_latest(latest: watch::Sender<Option<ReceivedTelemetryFrame>>) -> io::Result<()> {
    serve_latest_on(PIPE_NAME, latest).await
}

/// Accept one local plugin connection at a time and publish only the newest validated,
/// in-order frame. Returns only on a pipe error (for example when another reader already
/// owns the pipe). Call this from a Tokio runtime on Windows.
pub async fn serve_latest_on(
    pipe_name: &str,
    latest: watch::Sender<Option<ReceivedTelemetryFrame>>,
) -> io::Result<()> {
    let mut order = FrameOrder::default();
    let mut server = create_pipe_server(pipe_name)?;
    loop {
        server.connect().await?;
        // A broken or malformed stream ends this connection; the plugin reconnects.
        let _ = read_connection(&mut server, &mut order, &latest).await;
        order.connection_closed();
        let _ = server.disconnect();
        drop(server);
        // A fresh instance per client: a reused instance can return the previous client's
        // end-of-stream error on its first read and drop the next client's first frames.
        server = recreate_pipe_server(pipe_name).await?;
    }
}

/// The dropped instance's handle is closed only once its pending overlapped I/O has been
/// processed by the runtime, so creating the next first instance can briefly fail with
/// "access denied". Retry for up to one second; after that the name is genuinely taken.
async fn recreate_pipe_server(pipe_name: &str) -> io::Result<NamedPipeServer> {
    const ATTEMPTS: u32 = 100;
    let mut attempt = 0;
    loop {
        match create_pipe_server(pipe_name) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < ATTEMPTS => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            result => return result,
        }
    }
}

/// Reads frames from one connection until it ends or delivers a malformed frame.
/// Out-of-order frames are skipped without closing the connection.
pub async fn read_connection<R>(
    reader: &mut R,
    order: &mut FrameOrder,
    latest: &watch::Sender<Option<ReceivedTelemetryFrame>>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let DecodedFrame {
            frame,
            dropped_fields,
        } = read_frame(reader).await?;
        if order.accept(frame.session, frame.sequence) {
            latest.send_replace(Some(ReceivedTelemetryFrame {
                frame,
                dropped_fields,
                received_at: std::time::Instant::now(),
            }));
        }
    }
}

fn create_pipe_server(pipe_name: &str) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .access_outbound(false)
        .max_instances(1)
        .first_pipe_instance(true)
        .reject_remote_clients(true);

    // Allow the local game process to write even when it runs with a different
    // token from the desktop reader. Remote clients remain rejected above.
    let sddl: Vec<u16> = "D:P(A;;GRGW;;;AU)".encode_utf16().chain(Some(0)).collect();
    let mut security_descriptor: *mut c_void = null_mut();
    // SAFETY: `sddl` is a NUL-terminated UTF-16 string; the descriptor is freed below.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut security_descriptor,
            null_mut(),
        )
    };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: the attributes point to a valid descriptor for the duration of the call.
    let server_result = unsafe {
        options.create_with_security_attributes_raw(
            pipe_name,
            &mut security_attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
        )
    };
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(security_descriptor) };
    server_result
}

async fn read_frame<R>(reader: &mut R) -> io::Result<DecodedFrame>
where
    R: AsyncRead + Unpin,
{
    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes[..1]).await?;
    timeout(
        FRAME_COMPLETION_TIMEOUT,
        reader.read_exact(&mut length_bytes[1..]),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "incomplete telemetry frame header"))??;

    let length = u32::from_le_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "telemetry frame length is outside the accepted range",
        ));
    }

    let mut payload = vec![0_u8; length];
    timeout(FRAME_COMPLETION_TIMEOUT, reader.read_exact(&mut payload))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "incomplete telemetry frame"))??;

    decode_frame(&payload).map_err(decode_error_to_io)
}

fn decode_error_to_io(error: DecodeError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid telemetry frame: {error:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn frame_json(session: u64, sequence: u64) -> String {
        format!(
            r#"{{"protocol":"roadpilot.telemetry","schemaVersion":2,"session":{session},"sequence":{sequence},"game":"ats","pluginState":"running","truck":{{"speedMps":{sequence}}}}}"#
        )
    }

    fn framed(json: &str) -> Vec<u8> {
        let mut bytes = (json.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        bytes
    }

    fn unique_pipe() -> String {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\RoadPilot.Telemetry.test.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Behaves like the plugin: synchronous open, write and close of the pipe handle.
    struct Client(std::fs::File);

    impl Client {
        async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.write_all(bytes)
        }
    }

    async fn connect(name: &str) -> Client {
        let mut last = None;
        for _ in 0..200 {
            match std::fs::OpenOptions::new().write(true).open(name) {
                Ok(file) => return Client(file),
                Err(e) => last = Some(e),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("pipe {name} did not become available: {last:?}");
    }

    async fn wait_for_sequence(
        rx: &mut watch::Receiver<Option<ReceivedTelemetryFrame>>,
        sequence: u64,
    ) {
        timeout(Duration::from_secs(5), async {
            loop {
                if rx.borrow().as_ref().map(|f| f.frame.sequence) == Some(sequence) {
                    return;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("frame not received");
    }

    #[tokio::test]
    async fn persistent_connection_delivers_many_frames() {
        let name = unique_pipe();
        let (tx, mut rx) = watch::channel(None);
        let server = tokio::spawn({
            let name = name.clone();
            async move { serve_latest_on(&name, tx).await }
        });
        let mut client = connect(&name).await;
        for sequence in 1..=50 {
            client
                .write_all(&framed(&frame_json(7, sequence)))
                .await
                .unwrap();
        }
        wait_for_sequence(&mut rx, 50).await;
        let latest = rx.borrow().clone().unwrap();
        assert_eq!(latest.frame.session, Some(7));
        assert_eq!(latest.frame.truck.unwrap().speed_mps, Some(50.0));
        server.abort();
    }

    #[tokio::test]
    async fn reconnect_keeps_ordering_and_accepts_a_new_session() {
        let name = unique_pipe();
        let (tx, mut rx) = watch::channel(None);
        let server = tokio::spawn({
            let name = name.clone();
            async move { serve_latest_on(&name, tx).await }
        });

        let mut client = connect(&name).await;
        client.write_all(&framed(&frame_json(7, 10))).await.unwrap();
        wait_for_sequence(&mut rx, 10).await;
        drop(client);

        // Same session after reconnect: an older frame is dropped, a newer one accepted.
        let mut client = connect(&name).await;
        client.write_all(&framed(&frame_json(7, 9))).await.unwrap();
        client.write_all(&framed(&frame_json(7, 11))).await.unwrap();
        wait_for_sequence(&mut rx, 11).await;
        drop(client);

        // A restarted plugin uses a new session and may start at 1 again.
        let mut client = connect(&name).await;
        client.write_all(&framed(&frame_json(8, 1))).await.unwrap();
        wait_for_sequence(&mut rx, 1).await;
        assert_eq!(rx.borrow().as_ref().unwrap().frame.session, Some(8));
        server.abort();
    }

    #[tokio::test]
    async fn malformed_frame_closes_only_that_connection() {
        let name = unique_pipe();
        let (tx, mut rx) = watch::channel(None);
        let server = tokio::spawn({
            let name = name.clone();
            async move { serve_latest_on(&name, tx).await }
        });

        let mut client = connect(&name).await;
        client.write_all(&framed("{not json")).await.unwrap();
        drop(client);

        let mut client = connect(&name).await;
        client.write_all(&framed(&frame_json(3, 1))).await.unwrap();
        wait_for_sequence(&mut rx, 1).await;
        server.abort();
    }

    #[tokio::test]
    async fn accepts_v1_frames_from_older_plugins() {
        let name = unique_pipe();
        let (tx, mut rx) = watch::channel(None);
        let server = tokio::spawn({
            let name = name.clone();
            async move { serve_latest_on(&name, tx).await }
        });
        // The v1 plugin opens one connection per frame.
        for sequence in [4, 5] {
            let mut client = connect(&name).await;
            let v1 = format!(
                r#"{{"protocol":"roadpilot.telemetry","schemaVersion":1,"sequence":{sequence},"game":"ets2","pluginState":"running","values":{{"gear":3}}}}"#
            );
            client.write_all(&framed(&v1)).await.unwrap();
            wait_for_sequence(&mut rx, sequence).await;
        }
        let frame = rx.borrow().clone().unwrap().frame;
        assert_eq!(frame.schema_version, 1);
        assert_eq!(frame.truck.unwrap().gear, Some(3));
        server.abort();
    }

    #[tokio::test]
    async fn a_second_server_on_the_same_pipe_fails() {
        let name = unique_pipe();
        let (tx, _rx) = watch::channel(None);
        let first = tokio::spawn({
            let name = name.clone();
            async move { serve_latest_on(&name, tx).await }
        });
        let _client = connect(&name).await;
        let (tx2, _rx2) = watch::channel(None);
        assert!(serve_latest_on(&name, tx2).await.is_err());
        first.abort();
    }

    #[tokio::test]
    async fn rejects_oversized_length_prefix() {
        let (tx, _rx) = watch::channel(None);
        let mut order = FrameOrder::default();
        let bytes = ((MAX_FRAME_BYTES + 1) as u32).to_le_bytes();
        let mut reader = &bytes[..];
        let error = read_connection(&mut reader, &mut order, &tx)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
