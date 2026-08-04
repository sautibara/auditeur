A [`Resource`] that is made up of many concurrently-run [event streams](Stream).

Use [`#[listener]`](macro@crate::listener) to create one of these resources.

# Components

[`Listener`]s are broken up into [`Stream`]s of events. Each stream is run concurrently, all 
waiting for new events at the same time(see [`Stream::wait`]). Once a stream receives an event, it 
obtains mutable access to the listener and handles the event (see [`Stream::handle`]). After 
handling it, it yields control back to the main loop so that other streams can wait for events as 
well.

For example, the [`Listen`] trait provides [`ListenStream`]: which allows listening to a
[`ListenMessage`] stream shared between all other resources (a broadcast).

However, since [`Listen`] is a broadcast, it can't form a message channel to send a response back
to the original sender. For this case, you can instead use [`Take`], which in turn provides
[`TakeStream`]. This trait allows you to send messages that only a single listener can see,
providing a mechanism for forming one-to-one conversations. For example, [`#[respond]`] uses `Take` 
to allow other resources to call methods with return values on a listener.

# Macros

To implement the [`Listener`] trait, see the [`listener`] proc-macro.

# Examples

```rust
use auditeur::prelude::*;
use std::time::Duration;
# // proc_macro_crate shenanigans
# use auditeur::*;

/// A message that tells [`ExitListener`] to exit the program
#[derive(Clone, Debug)]
#[listen_message("test:exit")]
struct Exit;

/// A [`Listener`] that exits the program once it receives an [`Exit`] message
#[create(default)]
#[resource("test:exit_listener")]
#[listener(listen(Exit))]
struct ExitListener {
    #[create(input)]
    app: App,
}

impl Listen<Exit> for ExitListener {
    async fn on(&mut self, _: Exit) -> Result<()> {
        self.app.exit().await;
        Ok(())
    }
}


# #[tokio::main]
# async fn main() -> Result<(), auditeur::Error> {
// Create a new App with only an ExitListener
let mut app = App::builder()
    .with::<Exit>(())
    .with::<ExitListener>(())
    .spawn();

// Make sure that the listener is loaded and listening before we send the message.
let load = app
    .components_mut()
    .load(ExitListener::component_id())
    .await?;

// Get a message sender from the app
let sender = app.listen_mut().shouter::<Exit>().await?;

// Send the exit message in the registry, telling the ExitListener to exit the program
sender.shout(Exit);

// Drop our dependencies, telling the app that we're no longer using them
//   (and they thus can be unloaded).
drop((sender, load));

// Make sure the app exits within an acceptable time frame
auditeur::test::finish(app, false).await
# }
```
