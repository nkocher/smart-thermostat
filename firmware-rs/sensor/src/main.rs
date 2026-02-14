#[cfg(feature = "esp32")]
mod esp;
#[cfg(not(feature = "esp32"))]
mod host;

#[cfg(not(feature = "esp32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    host::run().await
}

#[cfg(feature = "esp32")]
fn main() -> anyhow::Result<()> {
    esp::run()
}
