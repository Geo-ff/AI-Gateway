use rusqlite::OptionalExtension;

use crate::server::storage_traits::{BoxFuture, FavoriteKind, FavoritesStore};

use super::database::DatabaseLogger;

impl FavoritesStore for DatabaseLogger {
    fn set_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
        favorite: bool,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            conn.execute(
                "INSERT INTO favorites (kind, target, favorite)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(kind, target) DO UPDATE SET favorite=excluded.favorite",
                rusqlite::params![kind.as_str(), target, if favorite { 1 } else { 0 }],
            )?;
            Ok(())
        })
    }

    fn is_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let fav: Option<i64> = conn
                .query_row(
                    "SELECT favorite FROM favorites WHERE kind = ?1 AND target = ?2",
                    rusqlite::params![kind.as_str(), target],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(fav.unwrap_or(0) != 0)
        })
    }

    fn list_favorites<'a>(
        &'a self,
        kind: FavoriteKind,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT target FROM favorites WHERE kind = ?1 AND favorite = 1",
            )?;
            let rows = stmt.query_map([kind.as_str()], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }
}

