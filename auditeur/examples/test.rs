use auditeur::{
    app::map::{DependencyId, RequestSet},
    component::{
        IdentifiedComponentType, RegistryOf,
        resource::{
            GetRunnerWith, Resources,
            listener::{ListenerMethod, StopComponent, Unbiased},
        },
    },
    prelude::*,
    test::finish,
};

use auditeur::*;

#[resource("test:meow")]
#[create(default)]
struct Meow {
    #[create(input)]
    app: App,
}

impl GetRunnerWith for Meow {
    type Method = ListenerMethod;
}

impl Listener for Meow {
    type Components = (Unbiased, StopComponent);

    async fn init(&mut self) -> anyhow::Result<()> {
        println!("meow");
        self.app
            .map_mut()
            .request(RequestSet::remove_dependency(DependencyId {
                dependency: Self::component_id(),
                dependent: App::component_id(),
            }))
            .await;
        Ok(())
    }

    async fn exit(&mut self) -> anyhow::Result<()> {
        println!("exiting");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    let app = App::builder().with::<Meow>(()).spawn();

    finish(app, false).await
}
