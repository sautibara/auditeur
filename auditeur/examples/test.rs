use auditeur::app::event::ComponentLifecycle;
use auditeur::component::bundle::Bundles;

use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
#[allow(clippy::items_after_statements)]
async fn main() -> Result<(), auditeur::Error> {
    // Since the app is communicated to with messages, we have to be weary of race conditions.

    // Let's imagine we want to listen to when bundles are added to the app.

    let mut app = App::builder()
        // Here's a bundle added before we start listening.
        .with::<Bundle>(new_bundle!("test:bundle_1"))
        .spawn();

    // Now, to start listening, we get a map and event listener from the app.
    // For this, we use [`AppFn`], which mirrors [`App`]'s api without accessing a handle.
    let (map, event_listener) = (AppFn.map().get().using_mut())
        .then(AppFn.get_event_listener())
        // Only once we finish building up our request do we have to use our app handle,
        .call(&mut app)
        // allowing us to complete the request all at once.
        .await;
    let mut event_listener = event_listener?;

    // Through the map, we could find the bundle added before we started listening.
    assert!(map.is_added(&ComponentId::inn::<Bundles>(ident!("test:bundle_1"))));

    // And if we add a bundle after we start listening,
    app.components_mut()
        .add(Children::new().with::<Bundle>(new_bundle!("test:bundle_2")))
        .await;

    // we can find it using the event listener.
    loop {
        if let auditeur::app::Event::Component { id, lifecycle } = event_listener.listen().await
            && lifecycle == ComponentLifecycle::Add
            && id == ComponentId::inn::<Bundles>(ident!("test:bundle_2"))
        {
            break;
        }
    }

    // If we didn't use the [`AppFn`] api imagine if we (or another thread):
    // - gets a map,
    // - adds a bundle now, and
    // - gets an event listener after.
    //
    // Since the bundle was added after the map was obtained, there's no way that it could've been
    // included in the map. Similarly, since event listeners only return events from after they're
    // obtained, there's no way it would show the event. Because of these reasons, the bundle added
    // would've been missed completely.
    //
    // Instead, since we got the map and event listener in the same request, there's no way that
    // a bundle could've been added in between the two actions, and there was no risk of a race
    // condition.

    drop(event_listener);
    auditeur::test::finish(app, false).await
}
