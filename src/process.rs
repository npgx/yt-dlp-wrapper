use crate::fingerprinting::ExitStatusExt;
use console::style;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub(crate) struct WithExitStatus<T> {
    pub(crate) exit_status: std::process::ExitStatus,
    pub(crate) data: T,
}

pub(crate) async fn wait_for_cmd(mut child: tokio::process::Child) -> Result<WithExitStatus<()>, std::io::Error> {
    child.wait().await.map(|status| status.with_unit())
}

pub(crate) async fn wait_for_cmd_output(
    child: tokio::process::Child,
) -> Result<WithExitStatus<std::process::Output>, std::io::Error> {
    child.wait_with_output().await.map(|output| output.status.with(output))
}

pub(crate) async fn wrap_command_print_context<T, Ex, FT, FTErr>(
    full_command: &[impl AsRef<str>],
    work_dir: &Path,
    user_settings: impl FnOnce(tokio::process::Command) -> tokio::process::Command,
    extract: Ex,
) -> Result<WithExitStatus<T>, anyhow::Error>
where
    Ex: FnOnce(tokio::process::Child) -> FT,
    FT: Future<Output = Result<WithExitStatus<T>, FTErr>>,
    FTErr: std::error::Error + Send + Sync + 'static,
{
    let full_command = full_command.iter().map(AsRef::as_ref).collect::<Vec<_>>();

    static SEPARATOR: once_cell::sync::OnceCell<String> = once_cell::sync::OnceCell::new();

    let separator: &str = SEPARATOR.get_or_init(|| {
        let width = console::Term::stdout().size().1 as usize;
        let mut sep = String::with_capacity(width);
        for _ in 0..width {
            sep.push('=');
        }
        sep
    });

    println!();
    println!("{}", style(separator).cyan());
    println!("Entering command context.");
    println!("Executing: {}", full_command.join(" "));
    println!("{}", style(separator).cyan());
    println!();

    let mut command = tokio::process::Command::new(full_command[0]);
    command.args(&full_command[1..]);
    command.current_dir(work_dir);
    let mut command = user_settings(command);
    let child = command.spawn()?;
    let result = extract(child).await?;

    println!();
    println!("{}", style(separator).yellow());
    println!("Returned to daemon context.");
    if result.exit_status.success() {
        println!(
            "{}",
            style(format!(
                "Command returned exit code {}.",
                &result.exit_status.code().unwrap()
            ))
            .green()
        );
    } else if let Some(err_code) = result.exit_status.code() {
        println!("{}", style(format!("Command returned exit code {}.", err_code)).red());
    } else {
        println!("{}", style("Command was terminated by signal.").red());
    }
    println!("{}", style(separator).yellow());
    println!();

    Ok(result)
}
