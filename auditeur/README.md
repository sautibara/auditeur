# Auditeur

[![crates.io](https://img.shields.io/crates/v/auditeur.svg)](https://crates.io/crates/auditeur)
[![docs.rs](https://docs.rs/auditeur/badge.svg)](https://docs.rs/auditeur)
[![License](https://img.shields.io/crates/l/auditeur)](https://github.com/sautibara/auditeur)
[![brainmade.org](https://img.shields.io/badge/brainmade.org-FFFFFF?style=social&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxZW0iIGhlaWdodD0iNzkiIHZpZXdCb3g9IjAgMCA2NyA3OSIgZmlsbD0ibm9uZSI%2BPHBhdGggZmlsbD0iIzAwMCIgZD0iTTUyLjYxMiA3OC43ODJIMjMuMzNhMi41NTkgMi41NTkgMCAwIDEtMi41Ni0yLjU1OHYtNy42NzdoLTcuOTczYTIuNTYgMi41NiAwIDAgMS0yLjU2LTIuNTZWNTUuMzE1bC04LjgyLTQuMzk3YTIuNTU5IDIuNTU5IDAgMCAxLS45ODYtMy43MWw5LjgwNy0xNC43MTR2LTQuMzVDMTAuMjQgMTIuNTk5IDIyLjg0MyAwIDM4LjM4OCAwIDUzLjkzMiAwIDY2LjUzNCAxMi42IDY2LjUzOCAyOC4xNDNjLS42MzIgMjcuODI0LTEwLjc2IDIzLjUxNi0xMS4xOCAzNC4wNDVsLS4xODcgMTQuMDM1YTIuNTkgMi41OSAwIDAgMS0uNzUgMS44MSAyLjU1IDIuNTUgMCAwIDEtMS44MDkuNzVabS0yNi43MjMtNS4xMTdoMjQuMTY0bC4yODYtMTQuNTQyYy0uMjYzLTYuNjU2IDExLjcxNi04LjI0MyAxMS4wOC0zMC43MzQtLjM1OC0xMi43MTMtMTAuMzEzLTIzLjI3MS0yMy4wMzEtMjMuMjcxLTEyLjcxOCAwLTIzLjAyOSAxMC4zMDctMjMuMDMyIDIzLjAyNXY1LjExN2MwIC41MDYtLjE1IDEtLjQzIDEuNDJsLTguNjMgMTIuOTQxIDcuNjQ1IDMuODJhMi41NTkgMi41NTkgMCAwIDEgMS40MTUgMi4yOTF2OS42OTdoNy45NzRhMi41NTkgMi41NTkgMCAwIDEgMi41NiAyLjU1OXY3LjY3N1oiLz48cGF0aCBmaWxsPSIjMDAwIiBkPSJNNDAuMzcyIDU4LjIyMlYzOC45MzRjLjExOCAwIC4yMzcuMDE4LjM1NS4wMTggOS43NjktLjAxMiAxNy4wNS05LjAxMiAxNS4wMjItMTguNTY3YTIuMzY2IDIuMzY2IDAgMCAwLTEuODIxLTEuODIyYy04LjEwNi0xLjczLTE2LjEyMSAzLjI5Mi0xOC4wOTggMTEuMzQxLS4wMjQtLjAyNC0uMDQzLS4wNS0uMDY2LS4wNzNhMTUuMzIzIDE1LjMyMyAwIDAgMC0xNC4wNi00LjE3IDIuMzY1IDIuMzY1IDAgMCAwLTEuODIxIDEuODJjLTIuMDI4IDkuNTU1IDUuMjUyIDE4LjU1NCAxNS4wMiAxOC41NjguMjM2IDAgLjQ5Mi0uMDI4LjczOC0uMDR2MTIuMjEzaDQuNzMxWm0yLjgzOS0zMi4xNDNhMTAuNjQ2IDEwLjY0NiAwIDAgMSA4LjEyNC0zLjEwNmMuMzUgNi4zNC00Ljg4OCAxMS41NzctMTEuMjI4IDExLjIzYTEwLjU4IDEwLjU4IDAgMCAxIDMuMTA0LTguMTI0Wk0yNy40MDMgMzguMTkzYTEwLjYwNyAxMC42MDcgMCAwIDEtMy4xMTgtOC4xMjNjNi4zNDQtLjM1OCAxMS41ODcgNC44ODYgMTEuMjI4IDExLjIzLTMuMDIzLjE2OS01Ljk3My0uOTYxLTguMTEtMy4xMDdaIi8%2BPC9zdmc%2B)](https://brainmade.org/)

An application framework for Rust with focuses on developer experience, flexibility, and
modularity.

This framework was built with plugin support in mind, with
[`auditeur-plugin-host`](https://crates.io/crates/auditeur-plugin-host) developed in tandem.
It's not required to use the application framework, but it is meant to be an easy-to-fit
addition for applications who want to use it.

## Quick Start

### Resources

This library is primarily built around [`Resource`]s: types that provide asynchronous tasks to
be run globally, communicating to each other by sending and receiving messages. Each resource
can be added to an [`App`] exactly once, providing a single asynchronous task. `Resource`s are
a type of [`Component`], which is an abstract object that has an [identifier](ComponentId) and
can be added to the `App`.

```rust
use auditeur::prelude::*;

// All components have a unique [`NamespacedId`]: an identifier made up of a namespace and a
// path separated by a colon. Usually, the name of the namespace is the name of the project.
// Using this id, we could implement all the odd traits for [`Resource`].
#[resource("test:resource")]
// Each type of [`Resource`] has a separate macro to implement the trait.
// The watcher is the simplest type of runnable resource; it just runs a single task.
#[watcher]
// Each resource also has to define how it should be created when added to an app.
// Since all fields of this type (of which there are none) implement [`Default`], we could use:
#[create(default)]
struct Resource;

impl Watcher for Resource {
    async fn run(&mut self) -> anyhow::Result<()> {
        println!("initialized!");
        Ok(())
    }
}

// All `auditeur` projects start with an [`App`].
// It holds all the [`Component`]s we add and allows them to interact with one another.
let app = App::builder()
    // With the app, we could add the resource we created,
    .with::<Resource>(())
    // and start running the app as a whole.
    .spawn();

// Resources' tasks are automatically started when added to an app,
// so the watcher should send its message now.

// After, we wait for the app to exit. This should happen automatically, as our resource
// immediately exits its task after printing its message.
app.wait_for_exit().await;
```

### Messages and Dependencies

Resources can communicate with each other using
[`ListenMessage`](crate::component::listen::ListenMessage)s and
[`TakeMessage`](crate::component::take::TakeMessage)s. Listen messages are backed by
[`broadcast`](tokio::sync::broadcast) channels, so each message is copied and seen by every
single listener. Take messages, on the other hand, are backed by [`mpmc`](flume) channels, so
each message is seen by only a single listener.

Resources can also access the [`App`] during initialization through the [`Create`] trait. This
allows them to access and depend on messages and other resources, forming complex webs of
[dependencies](Dependency). The app automatically manages these dependencies, loading and
unloading things when necessary. When using plugins, this is leveraged to make sure that
plugins are only unloaded when they're not being used anymore.

```rust
use auditeur::prelude::*;

// Listen messages can be created through the [`#[listen_message]`] macro.
// As [`Component`]s, they also have global identifiers.
#[listen_message("test:exit")]
#[derive(Clone)]
struct Exit;

#[resource("test:resource")]
// Listeners are the most common type of resource. They listen to multiple event streams.
// Every stream that it listens to has to be included in the macro, so here we define that we
// listen to the [`Exit`] listen channel.
#[listener(listen(Exit))]
#[create(default)]
struct Resource {
    // Resources can access different parts of the app using the [`Create`] trait.
    // [`Input`]s can automatically be obtained using the `#[create(input)]` attribute.
    // And since we're using it to create the field, we could still use `#[create(default)].
    #[create(input)]
    app: App,
}

impl Listen<Exit> for Resource {
    async fn on(&mut self, _: Exit) -> anyhow::Result<()> {
        // With an [`App`] handle, we could do various things, such as unloading a resource.
        self.app.resources_mut().stop::<Self>().await;
        Ok(())
    }
}

let mut app = App::builder()
    .with::<Resource>(())
    // Now that we have another component, we also have to include it in the [`App`].
    .with::<Exit>(())
    .spawn();

// The listener should load automatically, but if it doesn't load in time, it'll miss our
// message. To prevent this, we'll load it manually.
app.components_mut().load(Resource::component_id()).await?;

// Now that it's loaded, we could send the message using a [`Shouter`] (the sending half of a
// listen channel) and be sure that it'll be received.
let shouter = app.listen_mut().shouter::<Exit>().await?;
shouter.shout(Exit);

// But since we're done with using the shouter, we should drop it to let the app know that
// we're not using it anymore. If we kept ahold of it, the app would know that the [`Exit`]
// message was still being used, and so it wouldn't allow it to unload and the app to exit.
drop(shouter);

// Now, because we sent the message, the listener should stop itself, and the app should exit
// automatically. We use [`auditeur::test::finish`] instead of [`App::wait_for_exit`],
// as it's a direct replacement built for use in tests.
auditeur::test::finish(app, false).await
```

### Respond

[`Resource`]s can also provide [handles](component::resource::get) for other resources to use,
who can be created based on data owned by the resource. A common way to create one of these
handles is through the [`#[respond]`](macro@respond) macro, which allows a component to
export methods that could be called through message passing.

```rust
use auditeur::prelude::*;

// Here's a resource that we'll use [`#[respond]`] with.
#[resource("test:resource")]
// Since [`#[respond]`] uses messages, we have to declare it in [`#[listener]`].
// We also use the `weak` attribute so that the resource automatically exits when all
// dependencies to it are dropped.
#[listener(respond, weak)]
// And because we want to customize [`Create`], we don't use `#[create(default)]`.
#[create]
struct Resource {
    value: usize,
}

// Since we didn't use `#[create(default)]`, we have to implement [`CreateDerive`].
impl CreateDerive for Resource {
    // In [`CreateDerive`], we define how to create a `Plain` struct,
    // which is our type with all the [`Input`] fields removed.
    async fn create((): &(), _: &mut App) -> Result<Self::Plain, auditeur::Error> {
        // This allows us to set a new default value of `10`.
        Ok(Self::Plain { value: 10 })
    }
}

// The [`#[respond]`] macro keeps the impl block largely unchanged, but it also defines a new
// type that has methods that call the methods in the resource using messages. Effectively,
// it allows other resources to call these methods through that new "connection" type.
#[respond("test:resource", get_ref)]
impl Resource {
    pub const fn get(&self) -> usize {
        self.value
    }

    pub const fn set(&mut self, value: usize) {
        self.value = value;
    }
}

let mut app = App::builder()
    // Note that we don't have to add the message that [`#[respond]`] uses here.
    // This is because it's automatically added as a child of the resource due to the resource
    // having a [`RespondStream`]. [`TakeStream`] also acts this way.
    .with::<Resource>(())
    .spawn();

// Since we used the `get_ref` attribute, the macro will also implement [`GetRef`] using
// the connection as the handle, allowing other resources to get the connection using [`Get`].
let resource: Get<Resource> = app.resources_mut().get::<Resource>().await?;

// Now that we have the connection, we could access any methods wrapped in [`#[respond]`].
// No matter the signature of the source method, connection methods will always be `async`.
// This is because they have to `await` the response of the query.
assert_eq!(resource.get().await, 10);

// And afterwards if we set a new value,
resource.set(5).await;

// we could see that the value was changed.
assert_eq!(resource.get().await, 5);

// Note that we didn't have to do anything to make sure that `set` completed, because
// [`#[respond]`] always ensures that methods are fully completed before returning.

drop(resource);
auditeur::test::finish(app, false).await
```

## Further Reading

For further reading, see the module documentations of [component] and [app], as well as the
macros [`#[create]`](macro@create), [`#[listener]`](macro@listener), and
[`#[respond]`](macro@respond).
