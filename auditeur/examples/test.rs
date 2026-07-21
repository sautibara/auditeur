use auditeur::prelude::*;
use auditeur::*;

// All that needs to be done in the `#[listener]` macro is to pass in a `respond` flag.
// The macro then pulls the necessary information from the `impl` block for which messages
// to listen to.
#[create(default)]
#[resource("test:counter")]
#[listener(respond)]
struct Counter {
    value: usize,
    #[create(input)]
    sender: Shouter<CounterChanged>,
}

// The macro takes the NamespacedId of the listener as the first argument to fill out the id
// for the `Query` message (which is automatically generated).
// The `get_ref` flag makes `Get<Counter>` return the created `CounterConnection` struct.
// The connection has methods that call the corresponding methods on the actual `Counter`.
#[respond("test:counter", get_ref)]
impl Counter {
    // These methods can have return values, but note that neither the parameters or return
    // types can hold references (since they're sent using messages). To get around this,
    // use transactions, clone items, or use `Arc`s.
    // Thankfully here we don't have to worry about that, since `usize` is copyable.
    pub fn get(&self) -> usize {
        self.value
    }

    // If methods are provided that return nothing, when they're called on the connection
    // and awaited, the future will await until the listener is finished handling the method.
    // This is used here to allow other resources to wait until the value is fully set.
    pub fn set(&mut self, value: usize) {
        self.value = value;
        self.sender.shout(CounterChanged);
    }
}

#[derive(Clone)]
#[listen_message("test:counter_changed")]
struct CounterChanged;

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    let mut app = App::builder()
        .with::<Counter>(())
        .with::<CounterChanged>(())
        .spawn();

    let listener = app.listen_mut().listener::<CounterChanged>().await?;
    let counter = app.resources_mut().get::<Counter>().await?;

    assert!(listener.is_empty());
    assert_eq!(counter.get().await, 0);

    counter.set(5).await;

    assert!(!listener.is_empty());
    assert_eq!(counter.get().await, 5);

    drop((listener, counter));
    auditeur::test::finish(app, true).await
}
