//! Les listes (courses, tâches) et l'agenda de la maison.
//!
//! Les textes et les dates sont vérifiés par la tâche de l'agenda (crate
//! `ha-agenda`) avant d'arriver ici ; on se contente de les ranger et de les
//! relire. Toutes les requêtes sont paramétrées (`?1`, `?2`…).

use std::time::SystemTime;

use crate::error::StorageError;
use crate::store::{Store, to_micros};

/// Un élément d'une liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub id: i64,
    pub summary: String,
    pub done: bool,
    /// « 2026-10-20 », facultatif.
    pub due: Option<String>,
}

/// Ce qu'on fait de la date d'un élément de liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueChange<'a> {
    /// On n'y touche pas.
    Keep,
    /// On l'efface.
    Clear,
    /// On la change : « 2026-10-20 ».
    Set(&'a str),
}

/// Un rendez-vous de l'agenda. Les dates sont en heure locale :
/// « 2026-10-15T18:30:00 ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: i64,
    pub summary: String,
    pub description: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub created_by: Option<String>,
}

/// Un rendez-vous à ajouter.
#[derive(Debug, Clone, Copy)]
pub struct NewCalendarEvent<'a> {
    pub summary: &'a str,
    pub description: &'a str,
    pub start: &'a str,
    pub end: &'a str,
    pub all_day: bool,
    pub created_by: Option<&'a str>,
}

type TodoRow = (i64, String, i64, Option<String>);
type EventRow = (i64, String, String, String, String, i64, Option<String>);

