use crate::tools::chromaprint::ChromaprintFingerprint;
use crate::tools::fpcalc::FPCalcJsonOutput;
use crate::tty::{print_enter_command_context, print_return_daemon_context};
use std::path::Path;

pub(crate) mod acoustid;
pub(crate) mod chromaprint;
pub(crate) mod fpcalc;

pub async fn fingerprint_file(path: &Path) -> Result<(ChromaprintFingerprint, f64), anyhow::Error> {
    let mut fpcalc_cmd = vec![String::from("fpcalc"), String::from("-json")];
    fpcalc_cmd.push(path.display().to_string());

    print_enter_command_context(&fpcalc_cmd.join(" "));
    let output = tokio::process::Command::new(&fpcalc_cmd[0])
        .args(&fpcalc_cmd[1..])
        .current_dir(path.parent().unwrap())
        .output()
        .await?;
    print_return_daemon_context(output.status.code());

    let fpcalc_output: FPCalcJsonOutput = serde_json::from_slice(&output.stdout)?;

    Ok((
        ChromaprintFingerprint::from_base64_urlsafe(fpcalc_output.fingerprint),
        fpcalc_output.duration,
    ))
}
