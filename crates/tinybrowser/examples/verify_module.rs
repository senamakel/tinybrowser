//! Loads a built module through the real `TinyBus` dynamic loader.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tinybrowser::{is_compatible, names};
use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = module_argument()?;
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let module_host = ModuleHost::new(broker);
    let info = module_host.load_file(&module)?;

    if info.name != env!("CARGO_PKG_NAME") {
        return Err(io::Error::other(format!(
            "loaded module `{}` instead of `{}`",
            info.name,
            env!("CARGO_PKG_NAME")
        ))
        .into());
    }

    let client = Connection::connect(bus.connect().await?).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let claimed = client.list_names().await?;
            if claimed.iter().any(|name| name.as_str() == names::INTERFACE) {
                return tinybus::Result::Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;

    // `ContractVersion` rather than a browser operation on purpose: this
    // verifies that the artifact loads, claims its name, and answers a call.
    // Launching a browser would make the check depend on what the machine
    // running it happens to have installed, which is a different question.
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let version: (u32, u32) = proxy.call(names::methods::CONTRACT_VERSION, ()).await?;
    if !is_compatible(version) {
        return Err(io::Error::other(format!(
            "module serves contract {version:?}, which this build cannot bind to"
        ))
        .into());
    }

    println!(
        "verified {} as TinyBus module `{}`",
        module.display(),
        info.name
    );
    broker_task.abort();
    Ok(())
}

fn module_argument() -> Result<PathBuf, io::Error> {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cargo run --example verify_module -- <module-path>",
            )
        })
}
