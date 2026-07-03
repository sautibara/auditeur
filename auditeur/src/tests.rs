use std::time::Duration;

use crate::prelude::*;

#[tokio::test]
pub async fn app_exits() {
    let app = App::builder().spawn();

    // let map = app.map().get().await;
    // println!("{map:#?}");

    tokio::time::timeout(Duration::from_secs(5), app.wait_for_exit())
        .await
        .expect("app should exit");
}
