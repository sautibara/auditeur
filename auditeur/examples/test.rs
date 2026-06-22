// necessary for macros to work
use auditeur::*;

use auditeur::{
    input::get::{GetClone, GetRef, GetRefWith},
    prelude::*,
};

#[derive(Clone, Any)]
pub struct ThingA;

impl GetRefWith for ThingA {
    type Method = GetClone;
}

pub struct ThingB;

impl GetRef for ThingB {
    type Loader = ();
    type Ref = ();

    async fn create_loader(
        resource: &Self,
        app: &mut App,
    ) -> Result<Self::Loader, auditeur::Error> {
        Ok(())
    }

    async fn load_ref(loader: &Self::Loader, app: &mut App) -> Result<Self::Ref, auditeur::Error> {
        Ok(())
    }
}

fn main() {}
