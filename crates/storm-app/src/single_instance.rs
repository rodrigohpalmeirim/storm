//! One Storm per data folder.
//!
//! Two copies sharing a data folder would fight over the engine's database,
//! listen port and DHT state, so a second launch hands its command line (magnet
//! links / `.torrent` paths) to the running copy, which comes to the front, and
//! exits.
//!
//! Windows: a named mutex (keyed by the data folder) says whether a copy is
//! running; the running copy listens on a loopback port, recorded in
//! `<data dir>/instance.port`, for forwarded requests. Other platforms don't
//! detect a running copy yet (they only handle their own command line).

use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App, Window};

use crate::caps::BackendHandle;
use crate::format;
use crate::ui;
use crate::views::dialogs::{AddTorrentDialog, read_torrent_files};

/// What a launch asked Storm to open.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    /// Magnet links and `.torrent` URLs.
    pub links: Vec<String>,
    /// Local `.torrent` files (absolute, so they survive a hop to another process).
    pub files: Vec<PathBuf>,
}

impl OpenRequest {
    /// Parses command-line arguments (without the program name). Anything that
    /// is neither a link nor an existing file is ignored.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut req = Self::default();
        for arg in args {
            let arg = arg.trim();
            if arg.is_empty() {
                continue;
            }
            if format::is_torrent_link(arg) {
                req.links.push(arg.to_owned());
            } else if Path::new(arg).is_file() {
                let p = PathBuf::from(arg);
                req.files.push(std::path::absolute(&p).unwrap_or(p));
            } else {
                tracing::warn!("ignoring command-line argument {arg:?}: not a link or a file");
            }
        }
        req
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.files.is_empty()
    }

    /// One item per line: `L <link>` or `F <path>`.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn encode(&self) -> String {
        let mut out = String::new();
        for l in &self.links {
            out.push_str(&format!("L {l}\n"));
        }
        for f in &self.files {
            out.push_str(&format!("F {}\n", f.display()));
        }
        out
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn decode(text: &str) -> Self {
        let mut req = Self::default();
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("L", l)) if format::is_torrent_link(l) => req.links.push(l.to_owned()),
                Some(("F", f)) if !f.is_empty() => req.files.push(PathBuf::from(f)),
                _ => {}
            }
        }
        req
    }
}

pub enum Launch {
    /// No other copy is running: carry on starting up.
    Primary(Instance),
    /// A running copy took our request (and came to the front): exit.
    Forwarded,
    /// A copy is running but didn't answer: exit rather than clash with it.
    Unreachable,
}

/// Held for the life of the process; dropping it lets another copy start.
pub struct Instance {
    #[cfg(windows)]
    _mutex: win::Mutex,
    requests: Option<mpsc::UnboundedReceiver<OpenRequest>>,
}

impl Instance {
    /// Requests forwarded by later launches (taken once, by the UI).
    pub fn take_requests(&mut self) -> Option<mpsc::UnboundedReceiver<OpenRequest>> {
        self.requests.take()
    }
}

#[cfg(not(windows))]
pub fn acquire(_data_dir: &Path, _req: &OpenRequest) -> Launch {
    Launch::Primary(Instance { requests: None })
}

