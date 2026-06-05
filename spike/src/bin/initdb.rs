//! Initialize the spike schema.
use anyhow::Result;
use telex_spike::{connect, SCHEMA};

#[tokio::main]
async fn main() -> Result<()> {
    let client = connect().await?;
    client.batch_execute(SCHEMA).await?;
    println!("schema applied");
    Ok(())
}
