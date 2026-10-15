-- Listes (courses, tâches) et agenda de la maison.
-- Les dates sont en heure locale de la maison, au format « 2026-10-15T18:30:00 » :
-- comparer deux textes de ce format revient à comparer les dates.
CREATE TABLE todo_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    list       TEXT    NOT NULL,                       -- « todo.courses »
    summary    TEXT    NOT NULL,                       -- « Lait »
    done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    due        TEXT,                                   -- « 2026-10-20 », facultatif
    created_at INTEGER NOT NULL,                       -- microsecondes depuis 1970 (UTC)
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_todo_items_list ON todo_items (list, done, id);

CREATE TABLE calendar_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    calendar    TEXT    NOT NULL,                      -- « calendar.maison »
    summary     TEXT    NOT NULL,                      -- « Dentiste »
    description TEXT    NOT NULL DEFAULT '',
    start       TEXT    NOT NULL,                      -- heure locale
    end         TEXT    NOT NULL,                      -- heure locale, après start
    all_day     INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
    created_by  TEXT,                                  -- « david », NULL pour une automatisation
    created_at  INTEGER NOT NULL,
    CHECK (end > start)
) STRICT;

CREATE INDEX idx_calendar_events_range ON calendar_events (calendar, start, end);