#[cfg(windows)]
pub fn acquire(data_dir: &Path, req: &OpenRequest) -> Launch {
    use std::time::Duration;

    let name = format!("Local\\storm-{:016x}", fnv1a(data_dir.to_string_lossy().to_lowercase().as_bytes()));
    // The running copy may be starting (no port file yet) or shutting down
    // (still holding the mutex), so keep trying for a while before giving up.
    for _ in 0..30 {
        let Some((mutex, existed)) = win::Mutex::create(&name) else {
            // Can't tell; behave as before single-instance support.
            tracing::warn!("couldn't create the single-instance mutex");
            return Launch::Primary(Instance { _mutex: win::Mutex::none(), requests: None });
        };
        if !existed {
            return Launch::Primary(Instance { _mutex: mutex, requests: ipc::serve(data_dir) });
        }
        drop(mutex);
        win::allow_foreground();
        match ipc::forward(data_dir, req) {
            Ok(()) => {
                tracing::info!("Storm is already running; handed over to it");
                return Launch::Forwarded;
            }
            Err(e) => tracing::debug!("forwarding to the running copy: {e}"),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    tracing::error!("Storm is already running for {} but isn't responding", data_dir.display());
    Launch::Unreachable
}

#[cfg(windows)]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(windows)]
mod ipc {
    use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
    use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use futures::channel::mpsc;

    use super::OpenRequest;

    const MAGIC: &str = "storm-open/1";
    const ACK: &str = "ok";

    fn port_file(data_dir: &Path) -> PathBuf {
        data_dir.join("instance.port")
    }

    /// Listens for forwarded requests; `None` if the listener can't be set up
    /// (later launches then just fail to hand over).
    pub fn serve(data_dir: &Path) -> Option<mpsc::UnboundedReceiver<OpenRequest>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| {
                let port = l.local_addr()?.port();
                std::fs::create_dir_all(data_dir)?;
                std::fs::write(port_file(data_dir), port.to_string())?;
                Ok(l)
            })
            .map_err(|e| tracing::warn!("single-instance listener: {e}"))
            .ok()?;
        let (tx, rx) = mpsc::unbounded();
        std::thread::Builder::new()
            .name("storm-instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Err(e) = handle(stream, &tx) {
                        tracing::debug!("single-instance request: {e}");
                    }
                    if tx.is_closed() {
                        break; // The UI is gone; we're exiting.
                    }
                }
            })
            .ok()?;
        Some(rx)
    }

    fn handle(mut stream: TcpStream, tx: &mpsc::UnboundedSender<OpenRequest>) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut text = String::new();
        BufReader::new((&stream).take(256 * 1024)).read_to_string(&mut text)?;
        let Some(body) = text.strip_prefix(MAGIC).and_then(|t| t.strip_prefix('\n')) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a Storm request"));
        };
        if tx.unbounded_send(OpenRequest::decode(body)).is_ok() {
            stream.write_all(format!("{ACK}\n").as_bytes())?;
        }
        Ok(())
    }

    /// Sends `req` to the running copy; `Ok` once it has acknowledged it.
    pub fn forward(data_dir: &Path, req: &OpenRequest) -> io::Result<()> {
        let port: u16 = std::fs::read_to_string(port_file(data_dir))?
            .trim()
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad port file"))?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(format!("{MAGIC}\n{}", req.encode()).as_bytes())?;
        stream.shutdown(Shutdown::Write)?;
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply)?;
        if reply.trim() == ACK { Ok(()) } else { Err(io::Error::other("no acknowledgement")) }
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    pub struct Mutex(HANDLE);

    impl Mutex {
        /// Opens or creates the named mutex; the flag is whether it already existed.
        pub fn create(name: &str) -> Option<(Self, bool)> {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call.
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
            // Read straight after the call, before anything else can reset it.
            let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            (!handle.is_null()).then(|| (Self(handle), existed))
        }

        pub fn none() -> Self {
            Self(std::ptr::null_mut())
        }
    }

    impl Drop for Mutex {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: we own the handle and close it exactly once.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    /// Lets the running copy take the foreground (we have the right, having
    /// just been launched by the user; it, in the background, doesn't).
    pub fn allow_foreground() {
        // SAFETY: plain Win32 call without pointers.
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
}

/// Brings the main window forward and opens every request later launches forward.
pub fn serve_requests(mut rx: mpsc::UnboundedReceiver<OpenRequest>, window: AnyWindowHandle, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(req) = rx.next().await {
            let _ = window.update(cx, |_, window, cx| {
                window.activate_window();
                open(req, window, cx);
            });
        }
    })
    .detach();
}

/// Opens the Add dialog for `req`'s links and/or files.
pub fn open(req: OpenRequest, window: &mut Window, cx: &mut App) {
    if req.is_empty() {
        return;
    }
    let backend = BackendHandle::get(cx).backend;
    window
        .spawn(cx, async move |cx| {
            // Only for showing the default folder; the dialog copes without it.
            let dir = backend.settings().await.ok().map(|s| s.download_dir);
            let _ = cx.update(|window, cx| {
                if !req.links.is_empty() {
                    AddTorrentDialog::open_links(Some(req.links.join(" ")), dir.clone(), window, cx);
                }
                if !req.files.is_empty() {
                    let (files, errors) = read_torrent_files(req.files);
                    if !errors.is_empty() {
                        ui::notify_error(window, cx, format!("Couldn't read: {}", errors.join("; ")));
                    }
                    if !files.is_empty() {
                        AddTorrentDialog::open_files(files, dir, window, cx);
                    }
                }
            });
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let req = OpenRequest {
            links: vec!["magnet:?xt=urn:btih:abc&dn=a b".into(), "https://x/y.torrent".into()],
            files: vec![PathBuf::from(r"C:\t\a b.torrent")],
        };
        assert_eq!(OpenRequest::decode(&req.encode()), req);
    }

    #[test]
    fn args_keep_links_and_skip_junk() {
        let req = OpenRequest::from_args(["magnet:?xt=urn:btih:abc".into(), "".into(), "no-such-file.torrent".into()]);
        assert_eq!(req.links, vec!["magnet:?xt=urn:btih:abc".to_owned()]);
        assert!(req.files.is_empty());
    }
}
