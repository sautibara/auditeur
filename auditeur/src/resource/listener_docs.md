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

# Inputs

Each component can use their own separate [`Input`], which is the main way that resources can talk
to each other. Please refer to that trait's documentation for more information.

Much of the underlying implementation complexity of the [`Listener`] trait (which is handled
through [macros](#macros)) comes from the fact that [`Input`]s can be shared between all of the
components. Generally, user inputs (such as those specified in [`Listen::OnInput`]) are shared,
while inputs specific to the components themselves (like the message channel backing
[`ListenComponent`]) are not shared.

Since the amount of trait definitions for [`Subset`] grows at an exponential rate as the amount of
accepted inputs increases, only 8 unique [`Input`]s for a [`Listener`] are accepted at this time.
To work around this, try using a nested tuple or struct with a
[`derive(Input)`](macro@crate::Input) annotation, which will only be shared if the entire tuple or
struct is the same between different components.

# Runnable

[`Listener`] has [`Runnable`](trait@Runnable) as a supertrait, which provides callbacks for when
the listener [initializes](Runnable::init) or [exits](Runnable::exit). These methods are included
in the shared inputs like components.

Unfortunately, since Rust doesn't support associated `const` defaults, these methods cannot have
default implementations. As a workaround, use the [`Runnable`](macro@crate::Runnable) derive macro
instead.

# Macros

To implement the [`Listener`] trait, see the [`listener`] proc-macro. It pulls in the [`Input`]s for
each component, combines them, and implements the trait. Note that the implementations for traits
with inputs must also be attached with the [`has_input`] proc-macro to provide the input information
to the listener macro.

# Examples

```rust
use auditeur::prelude::*;
use std::time::Duration;
# // TODO: add to prelude
# use auditeur::resource::listener::*;
# // proc_macro_crate shenanigans
# use auditeur::*;

/// A message that tells [`ExitListener`] to exit the program
#[derive(Clone, Debug)]
#[message("test:exit")]
struct Exit;

/// A [`Listener`] that exits the program once it receives an [`Exit`] message
#[listener("test:exit_listener", listen(Exit))]
#[derive(Runnable, Default, Create)] // use default implementations for Runnable and Create
struct ExitListener;

#[has_input]
impl Listen<Exit> for ExitListener {
    // Note that this method only has one [`Input`], so the last parameter is simple.
    // If the method had none, the last parameter would have to be `(): ()` to signify no inputs.
    // If the method had more than one, the last parameter would have to be a tuple of mutable
    //   references (such as `(stopper, sender): (&mut Stopper, &mut Sender<Exit>)`).
    async fn on(&mut self, _: Exit, stopper: &mut Stopper) -> Result<()> {
        // [`Stopper`] can be used to stop individual resources or the entire program
        stopper.exit();
        Ok(())
    }
}


# #[tokio::main]
# async fn main() {
// Create a new App with only an ExitListener
let app =
    App::new().with_bundle(new_bundle!("test:exit").with_listener::<ExitListener>(()));
// Build it into a Registry so that we can get the message sender before running it
// Usually, it's simpler just to call app.run(_)
let registry = app.build(1024).await.expect("failed to build app");

// Get a message sender from the registry
let sender = registry.message_sender::<Exit>();
// Start the registry running in the background now that we got the message sender
let running_registry = tokio::task::spawn(registry.run());

// Send the exit message in the registry, telling the ExitListener to exit the program
// Usually, resources send messages to other resources, but the main function can send them too
sender.send(Exit).expect("failed to send message");

// Make sure the app exits within an acceptable time frame
tokio::time::timeout(Duration::from_secs(1), running_registry)
    .await
    .expect("ExitListener should exit immediately after the Exit message is sent")
    .expect("the app shouldn't panic");
# }
```
