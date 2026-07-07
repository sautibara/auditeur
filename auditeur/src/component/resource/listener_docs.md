A [`Resource`] that is made up of many concurrently-run [event streams](Component).

# Components

[`Listener`]s are broken up into [`Component`]s, which are each expected to handle a stream of
events. Each component is run concurrently, all waiting for new events (see [`Component::wait`]).
Once a component receives an event, it obtains mutable access to the listener and its respective
inputs and handles the event (see [`Component::handle`]). After handling the event, it yields
control back to the main loop so that other components can wait for events as well.

For example, the [`Listen`] trait provides [`ListenComponent`]: a component that handles listening
to a message stream shared between all other resources (a broadcast). In effect, this allows
[`Listener`]s to listen to many different message streams in the app.

However, since [`Listen`] is a broadcast, it can't form a message channel to send a response back
to the original sender. For this case, you can instead use [`Take`], which in turn provides
[`TakeComponent`]. This trait allows you to send messages that only a single listener can see,
providing a mechanism for forming one-to-one conversations.

For example, [`#[respond]`] uses `Take` to allow other resources to call methods with return values
on a listener.

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

// Get a message sender from the app
let sender = app.listen_mut().shouter::<Exit>().await?;

// Send the exit message in the registry, telling the ExitListener to exit the program
// Usually, resources send messages to other resources, but the main function can send them too
sender.shout(Exit);
// Drop the sender, telling the app that we're no longer using it (and it thus can be cleaned up).
drop(sender);

// Make sure the app exits within an acceptable time frame
auditeur::test::finish(app, false).await
# }
```
