#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]

use std::time::Duration;

use futures::FutureExt;
use tokio::{pin, select};

use crate::{app::event::Event, prelude::*};

pub async fn finish(mut app: App, exit: bool) -> Result<(), Error> {
    let mut errored = false;

    let mut event_listener = app.get_event_listener().await?;
    event_listener.release_dependency();

    if exit {
        app.exit().await;
    }

    let exit_releaser = tokio::time::timeout(Duration::from_secs(2), app.wait_for_exit().fuse());
    pin!(exit_releaser);

    loop {
        select! {
            event = event_listener.listen() => {
                match event {
                    Event::Error(error) => { eprintln!("[error] {error}"); errored = true; },
                    Event::Exit => break,
                    _ => (),
                }
            }
            res = &mut exit_releaser => {
                if res.is_err() {
                    eprintln!("[error] app didn't exit in time");
                    errored = true;
                }
                break;
            },
        }
    }

    assert!(!errored);

    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn app_exits() -> Result<(), Error> {
    let app = App::builder().spawn();

    finish(app, false).await
}
