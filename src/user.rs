use console::style;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub(crate) enum WhatToDo {
    Retry,
    RestartRequest,
    Continue,
    AbortRequest,
}

impl WhatToDo {
    pub(crate) const fn all() -> &'static [Self] {
        &[
            WhatToDo::Retry,
            WhatToDo::RestartRequest,
            WhatToDo::Continue,
            WhatToDo::AbortRequest,
        ]
    }

    pub(crate) const fn all_except(except: WhatToDo) -> &'static [Self] {
        match except {
            WhatToDo::Retry => &[WhatToDo::RestartRequest, WhatToDo::Continue, WhatToDo::AbortRequest],
            WhatToDo::RestartRequest => &[WhatToDo::Retry, WhatToDo::Continue, WhatToDo::AbortRequest],
            WhatToDo::Continue => &[WhatToDo::Retry, WhatToDo::RestartRequest, WhatToDo::AbortRequest],
            WhatToDo::AbortRequest => &[WhatToDo::Retry, WhatToDo::RestartRequest, WhatToDo::Continue],
        }
    }
}

impl Display for WhatToDo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WhatToDo::Retry => {
                write!(f, "Retry")
            }
            WhatToDo::RestartRequest => {
                write!(f, "Restart video request")
            }
            WhatToDo::Continue => {
                write!(f, "Continue...")
            }
            WhatToDo::AbortRequest => {
                write!(f, "Abort the video request")
            }
        }
    }
}

pub(crate) async fn ask_what_to_do(
    message: console::StyledObject<String>,
    allowed: impl AsRef<[WhatToDo]>,
) -> Result<WhatToDo, anyhow::Error> {
    let mut allowed = allowed.as_ref().to_vec();
    allowed.sort();

    if allowed.is_empty() {
        panic!("Internal Error: ask_action_on_command_error received empty 'allowed'")
    }

    let todo = tokio::task::spawn_blocking(move || {
        dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(format!("{message}\n{}", style("What would you like to do?").cyan()))
            .default(0)
            .items(&allowed)
            .interact()
            .map(|ordinal| allowed[ordinal])
    })
    .await??;

    Ok(todo)
}
