use std::sync::Arc;
use tokio_postgres::Client;

// Spawn a lightweight keepalive task for a Postgres client connection.
// Adds jitter to avoid synchronized spikes and ignores errors (best-effort).
// Keeps behavior compatible with prior implementation while improving robustness.
pub fn spawn_keepalive(client: Arc<Client>, min_secs: u64, max_secs: u64) {
    let max_secs = max_secs.max(min_secs + 1);
    tokio::spawn(async move {
        loop {
            let jitter = rand::random_range(min_secs..=max_secs);
            tokio::time::sleep(std::time::Duration::from_secs(jitter)).await;
            // Best-effort ping with short timeout
            let c = Arc::clone(&client);
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                c.execute("SELECT 1", &[]),
            )
            .await;
            // Ignore errors; next loop will try again.
        }
    });
}
