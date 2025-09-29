use std::{path::PathBuf, rc::Rc};

use anyhow::Result;

use crate::{engine::run_flutter, wayland::WaylandConnection};

mod engine;
mod wayland;

fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .try_init()?;

    let conn = wayland_client::Connection::connect_to_env()?;
    let conn = WaylandConnection::new(conn)?;

    let conn = Rc::new(conn);
    let conn2 = conn.clone();

    let run_wayland_client = async {
        conn.run().await.unwrap();
    };

    let args = std::env::args().collect::<Vec<_>>();
    let asset_path = PathBuf::from(args.get(1).expect("no asset path given"));
    let icu_data_path = PathBuf::from(args.get(2).expect("no icu data path given"));

    let local_ex = smol::LocalExecutor::new();

    smol::future::block_on(local_ex.run(async {
        futures::join!(run_wayland_client, async {
            run_flutter(conn2, &asset_path, &icu_data_path, &local_ex)
                .await
                .unwrap();
        });
    }));

    Ok(())
}
