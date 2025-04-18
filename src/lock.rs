use std::error::Error;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

static LOCKFILE_PATH_STR: &str = "/tmp/a81f7509-2019-4fb9-8d72-ba66c897df34.pid";

async fn soft_open_rw_or_create_if_missing(path: &Path) -> Result<File, std::io::Error> {
    let file = File::options().read(true).write(true).open(path).await;
    match file {
        Ok(file) => Ok(file),
        Err(err) => match err.kind() {
            ErrorKind::NotFound => {
                File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)
                    .await
            }
            _ => Err(err),
        },
    }
}

pub(crate) async fn get_lock() -> Result<fd_lock::RwLock<File>, std::io::Error> {
    let lockfile_path = Path::new(LOCKFILE_PATH_STR);
    let lockfile = soft_open_rw_or_create_if_missing(lockfile_path).await?;
    let lock = fd_lock::RwLock::new(lockfile);
    Ok(lock)
}

pub(crate) async fn write_pid<'lock>(
    guard: &mut fd_lock::RwLockWriteGuard<'lock, File>,
) -> Result<(), std::io::Error> {
    let pid = std::process::id();
    guard.set_len(0).await?;
    guard.rewind().await?;
    guard.write_all(pid.to_string().as_ref()).await?;
    Ok(())
}

pub(crate) async fn write_port<'lock>(
    _guard: &mut fd_lock::RwLockWriteGuard<'lock, File>,
    port: u16,
) -> Result<(), std::io::Error> {
    let mut portfile_path = PathBuf::from(LOCKFILE_PATH_STR);
    portfile_path.set_extension("port");
    tokio::fs::write(&portfile_path, port.to_string()).await?;
    Ok(())
}

pub(crate) async fn read_port_no_lock() -> Result<u16, Box<dyn Error + Send + Sync>> {
    let mut portfile_path = PathBuf::from(LOCKFILE_PATH_STR);
    portfile_path.set_extension("port");
    let raw = match tokio::fs::read(&portfile_path).await {
        Ok(raw) => raw,
        Err(err) => return Err(Box::new(err)),
    };

    let string = match String::from_utf8(raw) {
        Ok(string) => string,
        Err(err) => return Err(Box::new(err)),
    };

    string.parse::<u16>().map_err(|err| Box::new(err) as _)
}
