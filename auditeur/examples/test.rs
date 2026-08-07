use auditeur::app::event::ComponentLifecycle;

use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    #[resource("test:resource")]
    #[pure_resource]
    #[create(default)]
    struct Resource;

    let mut app = App::builder().spawn();

    // With an event listener, we could listen to events that happen in the app.
    let mut events = app.get_event_listener().await?;

    // For example, if we add a component to the app,
    app.components_mut()
        .add(Children::new().with::<Resource>(()))
        .await;

    // we should be able to see that in the event list.
    loop {
        if let auditeur::app::Event::Component { id, lifecycle } = events.listen().await
            && id == Resource::component_id()
            && lifecycle == ComponentLifecycle::Add
        {
            break;
        }
    }

    // Note that [`Event`]s are backed by [`listen`]. This means that event listeners can only
    // see events that happen after they're obtained.

    auditeur::test::finish(app, false).await
}
