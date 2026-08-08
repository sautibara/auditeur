use auditeur::app::map::ListenerId;

use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    // With the map, we could also determine what events are blocking on other events.

    #[resource("test:resource")]
    #[pure_resource]
    #[create(default)]
    struct Resource;

    // Consider a map with an added and loaded resource.
    let mut app = App::builder().with::<Resource>(()).spawn();
    let dependency = app.components_mut().load(Resource::component_id()).await?;

    let map = app.map().get().await;

    // First, let's ensure that the component is added and loaded.
    assert!(map.is_added(&Resource::component_id()));
    assert!(map.is_loaded(&Resource::component_id()));

    // Now, if we access the listener for the removal of the component,
    let listener = ListenerId::remove_component(Resource::component_id());
    let listener = &map.listeners[&listener];

    // we could see that it's being blocked on the component's unload.
    assert!(listener.blocked_on.iter().any(|id| {
        id.as_unload_component()
            .is_some_and(|id| **id == Resource::component_id())
    }));

    drop(dependency);
    auditeur::test::finish(app, false).await
}
