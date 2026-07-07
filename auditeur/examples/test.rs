use auditeur::component::IdentifiedType;
use auditeur::prelude::*;
use auditeur::*;

#[resource("test:attribute_example")]
#[listener(respond)]
#[create(default)]
pub struct IdentList {
    idents: Vec<NamespacedId>,
}

#[respond("test:attribute_example", get_ref, transaction)]
impl IdentList {
    pub fn push_ident(&mut self, ident: NamespacedId) {
        self.idents.push(ident);
    }

    // These use generics, which other methods can't.
    #[respond(resource_fn)]
    pub fn push_ident_of<I: IdentifiedType>(&mut self) {
        self.push_ident(I::id().to_owned())
    }

    #[respond(connection_fn)]
    pub async fn push_ident_of<I: IdentifiedType>(&self) {
        self.push_ident(I::id().to_owned()).await
    }

    pub fn index(&self, index: usize) -> NamespacedId {
        self.idents[index].clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    let mut app = App::builder().with::<IdentList>(()).spawn();

    #[identified("test:identifiable1")]
    struct TestIdentifiable1;
    #[identified("test:identifiable2")]
    struct TestIdentifiable2;

    let conn = Get::<IdentList>::create(&mut app).await?;

    conn.push_ident(ident!("test:ident1").to_owned()).await;
    conn.push_ident_of::<TestIdentifiable1>().await;
    conn.run_transaction(async |reference: &mut IdentList| {
        reference.push_ident_of::<TestIdentifiable2>();
    })
    .await;

    assert_eq!(*ident!("test:ident1"), conn.index(0).await);
    assert_eq!(*ident!("test:identifiable1"), conn.index(1).await);
    assert_eq!(*ident!("test:identifiable2"), conn.index(2).await);
    Ok(())
}
