use crate::tools::chromaprint::ChromaprintFingerprint;
use crate::tools::fpcalc::FPCalcJsonOutput;
use crate::tty;
use std::path::Path;
use std::process::Stdio;

pub(crate) mod acoustid;
pub(crate) mod chromaprint;
pub(crate) mod fpcalc;

pub async fn fingerprint_file(path: &Path) -> Result<(ChromaprintFingerprint, f64), anyhow::Error> {
    let mut fpcalc_cmd = vec![String::from("fpcalc"), String::from("-json")];
    fpcalc_cmd.push(path.display().to_string());

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

    let fpcalc_output: FPCalcJsonOutput = serde_json::from_slice(&output.data.stdout)?;

    Ok((
        ChromaprintFingerprint::from_base64_urlsafe(fpcalc_output.fingerprint),
        fpcalc_output.duration,
    ))
}
