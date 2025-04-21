use crate::user::{ask_what_to_do, WhatToDo};
use console::style;
use std::process::exit;
use std::sync::atomic::AtomicBool;
use tokio::task::JoinHandle;

static CTRLC: AtomicBool = AtomicBool::new(false);

pub(crate) async fn spawn_ctrlc_listener() -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to register CTRL-C handler");

            match CTRLC.swap(true, std::sync::atomic::Ordering::SeqCst) {
                true => {
                    // called CTRL-C two times quickly or while it was being handled, kill the program
                    exit(2)
                }
                false => {
                    // handle it in the code at the earliest convenience
                }
            }
        }
    })
}

pub(crate) async fn check_ctrlc() -> Option<WhatToDo> {
    match CTRLC.compare_exchange(
        true,
        false,
        core::sync::atomic::Ordering::SeqCst,
        core::sync::atomic::Ordering::SeqCst,
    ) {
        Ok(_) => {
            // CTRL-C was used
            Some(
                ask_what_to_do(style("".to_string()).red(), WhatToDo::all_except(WhatToDo::Retry))
                    .await
                    .unwrap(),
            )
        }
        Err(_) => {
            // nothing to do
            None
        }
    }
}