impl Store {
    /// Les éléments d'une liste : d'abord ceux à faire, puis ceux faits,
    /// chacun dans l'ordre d'ajout.
    pub async fn todo_items(&self, list: &str) -> Result<Vec<TodoItem>, StorageError> {
        let rows: Vec<TodoRow> = sqlx::query_as(
            "SELECT id, summary, done, due FROM todo_items
             WHERE list = ?1 ORDER BY done, id",
        )
        .bind(list)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, summary, done, due)| TodoItem {
                id,
                summary,
                done: done != 0,
                due,
            })
            .collect())
    }

    /// Ajoute un élément (pas encore fait). Renvoie son numéro.
    pub async fn add_todo_item(
        &self,
        list: &str,
        summary: &str,
        due: Option<&str>,
    ) -> Result<i64, StorageError> {
        let now = to_micros(SystemTime::now())?;
        let id = sqlx::query(
            "INSERT INTO todo_items (list, summary, done, due, created_at, updated_at)
             VALUES (?1, ?2, 0, ?3, ?4, ?4)",
        )
        .bind(list)
        .bind(summary)
        .bind(due)
        .bind(now)
        .execute(self.pool())
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Modifie un élément : son texte, son état (fait ou non), sa date.
    /// Renvoie `false` s'il n'existe pas.
    pub async fn update_todo_item(
        &self,
        list: &str,
        id: i64,
        summary: Option<&str>,
        done: Option<bool>,
        due: DueChange<'_>,
    ) -> Result<bool, StorageError> {
        let changed = sqlx::query(
            "UPDATE todo_items SET
                summary = COALESCE(?3, summary),
                done = COALESCE(?4, done),
                due = CASE WHEN ?5 THEN ?6 ELSE due END,
                updated_at = ?7
             WHERE list = ?1 AND id = ?2",
        )
        .bind(list)
        .bind(id)
        .bind(summary)
        .bind(done.map(i64::from))
        .bind(due != DueChange::Keep)
        .bind(match due {
            DueChange::Set(date) => Some(date),
            DueChange::Keep | DueChange::Clear => None,
        })
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(changed > 0)
    }

    /// Retire un élément. Renvoie `false` s'il n'existait pas.
    pub async fn remove_todo_item(&self, list: &str, id: i64) -> Result<bool, StorageError> {
        let removed = sqlx::query("DELETE FROM todo_items WHERE list = ?1 AND id = ?2")
            .bind(list)
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(removed > 0)
    }

    /// Retire tous les éléments faits d'une liste. Renvoie leur nombre.
    pub async fn remove_done_todo_items(&self, list: &str) -> Result<u64, StorageError> {
        Ok(
            sqlx::query("DELETE FROM todo_items WHERE list = ?1 AND done = 1")
                .bind(list)
                .execute(self.pool())
                .await?
                .rows_affected(),
        )
    }

    /// Nombre d'éléments d'une liste (pour limiter sa taille).
    pub async fn count_todo_items(&self, list: &str) -> Result<i64, StorageError> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM todo_items WHERE list = ?1")
            .bind(list)
            .fetch_one(self.pool())
            .await?;
        Ok(count)
    }

    /// Les rendez-vous qui touchent la période `[from, until[` (heure
    /// locale), du plus tôt au plus tard, `limit` au plus.
    pub async fn calendar_events(
        &self,
        calendar: &str,
        from: &str,
        until: &str,
        limit: u32,
    ) -> Result<Vec<CalendarEvent>, StorageError> {
        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT id, summary, description, start, end, all_day, created_by
             FROM calendar_events
             WHERE calendar = ?1 AND end > ?2 AND start < ?3
             ORDER BY start, id LIMIT ?4",
        )
        .bind(calendar)
        .bind(from)
        .bind(until)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, summary, description, start, end, all_day, created_by)| CalendarEvent {
                    id,
                    summary,
                    description,
                    start,
                    end,
                    all_day: all_day != 0,
                    created_by,
                },
            )
            .collect())
    }

    /// Ajoute un rendez-vous. Renvoie son numéro.
    pub async fn add_calendar_event(
        &self,
        calendar: &str,
        event: NewCalendarEvent<'_>,
    ) -> Result<i64, StorageError> {
        let id = sqlx::query(
            "INSERT INTO calendar_events
                (calendar, summary, description, start, end, all_day, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(calendar)
        .bind(event.summary)
        .bind(event.description)
        .bind(event.start)
        .bind(event.end)
        .bind(i64::from(event.all_day))
        .bind(event.created_by)
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Supprime un rendez-vous. Renvoie `false` s'il n'existait pas.
    pub async fn delete_calendar_event(
        &self,
        calendar: &str,
        id: i64,
    ) -> Result<bool, StorageError> {
        let removed = sqlx::query("DELETE FROM calendar_events WHERE calendar = ?1 AND id = ?2")
            .bind(calendar)
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(removed > 0)
    }

    /// Nombre de rendez-vous à venir (pour limiter la taille de l'agenda).
    pub async fn count_future_calendar_events(
        &self,
        calendar: &str,
        now: &str,
    ) -> Result<i64, StorageError> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM calendar_events WHERE calendar = ?1 AND end > ?2")
                .bind(calendar)
                .bind(now)
                .fetch_one(self.pool())
                .await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn todo_items_round_trip() {
        let store = store().await;
        let milk = store
            .add_todo_item("todo.courses", "Lait", None)
            .await
            .unwrap();
        let bread = store
            .add_todo_item("todo.courses", "Pain", Some("2026-10-20"))
            .await
            .unwrap();
        store
            .add_todo_item("todo.taches", "Tondre", None)
            .await
            .unwrap();

        assert!(
            store
                .update_todo_item("todo.courses", milk, None, Some(true), DueChange::Keep)
                .await
                .unwrap()
        );
        // Un numéro d'une autre liste ne change rien.
        assert!(
            !store
                .update_todo_item("todo.taches", milk, Some("Piège"), None, DueChange::Keep)
                .await
                .unwrap()
        );
        let items = store.todo_items("todo.courses").await.unwrap();
        let names: Vec<(&str, bool)> = items.iter().map(|i| (i.summary.as_str(), i.done)).collect();
        // Ce qui reste à faire d'abord.
        assert_eq!(names, [("Pain", false), ("Lait", true)]);
        assert_eq!(items[0].due.as_deref(), Some("2026-10-20"));

        // Effacer la date.
        store
            .update_todo_item("todo.courses", bread, None, None, DueChange::Clear)
            .await
            .unwrap();
        assert_eq!(store.todo_items("todo.courses").await.unwrap()[0].due, None);

        assert_eq!(
            store.remove_done_todo_items("todo.courses").await.unwrap(),
            1
        );
        assert!(store.remove_todo_item("todo.courses", bread).await.unwrap());
        assert!(!store.remove_todo_item("todo.courses", bread).await.unwrap());
        assert_eq!(store.count_todo_items("todo.courses").await.unwrap(), 0);
        assert_eq!(store.count_todo_items("todo.taches").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn calendar_events_by_period() {
        let store = store().await;
        let event = |summary, start, end| NewCalendarEvent {
            summary,
            description: "",
            start,
            end,
            all_day: false,
            created_by: Some("david"),
        };
        store
            .add_calendar_event(
                "calendar.maison",
                event("Dentiste", "2026-10-15T18:30:00", "2026-10-15T19:00:00"),
            )
            .await
            .unwrap();
        let id = store
            .add_calendar_event(
                "calendar.maison",
                event("Vacances", "2026-10-20T00:00:00", "2026-10-27T00:00:00"),
            )
            .await
            .unwrap();
        let in_period = store
            .calendar_events(
                "calendar.maison",
                "2026-10-22T00:00:00",
                "2026-10-23T00:00:00",
                10,
            )
            .await
            .unwrap();
        // Les vacances ont commencé avant, mais touchent la période.
        assert_eq!(in_period.len(), 1);
        assert_eq!(in_period[0].summary, "Vacances");
        assert_eq!(
            store
                .count_future_calendar_events("calendar.maison", "2026-10-16T00:00:00")
                .await
                .unwrap(),
            1
        );
        assert!(
            !store
                .delete_calendar_event("calendar.autre", id)
                .await
                .unwrap()
        );
        assert!(
            store
                .delete_calendar_event("calendar.maison", id)
                .await
                .unwrap()
        );
        // La base refuse une fin avant le début.
        assert!(
            store
                .add_calendar_event(
                    "calendar.maison",
                    event("Absurde", "2026-10-15T19:00:00", "2026-10-15T18:00:00"),
                )
                .await
                .is_err()
        );
    }
}
