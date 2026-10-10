//! Native Messaging host that bridges the WebTerm extension to a serial port.
//!
//! Framing (both directions): 4-byte native-endian length, then UTF-8 JSON.
//! Serial payloads travel base64-encoded in the `data` field.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, SerialPort, SerialPortType, StopBits};

/// Read timeout; also bounds how long `close` waits for the reader thread.
const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// Well below Chrome's 1 MB host-to-browser message limit once base64-encoded.
const READ_CHUNK: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    List,
    Open(OpenParams),
    Write { data: String },
    Signals { dtr: Option<bool>, rts: Option<bool> },
    Close,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenParams {
    path: String,
    #[serde(default = "default_baud")]
    baud: u32,
    #[serde(default = "default_data_bits")]
    data_bits: u8,
    #[serde(default = "default_parity")]
    parity: String,
    #[serde(default = "default_stop_bits")]
    stop_bits: u8,
    #[serde(default = "default_flow")]
    flow: String,
}

fn default_baud() -> u32 {
    115_200
}
fn default_data_bits() -> u8 {
    8
}
fn default_parity() -> String {
    "none".into()
}
fn default_stop_bits() -> u8 {
    1
}
fn default_flow() -> String {
    "none".into()
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Response {
    Ports { ports: Vec<PortEntry> },
    Opened { path: String },
    Data { data: String },
    Closed { reason: Option<String> },
    Error { message: String },
}

#[derive(Serialize)]
struct PortEntry {
    path: String,
    description: Option<String>,
}

struct Session {
    port: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    reader: JoinHandle<()>,
}

impl Session {
    fn open(params: &OpenParams) -> Result<Self, String> {
        let port = open_port(params)?;
        let reader_port = port.try_clone().map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let reader = thread::spawn({
            let stop = stop.clone();
            move || read_loop(reader_port, stop)
        });
        Ok(Self { port, stop, reader })
    }

    /// True once the reader thread has given up (device unplugged, driver error).
    fn is_dead(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn close(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.reader.join();
    }
}

fn main() {
    let mut stdin = io::stdin().lock();
    let mut session: Option<Session> = None;

    // The browser closes our stdin when the extension disconnects.
    while let Ok(Some(raw)) = read_message(&mut stdin) {
        match serde_json::from_slice::<Request>(&raw) {
            Ok(request) => handle(request, &mut session),
            Err(e) => send(&Response::Error {
                message: format!("bad request: {e}"),
            }),
        }
    }

    if let Some(session) = session {
        session.close();
    }
}

fn handle(request: Request, session: &mut Option<Session>) {
    if session.as_ref().is_some_and(Session::is_dead) {
        session.take().unwrap().close();
    }

    let result = match request {
        Request::List => {
            send(&Response::Ports {
                ports: list_ports(),
            });
            Ok(())
        }
        Request::Open(params) => {
            if let Some(old) = session.take() {
                old.close();
                send(&Response::Closed { reason: None });
            }
            Session::open(&params).map(|new| {
                *session = Some(new);
                send(&Response::Opened { path: params.path });
            })
        }
        Request::Write { data } => with_port(session, |port| {
            let bytes = B64.decode(data).map_err(|e| e.to_string())?;
            port.write_all(&bytes).map_err(|e| e.to_string())
        }),
        Request::Signals { dtr, rts } => with_port(session, |port| {
            if let Some(level) = dtr {
                port.write_data_terminal_ready(level)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(level) = rts {
                port.write_request_to_send(level)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }),
        Request::Close => {
            if let Some(old) = session.take() {
                old.close();
                send(&Response::Closed { reason: None });
            }
            Ok(())
        }
    };

    if let Err(message) = result {
        send(&Response::Error { message });
    }
}

fn with_port(
    session: &mut Option<Session>,
    f: impl FnOnce(&mut dyn SerialPort) -> Result<(), String>,
) -> Result<(), String> {
    match session {
        Some(session) => f(session.port.as_mut()),
        None => Err("port is not open".into()),
    }
}

fn open_port(params: &OpenParams) -> Result<Box<dyn SerialPort>, String> {
    let data_bits = match params.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        8 => DataBits::Eight,
        other => return Err(format!("unsupported dataBits: {other}")),
    };
    let parity = match params.parity.as_str() {
        "none" => Parity::None,
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        other => return Err(format!("unsupported parity: {other}")),
    };
    let stop_bits = match params.stop_bits {
        1 => StopBits::One,
        2 => StopBits::Two,
        other => return Err(format!("unsupported stopBits: {other}")),
    };
    let flow = match params.flow.as_str() {
        "none" => FlowControl::None,
        "software" => FlowControl::Software,
        "hardware" => FlowControl::Hardware,
        other => return Err(format!("unsupported flow: {other}")),
    };

    let mut port = serialport::new(&params.path, params.baud)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(flow)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| format!("{}: {e}", params.path))?;

    // Assert the modem lines like other terminal programs do; many RS-232
    // devices stay silent until they see them. Not every driver supports it.
    let _ = port.write_data_terminal_ready(true);
    if flow != FlowControl::Hardware {
        let _ = port.write_request_to_send(true);
    }
    Ok(port)
}

fn read_loop(mut port: Box<dyn SerialPort>, stop: Arc<AtomicBool>) {
    let mut buf = vec![0u8; READ_CHUNK];
    while !stop.load(Ordering::Relaxed) {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => send(&Response::Data {
                data: B64.encode(&buf[..n]),
            }),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(e) => {
                stop.store(true, Ordering::Relaxed);
                send(&Response::Closed {
                    reason: Some(e.to_string()),
                });
                return;
            }
        }
    }
}

fn list_ports() -> Vec<PortEntry> {
    let mut ports: Vec<PortEntry> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|info| PortEntry {
            description: describe(&info.port_type),
            path: info.port_name,
        })
        .collect();

    // Vendor drivers that skip the standard device class still show up here.
    #[cfg(windows)]
    for path in registry_ports() {
        if !ports.iter().any(|p| p.path.eq_ignore_ascii_case(&path)) {
            ports.push(PortEntry {
                path,
                description: None,
            });
        }
    }

    ports.sort_by(|a, b| (a.path.len(), &a.path).cmp(&(b.path.len(), &b.path)));
    ports
}

