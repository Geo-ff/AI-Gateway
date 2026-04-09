use rusqlite::Result;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn list_organizations(&self) -> Result<Vec<String>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT name FROM organizations ORDER BY CASE WHEN name = 'default' THEN 0 ELSE 1 END, name",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub async fn create_organization(&self, organization_id: &str) -> Result<()> {
        let trimmed = organization_id.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT OR IGNORE INTO organizations (name) VALUES (?1)",
            [trimmed],
        )?;
        Ok(())
    }
}
