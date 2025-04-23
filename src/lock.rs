use anyhow::anyhow;
use ouroboros::self_referencing;
use std::fs::File;
use std::io::{ErrorKind, Seek, Write};
use std::path::{Path, PathBuf};

static LOCKFILE_PATH_STR: &str = "/tmp/a81f7509-2019-4fb9-8d72-ba66c897df34.pid";

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

pub(crate) fn write_pid(guard: &mut fd_lock::RwLockWriteGuard<File>) -> Result<(), std::io::Error> {
    let pid = std::process::id();
    guard.set_len(0)?;
    guard.rewind()?;
    guard.write_all(pid.to_string().as_ref())?;
    Ok(())
}

pub(crate) fn write_port(_guard: &mut fd_lock::RwLockWriteGuard<File>, port: u16) -> Result<(), std::io::Error> {
    let mut portfile_path = PathBuf::from(LOCKFILE_PATH_STR);
    portfile_path.set_extension("port");
    std::fs::write(&portfile_path, port.to_string())?;
    Ok(())
}

pub(crate) fn ensure_tty_running_and_read_port() -> Result<u16, anyhow::Error> {
    {
        let mut lock = get_lock()?;

        match lock.try_write() {
            Ok(_guard) => {
                panic!("TTY instance isn't running!")
            }
            Err(err) => match err.kind() {
                ErrorKind::ResourceBusy | ErrorKind::WouldBlock => {
                    // tty instance is running
                }
                _ => return Err(anyhow!("Invalid error kind: {}; {}", err.kind(), err)),
            },
        }
    }

    let mut portfile_path = PathBuf::from(LOCKFILE_PATH_STR);
    portfile_path.set_extension("port");

    let string = std::fs::read_to_string(&portfile_path)?;

    Ok(string.parse::<u16>()?)
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