fn describe(port_type: &SerialPortType) -> Option<String> {
    match port_type {
        SerialPortType::UsbPort(usb) => Some(
            usb.product
                .clone()
                .or_else(|| usb.manufacturer.clone())
                .unwrap_or_else(|| format!("USB {:04x}:{:04x}", usb.vid, usb.pid)),
        ),
        SerialPortType::BluetoothPort => Some("Bluetooth".into()),
        SerialPortType::PciPort => Some("PCI".into()),
        SerialPortType::Unknown => None,
    }
}

#[cfg(windows)]
fn registry_ports() -> Vec<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"HARDWARE\DEVICEMAP\SERIALCOMM")
    else {
        return Vec::new();
    };
    let ports = key
        .enum_values()
        .filter_map(Result::ok)
        .filter_map(|(name, _)| key.get_value::<String, _>(name).ok())
        .collect();
    ports
}

fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut body = vec![0u8; u32::from_ne_bytes(len) as usize];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one framed message. The stdout lock keeps frames from interleaving
/// between the main thread and the reader thread.
fn send(response: &Response) {
    let body = serde_json::to_vec(response).expect("response serializes");
    let mut out = io::stdout().lock();
    let written = out
        .write_all(&(body.len() as u32).to_ne_bytes())
        .and_then(|()| out.write_all(&body))
        .and_then(|()| out.flush());
    if written.is_err() {
        // The browser is gone; nothing left to talk to.
        std::process::exit(0);
    }
}
