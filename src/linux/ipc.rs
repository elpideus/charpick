//! One Charpick per session. Later starts hand their command to the running one
//! over a Unix socket in `$XDG_RUNTIME_DIR` and exit.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use calloop::channel::Sender;

use super::Cmd;

pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(dir) => dir.join("charpick.sock"),
        None => {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
            std::env::temp_dir().join(format!("charpick-{uid}.sock"))
        }
    }
}

/// Send a command to the running Charpick. False when none is running.
pub fn send(cmd: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(socket_path()) else { return false };
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    s.write_all(format!("{cmd}\n").as_bytes()).is_ok()
}

/// Ask the running Charpick to quit and wait for it to go.
pub fn quit_running(timeout: Duration) {
    if !send("quit") {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < timeout && UnixStream::connect(socket_path()).is_ok() {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Claim the socket. `AddrInUse` means another Charpick is running.
pub fn bind() -> std::io::Result<UnixListener> {
    let path = socket_path();
    match UnixListener::bind(&path) {
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return Err(e);
            }
            // Left behind by a Charpick that did not exit cleanly.
            std::fs::remove_file(&path)?;
            UnixListener::bind(&path)
        }
        r => r,
    }
}

fn parse(line: &str) -> Option<Cmd> {
    Some(match line.trim() {
        "toggle" => Cmd::Toggle,
        "show" => Cmd::Show,
        "hello" => Cmd::Hello,
        "quit" => Cmd::Quit,
        "change-shortcut" => Cmd::ChangeShortcut,
        _ => return None,
    })
}

/// Pass commands from later starts to the main loop.
pub fn serve(listener: UnixListener, tx: Sender<Cmd>) {
    let spawned = std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                log!("ipc   received {line:?}");
                if let Some(cmd) = parse(&line)
                    && tx.send(cmd).is_err()
                {
                    return;
                }
            }
        }
    });
    if let Err(e) = spawned {
        log!("ipc   cannot listen: {e}");
    }
}

/// Remove the socket when quitting.
pub fn remove() {
    let _ = std::fs::remove_file(socket_path());
}
