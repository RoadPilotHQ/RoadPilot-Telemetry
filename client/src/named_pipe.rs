//! Windows local named-pipe server for framed telemetry JSON.

use crate::{DecodeError, MAX_FRAME_BYTES, ReceivedTelemetryFrame, decode_frame};
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

pub const PIPE_NAME: &str = r"\\.\pipe\RoadPilot.Telemetry.v1";
const FRAME_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);

/// Accept one local plugin connection at a time and publish only the newest
/// validated frame. Call this from a Tokio runtime on Windows.
pub async fn serve_latest(latest: watch::Sender<Option<ReceivedTelemetryFrame>>) -> io::Result<()> {
    let mut server = create_pipe_server()?;
    loop {
        server.connect().await?;
        let mut previous_sequence = None;

        loop {
            match read_frame(&mut server).await {
                Ok(frame) if previous_sequence.is_none_or(|last| frame.sequence > last) => {
                    previous_sequence = Some(frame.sequence);
                    latest.send_replace(Some(ReceivedTelemetryFrame {
                        frame,
                        received_at: std::time::Instant::now(),
                    }));
                }
                Ok(_) | Err(_) => break,
            }
        }

        let _ = server.disconnect();
    }
}

fn create_pipe_server() -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .access_outbound(false)
        .max_instances(1)
        .first_pipe_instance(true)
        .reject_remote_clients(true);

    // Allow the local game process to write even when it runs with a different
    // token from the desktop reader. Remote clients remain rejected below.
    let sddl: Vec<u16> = "D:P(A;;GRGW;;;AU)".encode_utf16().chain(Some(0)).collect();
    let mut security_descriptor: *mut c_void = null_mut();
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
    let server_result = unsafe {
        options.create_with_security_attributes_raw(
            PIPE_NAME,
            &mut security_attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
        )
    };
    unsafe { LocalFree(security_descriptor) };
    server_result
}

async fn read_frame<R>(reader: &mut R) -> io::Result<crate::TelemetryFrameV1>
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
