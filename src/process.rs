use crate::handle_ctrlc;
use crate::user::{ask_what_to_do, WhatToDo};
use console::{style, StyledObject};
use std::path::Path;

pub(crate) async fn wait_for_child(
    mut child: tokio::process::Child,
) -> Result<(std::process::ExitStatus, ()), std::io::Error> {
    child.wait().await.map(|status| (status, ()))
}

pub(crate) async fn wait_for_child_output(
    child: tokio::process::Child,
) -> Result<(std::process::ExitStatus, std::process::Output), std::io::Error> {
    child.wait_with_output().await.map(|output| (output.status, output))
}

#[derive(Debug, Clone)]
pub(crate) enum ChildCommandExecution<T> {
    Success(T),
    NonZeroExitStatus(std::process::ExitStatus, T),
    KilledBySignal(std::process::ExitStatus, T),
    Wtd(WhatToDo),
}

impl<T> ChildCommandExecution<T> {
    pub(crate) async fn into_success_or_ask_wtd<WTD>(
        self,
        make_message: impl FnOnce(std::process::ExitStatus, T) -> (StyledObject<String>, WTD),
    ) -> Result<Result<T, WhatToDo>, anyhow::Error>
    where
        WTD: AsRef<[WhatToDo]>,
    {
        match self {
            ChildCommandExecution::Success(data) => Ok(Ok(data)),
            ChildCommandExecution::NonZeroExitStatus(status, data)
            | ChildCommandExecution::KilledBySignal(status, data) => {
                let (message, allowed) = make_message(status, data);
                let what_to_do = ask_what_to_do(message, &allowed).await?;
                Ok(Err(what_to_do))
            }
            ChildCommandExecution::Wtd(what_to_do) => Ok(Err(what_to_do)),
        }
    }
}

pub(crate) async fn handle_child_command_execution<Ret, RetFut, RetFutErr>(
    full_command: &[impl AsRef<str>],
    work_dir: &Path,
    user_settings: impl FnOnce(&mut tokio::process::Command),
    before_context_return: impl FnOnce(&Ret),
    extract: impl FnOnce(tokio::process::Child) -> RetFut,
) -> Result<ChildCommandExecution<Ret>, anyhow::Error>
where
    RetFut: Future<Output = Result<(std::process::ExitStatus, Ret), RetFutErr>> + Send,
    RetFutErr: std::error::Error + Send + Sync + 'static,
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

    handle_ctrlc!(restart: { return Ok(ChildCommandExecution::Wtd(WhatToDo::RestartRequest)) }, abort: { return Ok(ChildCommandExecution::Wtd(WhatToDo::AbortRequest)) });

    println!();
    println!("{}", style(separator).cyan());
    println!("Entering command context.");
    println!("Executing: [\'{}\']", full_command.join("\', \'"));
    println!("{}", style(separator).cyan());
    println!();

    let mut command = tokio::process::Command::new(full_command[0]);
    command.args(&full_command[1..]);
    command.current_dir(work_dir);
    user_settings(&mut command);

    let child = command.spawn()?;
    let (exit_status, result) = extract(child).await?;

    before_context_return(&result);

    let return_to_tty = move |message: &StyledObject<String>| {
        println!();
        println!("{}", style(separator).yellow());
        println!("Returned to tty context.");
        println!("{}", message);
        println!("{}", style(separator).yellow());
        println!();
    };

    if exit_status.success() {
        let code = exit_status.code().unwrap();
        let message = style(format!("Command returned exit code {}.", code)).green();
        return_to_tty(&message);

        handle_ctrlc!(restart: { return Ok(ChildCommandExecution::Wtd(WhatToDo::RestartRequest)) }, abort: { return Ok(ChildCommandExecution::Wtd(WhatToDo::AbortRequest)) });

        Ok(ChildCommandExecution::Success(result))
    } else if let Some(err_code) = exit_status.code() {
        let message = style(format!("Command returned exit code {}.", err_code)).red();
        return_to_tty(&message);

        handle_ctrlc!(restart: { return Ok(ChildCommandExecution::Wtd(WhatToDo::RestartRequest)) }, abort: { return Ok(ChildCommandExecution::Wtd(WhatToDo::AbortRequest)) });

        Ok(ChildCommandExecution::NonZeroExitStatus(exit_status, result))
    } else {
        let message = style("Command was terminated by signal.".to_string()).red();
        return_to_tty(&message);

        handle_ctrlc!(restart: { return Ok(ChildCommandExecution::Wtd(WhatToDo::RestartRequest)) }, abort: { return Ok(ChildCommandExecution::Wtd(WhatToDo::AbortRequest)) });

        Ok(ChildCommandExecution::KilledBySignal(exit_status, result))
    }
}
