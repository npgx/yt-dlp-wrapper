use anyhow::anyhow;
use ouroboros::self_referencing;
use std::fs::File;
use std::io::{ErrorKind, Seek, Write};
use std::path::Path;

static LOCKFILE_PATH_STR: &str = "/tmp/a81f7509-2019-4fb9-8d72-ba66c897df34.lock";

fn soft_open_rw_or_create_if_missing(path: &Path) -> Result<File, anyhow::Error> {
    let file = File::options().read(true).write(true).open(path);
    match file {
        Ok(file) => Ok(file),
        Err(err) => match err.kind() {
            ErrorKind::NotFound => {
                let path = path.to_path_buf();
                let file = File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&path)?;

                Ok(file)
            }
            _ => Err(err.into()),
        },
    }
}

pub(crate) fn get_lock() -> Result<fd_lock::RwLock<File>, anyhow::Error> {
    let lockfile_path = Path::new(LOCKFILE_PATH_STR);
    let lockfile = soft_open_rw_or_create_if_missing(lockfile_path)?;
    let lock = fd_lock::RwLock::new(lockfile);
    Ok(lock)
}

pub(crate) fn write_pid_port(guard: &mut fd_lock::RwLockWriteGuard<File>, port: u16) -> Result<(), std::io::Error> {
    let pid = std::process::id();
    guard.set_len(0)?;
    guard.rewind()?;
    let contents = format!("{pid}\n{port}");
    guard.write_all(contents.as_bytes())?;
    Ok(())
}

pub(crate) fn ensure_tty_running_and_read_port() -> Result<u16, anyhow::Error> {
    {
        let mut lock = get_lock()?;

        match lock.try_write() {
            Ok(_guard) => {
                return Err(anyhow!("TTY instance isn't running!"));
            }
            Err(err) => match err.kind() {
                ErrorKind::WouldBlock => {
                    // tty instance is running
                }
                _ => return Err(anyhow!("Invalid error kind: {}; {}", err.kind(), err)),
            },
        };
    } // drop lock, just in case

    fn get_pid_port() -> Result<(u32, u16), anyhow::Error> {
        // bypass lock
        let contents = std::fs::read_to_string(LOCKFILE_PATH_STR)?;
        let mut contents = contents.split('\n');

        let pid = match contents.next().map(|pid| pid.parse::<u32>()) {
            None => return Err(anyhow!("Invalid tty lockfile (couldn't find pid)!")),
            Some(Err(err)) => return Err(anyhow!("Invalid pid in lockfile: {}", err)),
            Some(Ok(pid)) => pid,
        };

        let port = match contents.next().map(|port| port.parse::<u16>()) {
            None => return Err(anyhow!("Invalid tty lockfile (couldn't find port)!")),
            Some(Err(err)) => return Err(anyhow!("Invalid port in lockfile: {}", err)),
            Some(Ok(pid)) => pid,
        };

        if contents.next().is_some() {
            eprintln!("WARNING: Malformed lockfile!");
        }

        Ok((pid, port))
    }

    // retry 4 times
    for _ in 0..4 {
        let (_pid, port) = get_pid_port()?;

        if port == 0 {
            // tty is initializing tcp listener, wait a bit
            std::thread::sleep(std::time::Duration::from_millis(200));
            continue;
        } else {
            return Ok(port);
        }
    }

    // tty probably crashed before initializing fully
    Err(anyhow!("lockfile PORT is set to 0, did the tty instance crash?"))
}

#[self_referencing]
pub(crate) struct InstanceLock {
    pub(super) lock: fd_lock::RwLock<File>,
    #[borrows(mut lock)]
    #[not_covariant]
    pub(super) guard: fd_lock::RwLockWriteGuard<'this, File>,
}

impl InstanceLock {
    pub(crate) fn lock_or_panic() -> Self {
        let lock = get_lock().expect("Failed to create lock for lockfile");

        InstanceLockBuilder {
            lock,
            guard_builder: |lock: &mut fd_lock::RwLock<File>| {
                lock.try_write()
                    .expect("Failed to acquire lock guard, is another daemon instance already running?")
            },
        }
        .build()
    }
}
