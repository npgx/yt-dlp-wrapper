use crate::tools::fpcalc::FPCalcJsonOutput;
use crate::tty;
use console::style;
use std::path::Path;
use std::process::Stdio;

pub(crate) mod acoustid;
pub(crate) mod fpcalc;

pub(crate) type ChromaprintFingerprint = String;

pub async fn fingerprint_file(
    path: &Path,
) -> Result<Result<FPCalcJsonOutput, tty::handle_requests::WhatToDo>, anyhow::Error> {
    let mut fpcalc_cmd = vec![String::from("fpcalc"), String::from("-json")];
    fpcalc_cmd.push(path.display().to_string());

    let output = 'last_command: loop {
        use tty::handle_requests::WhatToDo;

        let output = tty::wrap_command_print_context(
            &fpcalc_cmd,
            path.parent().unwrap(),
            |mut cmd| {
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());
                cmd
            },
            tty::wait_for_cmd_output,
        )
        .await?;

        if !output.exit_status.success() {
            match tty::handle_requests::ask_action_on_command_error(
                style(format!("fpcalc returned a non-zero exit code: {}", output.exit_status)).red(),
                WhatToDo::all_except(WhatToDo::Continue),
            )
            .await?
            {
                WhatToDo::Retry => continue 'last_command,
                WhatToDo::Continue => panic!(),
                WhatToDo::RestartRequest => return Ok(Err(WhatToDo::RestartRequest)),
                WhatToDo::AbortRequest => return Ok(Err(WhatToDo::AbortRequest)),
            }
        }

        break 'last_command output.data;
    };

    let fpcalc_output: FPCalcJsonOutput = serde_json::from_slice(&output.stdout)?;

    Ok(Ok(fpcalc_output))
}
