#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
//! Utilities for writing tests for [`auditeur`](crate)-based projects.

use std::time::Duration;

use futures::FutureExt;
use tokio::{pin, select};

use crate::{app::event::Event, prelude::*};

/// Waits for `app` to exit within a reasonable amount of time.
///
/// This will also print any errors that occur in the app before it exits, and panic if any were
/// encountered.
///
/// If `exit` is specified, the app will be manually exited.
///
/// # Errors
///
/// - If an event listener couldn't be obtained from the app.
///
/// # Examples
///
/// ```should_panic
/// use auditeur::prelude::*;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), auditeur::Error> {
/// let mut app = App::builder().spawn();
///
/// let handle = app.handle().await;
///
/// // This should panic, since we're still holding a handle onto the app.
/// auditeur::test::finish(app, false).await
/// # }
/// ```
pub async fn finish(mut app: App, exit: bool) -> Result<(), Error> {
    let mut errored = false;

    let mut event_listener = app.get_event_listener().await?;
    event_listener.release_dependency();

    if exit {
        app.exit().await;
    }

    drop(app);

    let timeout = tokio::time::sleep(Duration::from_secs(2)).fuse();
    pin!(timeout);

    loop {
        select! {
            event = event_listener.listen() => {
                match event {
                    Event::Error(error) => { eprintln!("[error] {error}"); errored = true; },
                    Event::Exit => break,
                    _ => (),
                }
            }
            () = &mut timeout => {
                eprintln!("[error] app didn't exit in time");
                errored = true;
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
