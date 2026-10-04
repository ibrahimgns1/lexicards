use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{Duration, Local, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::content;
use crate::model::{
    ActivityDay, Backup, PhraseEntry, Preferences, ReviewRating, ReviewRecord, Stats, WordCard,
};

pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }
        let connection =
            Connection::open(path).with_context(|| format!("Could not open {}", path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        let existing: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'cards')",
            [],
            |row| row.get(0),
        )?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if existing && version < 3 {
            // SQLite creates a consistent snapshot, including committed WAL
            // content. This happens before any schema changes to an old library.
            let directory = path.parent().unwrap_or(Path::new(".")).join("backups");
            std::fs::create_dir_all(&directory)?;
            let snapshot = directory.join(format!(
                "Before-v0.3-{}.sqlite",
                Utc::now().format("%Y%m%d-%H%M%S-%f")
            ));
            connection.execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])?;
        }
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS cards (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                word TEXT NOT NULL COLLATE NOCASE UNIQUE,
                phonetic TEXT,
                translation_tr TEXT NOT NULL,
                definitions_json TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                synonyms_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_reviewed_at TEXT,
                review_count INTEGER NOT NULL DEFAULT 0,
                correct_count INTEGER NOT NULL DEFAULT 0,
                mastery INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS review_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                card_id INTEGER NOT NULL,
                rating INTEGER NOT NULL,
                reviewed_at TEXT NOT NULL,
                FOREIGN KEY(card_id) REFERENCES cards(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS phrases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                phrase TEXT NOT NULL COLLATE NOCASE UNIQUE,
                category TEXT NOT NULL,
                translation_tr TEXT NOT NULL,
                definitions_json TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS review_date_index ON review_log(reviewed_at);",
        )?;
        let database = Self {
            connection,
            path: path.to_owned(),
        };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&self) -> Result<()> {
        let version: u32 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 2 {
            let transaction = self.connection.unchecked_transaction()?;
            for (table, name, declaration) in [
                ("cards", "due_at", "TEXT"),
                ("cards", "interval_days", "INTEGER NOT NULL DEFAULT 0"),
                ("cards", "favorite", "INTEGER NOT NULL DEFAULT 0"),
                ("cards", "notes", "TEXT NOT NULL DEFAULT ''"),
                ("cards", "collocations_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("cards", "archived", "INTEGER NOT NULL DEFAULT 0"),
                ("phrases", "usage_note", "TEXT NOT NULL DEFAULT ''"),
                ("phrases", "register", "TEXT NOT NULL DEFAULT ''"),
                ("phrases", "topic", "TEXT NOT NULL DEFAULT ''"),
                ("phrases", "personal_example", "TEXT NOT NULL DEFAULT ''"),
                ("phrases", "favorite", "INTEGER NOT NULL DEFAULT 0"),
                ("phrases", "known", "INTEGER NOT NULL DEFAULT 0"),
                ("phrases", "archived", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                let mut statement = self
                    .connection
                    .prepare(&format!("PRAGMA table_info({table})"))?;
                let columns = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if !columns.iter().any(|column| column == name) {
                    self.connection.execute_batch(&format!(
                        "ALTER TABLE {table} ADD COLUMN {name} {declaration};"
                    ))?;
                }
            }
            self.connection.execute_batch(
                "UPDATE phrases SET category = 'Reusable patterns'
                    WHERE category IN ('Prepositional patterns', 'Collocations & chunks', 'Everyday expressions');
                 UPDATE phrases SET category = 'Idioms' WHERE category = 'Idioms & sayings';
                 UPDATE phrases SET category = 'Reusable patterns' WHERE phrase = 'give rise to';"
            )?;
            self.connection.execute_batch(
                "UPDATE phrases SET usage_note = 'Pattern: give rise to + noun. A fixed verb phrase meaning cause; it is not a separable particle verb.',
                 register = 'Formal', topic = 'Opinions & evidence' WHERE phrase = 'give rise to' AND usage_note = '';
                 UPDATE phrases SET translation_tr = 'artı ve eksilerini değerlendirmek'
                 WHERE phrase = 'weigh up' AND translation_tr = 'artıp değerlendirmek';"
            )?;
            for phrase in content::starter_phrases() {
                self.connection.execute(
                    "INSERT OR IGNORE INTO phrases (phrase, category, translation_tr, definitions_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![phrase.phrase, phrase.category, phrase.translation("tr"), serde_json::to_string(&phrase.definitions)?, phrase.created_at],
                )?;
                self.connection.execute(
                    "UPDATE phrases SET usage_note = CASE WHEN usage_note = '' THEN ?1 ELSE usage_note END,
                     register = CASE WHEN register = '' THEN ?2 ELSE register END,
                     topic = CASE WHEN topic = '' THEN ?3 ELSE topic END WHERE phrase = ?4 COLLATE NOCASE",
                    params![phrase.usage_note, phrase.register, phrase.topic, phrase.phrase],
                )?;
            }
            self.connection.execute_batch("PRAGMA user_version = 2;")?;
            transaction.commit()?;
        }
        if version < 3 {
            let transaction = self.connection.unchecked_transaction()?;
            self.connection.execute_batch(
                "ALTER TABLE cards RENAME COLUMN translation_tr TO translations_json;
                 ALTER TABLE phrases RENAME COLUMN translation_tr TO translations_json;
                 UPDATE cards SET translations_json = CASE WHEN translations_json = '' THEN '{}'
                    ELSE json_object('tr', translations_json) END;
                 UPDATE phrases SET translations_json = CASE WHEN translations_json = '' THEN '{}'
                    ELSE json_object('tr', translations_json) END;
                 PRAGMA user_version = 3;",
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn preferences(&self) -> Result<Preferences> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'preferences'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut preferences: Preferences = value
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        preferences.daily_goal = preferences.daily_goal.clamp(1, 200);
        if preferences
            .translation_language
            .as_deref()
            .is_some_and(|language| !crate::language::is_supported(language))
        {
            preferences.translation_language = None;
        }
        Ok(preferences)
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES ('preferences', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(preferences)?],
        )?;
        Ok(())
    }

    pub fn upsert_card(&self, card: &WordCard) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO cards (word, phonetic, translations_json, definitions_json, parts_json, synonyms_json,
             created_at, notes, collocations_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(word) DO UPDATE SET
                phonetic = COALESCE(excluded.phonetic, cards.phonetic),
                translations_json = json_patch(excluded.translations_json, cards.translations_json),
                definitions_json = excluded.definitions_json, parts_json = excluded.parts_json, synonyms_json = excluded.synonyms_json,
                notes = CASE WHEN cards.notes = '' THEN excluded.notes ELSE cards.notes END,
                collocations_json = CASE WHEN excluded.collocations_json = '[]' THEN cards.collocations_json ELSE excluded.collocations_json END,
                archived = 0",
            params![card.word, card.phonetic, serde_json::to_string(&card.translations)?, serde_json::to_string(&card.definitions)?,
                serde_json::to_string(&card.parts_of_speech)?, serde_json::to_string(&card.synonyms)?,
                card.created_at, card.notes, serde_json::to_string(&card.collocations)?],
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM cards WHERE word = ?1 COLLATE NOCASE",
            [&card.word],
            |row| row.get(0),
        )?)
    }

    fn all_cards(&self) -> Result<Vec<WordCard>> {
        let mut statement = self.connection.prepare(
            "SELECT id, word, phonetic, translations_json, definitions_json, parts_json, synonyms_json, created_at,
             last_reviewed_at, review_count, correct_count, mastery, due_at, interval_days, favorite,
             notes, collocations_json, archived FROM cards ORDER BY created_at DESC, id DESC"
        )?;
        Ok(statement
            .query_map([], |row| {
                let definitions: String = row.get(4)?;
                let parts: String = row.get(5)?;
                let synonyms: String = row.get(6)?;
                let collocations: String = row.get(16)?;
                let translations: String = row.get(3)?;
                Ok(WordCard {
                    id: row.get(0)?,
                    word: row.get(1)?,
                    phonetic: row.get(2)?,
                    translations: serde_json::from_str(&translations).unwrap_or_default(),
                    definitions: serde_json::from_str(&definitions).unwrap_or_default(),
                    parts_of_speech: serde_json::from_str(&parts).unwrap_or_default(),
                    synonyms: serde_json::from_str(&synonyms).unwrap_or_default(),
                    created_at: row.get(7)?,
                    last_reviewed_at: row.get(8)?,
                    review_count: row.get(9)?,
                    correct_count: row.get(10)?,
                    mastery: row.get(11)?,
                    due_at: row.get(12)?,
                    interval_days: row.get(13)?,
                    favorite: row.get(14)?,
                    notes: row.get(15)?,
                    collocations: serde_json::from_str(&collocations).unwrap_or_default(),
                    archived: row.get(17)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn load_cards(&self) -> Result<Vec<WordCard>> {
        Ok(self
            .all_cards()?
            .into_iter()
            .filter(|card| !card.archived)
            .collect())
    }

    pub fn update_word_notes(
        &self,
        id: i64,
        language: &str,
        translation: &str,
        notes: &str,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        self.save_translation(false, id, language, translation)?;
        self.connection.execute(
            "UPDATE cards SET notes = ?1 WHERE id = ?2",
            params![notes, id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_translation(
        &self,
        is_phrase: bool,
        id: i64,
        language: &str,
        translation: &str,
    ) -> Result<()> {
        if !crate::language::is_supported(language) {
            bail!("Unsupported translation language.");
        }
        let table = if is_phrase { "phrases" } else { "cards" };
        let patch = serde_json::to_string(&std::collections::BTreeMap::from([(
            language,
            translation.trim(),
        )]))?;
        self.connection.execute(
            &format!("UPDATE {table} SET translations_json = json_patch(translations_json, ?1) WHERE id = ?2"),
            params![patch, id],
        )?;
        Ok(())
    }

    pub fn upsert_phrase(&self, phrase: &PhraseEntry) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO phrases (phrase, category, translations_json, definitions_json, created_at, usage_note, register, topic,
             personal_example) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(phrase) DO UPDATE SET category = excluded.category,
                translations_json = json_patch(excluded.translations_json, phrases.translations_json),
                definitions_json = excluded.definitions_json,
                usage_note = CASE WHEN phrases.usage_note = '' THEN excluded.usage_note ELSE phrases.usage_note END,
                register = CASE WHEN phrases.register = '' THEN excluded.register ELSE phrases.register END,
                topic = CASE WHEN phrases.topic = '' THEN excluded.topic ELSE phrases.topic END, archived = 0",
            params![phrase.phrase, phrase.category, serde_json::to_string(&phrase.translations)?, serde_json::to_string(&phrase.definitions)?,
                phrase.created_at, phrase.usage_note, phrase.register, phrase.topic, phrase.personal_example],
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM phrases WHERE phrase = ?1 COLLATE NOCASE",
            [&phrase.phrase],
            |row| row.get(0),
        )?)
    }

    fn all_phrases(&self) -> Result<Vec<PhraseEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, phrase, category, translations_json, definitions_json, created_at,
             usage_note, register, topic, personal_example, favorite, known, archived FROM phrases ORDER BY category, phrase"
        )?;
        Ok(statement
            .query_map([], |row| {
                let definitions: String = row.get(4)?;
                let translations: String = row.get(3)?;
                Ok(PhraseEntry {
                    id: row.get(0)?,
                    phrase: row.get(1)?,
                    category: row.get(2)?,
                    translations: serde_json::from_str(&translations).unwrap_or_default(),
                    definitions: serde_json::from_str(&definitions).unwrap_or_default(),
                    created_at: row.get(5)?,
                    usage_note: row.get(6)?,
                    register: row.get(7)?,
                    topic: row.get(8)?,
                    personal_example: row.get(9)?,
                    favorite: row.get(10)?,
                    known: row.get(11)?,
                    archived: row.get(12)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn load_phrases(&self) -> Result<Vec<PhraseEntry>> {
        Ok(self
            .all_phrases()?
            .into_iter()
            .filter(|phrase| !phrase.archived)
            .collect())
    }

    pub fn update_phrase(&self, phrase: &PhraseEntry) -> Result<()> {
        self.connection.execute(
            "UPDATE phrases SET category = ?1, translations_json = json_patch(translations_json, ?2), definitions_json = ?3, usage_note = ?4,
             register = ?5, topic = ?6, personal_example = ?7, phrase = ?9 WHERE id = ?8",
            params![phrase.category, serde_json::to_string(&phrase.translations)?, serde_json::to_string(&phrase.definitions)?, phrase.usage_note,
                phrase.register, phrase.topic, phrase.personal_example, phrase.id, phrase.phrase],
        )?;
        Ok(())
    }

    pub fn set_favorite(&self, is_phrase: bool, id: i64, value: bool) -> Result<()> {
        let table = if is_phrase { "phrases" } else { "cards" };
        self.connection.execute(
            &format!("UPDATE {table} SET favorite = ?1 WHERE id = ?2"),
            params![value, id],
        )?;
        Ok(())
    }

    pub fn set_known(&self, id: i64, known: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE phrases SET known = ?1 WHERE id = ?2",
            params![known, id],
        )?;
        Ok(())
    }

    pub fn set_archived(&self, is_phrase: bool, id: i64, value: bool) -> Result<()> {
        let table = if is_phrase { "phrases" } else { "cards" };
        self.connection.execute(
            &format!("UPDATE {table} SET archived = ?1 WHERE id = ?2"),
            params![value, id],
        )?;
        Ok(())
    }

    pub fn record_review(&self, id: i64, rating: ReviewRating) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let (mastery, interval): (u8, u32) = self.connection.query_row(
            "SELECT mastery, interval_days FROM cards WHERE id = ?1 AND archived = 0",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let now = Utc::now();
        self.connection.execute(
            "UPDATE cards SET last_reviewed_at = ?1, review_count = review_count + 1,
             correct_count = correct_count + ?2, mastery = ?3, interval_days = ?4, due_at = ?5 WHERE id = ?6",
            params![now.to_rfc3339(), rating.is_correct(), (mastery as i8 + rating.mastery_delta()).clamp(0, 5),
                rating.next_interval(interval), rating.next_due(interval, now).to_rfc3339(), id],
        )?;
        self.connection.execute(
            "INSERT INTO review_log (card_id, rating, reviewed_at) VALUES (?1, ?2, ?3)",
            params![id, rating.mastery_delta(), now.to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let cards = self.load_cards()?;
        let mut activity: HashMap<String, HashSet<i64>> = HashMap::new();
        let mut statement = self
            .connection
            .prepare("SELECT card_id, reviewed_at FROM review_log ORDER BY reviewed_at DESC")?;
        for review in statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, timestamp) = review?;
            if let Ok(date) = chrono::DateTime::parse_from_rfc3339(&timestamp) {
                activity
                    .entry(date.with_timezone(&Local).format("%Y-%m-%d").to_string())
                    .or_default()
                    .insert(id);
            }
        }
        let today = Local::now().date_naive();
        let today_key = today.format("%Y-%m-%d").to_string();
        let mut streak = 0;
        let mut day = if activity.contains_key(&today_key) {
            today
        } else {
            today - Duration::days(1)
        };
        while activity.contains_key(&day.format("%Y-%m-%d").to_string()) {
            streak += 1;
            day -= Duration::days(1);
        }
        Ok(Stats {
            total_words: cards.len(),
            total_reviews: self.connection.query_row(
                "SELECT COALESCE(SUM(review_count), 0) FROM cards",
                [],
                |row| row.get(0),
            )?,
            mastered_words: cards.iter().filter(|card| card.mastery >= 4).count(),
            reviewed_today: activity.get(&today_key).map_or(0, |ids| ids.len() as u32),
            due_words: cards.iter().filter(|card| card.is_due(Utc::now())).count(),
            streak,
            activity: (0..14)
                .rev()
                .map(|offset| {
                    let date = (today - Duration::days(offset))
                        .format("%Y-%m-%d")
                        .to_string();
                    ActivityDay {
                        count: activity.get(&date).map_or(0, |ids| ids.len() as u32),
                        date,
                    }
                })
                .collect(),
        })
    }

    pub fn backup(&self) -> Result<Backup> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut statement = self.connection.prepare(
            "SELECT cards.word, review_log.rating, review_log.reviewed_at FROM review_log
             JOIN cards ON cards.id = review_log.card_id ORDER BY review_log.id",
        )?;
        let reviews = statement
            .query_map([], |row| {
                Ok(ReviewRecord {
                    word: row.get(0)?,
                    rating: row.get(1)?,
                    reviewed_at: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let backup = Backup {
            version: 2,
            created_at: Utc::now().to_rfc3339(),
            cards: self.all_cards()?,
            phrases: self.all_phrases()?,
            reviews,
            preferences: self.preferences()?,
        };
        transaction.commit()?;
        Ok(backup)
    }

    pub fn export_backup(&self) -> Result<PathBuf> {
        let directory = self.path.parent().unwrap_or(Path::new(".")).join("backups");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "LexiCards-{}.json",
            Utc::now().format("%Y%m%d-%H%M%S-%f")
        ));
        let payload = serde_json::to_vec_pretty(&self.backup()?)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        Ok(path)
    }

    pub fn read_backup(path: &Path) -> Result<Backup> {
        if std::fs::metadata(path)?.len() > 50 * 1024 * 1024 {
            bail!("Backup exceeds the 50 MB limit.");
        }
        let mut value: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)
            .context("This is not a valid LexiCards backup.")?;
        if value["version"] == 1 {
            for key in ["cards", "phrases"] {
                if let Some(entries) = value[key].as_array_mut() {
                    for entry in entries {
                        if let Some(object) = entry.as_object_mut()
                            && let Some(translation) = object.remove("translation_tr")
                        {
                            object.insert(
                                "translations".into(),
                                serde_json::json!({"tr": translation}),
                            );
                        }
                    }
                }
            }
            value["version"] = 2.into();
        }
        let backup: Backup =
            serde_json::from_value(value).context("This is not a valid LexiCards backup.")?;
        Self::validate_backup(&backup)?;
        Ok(backup)
    }

    fn validate_backup(backup: &Backup) -> Result<()> {
        if backup.version != 2 {
            bail!("This backup version is not supported.");
        }
        if backup
            .preferences
            .translation_language
            .as_deref()
            .is_some_and(|code| !crate::language::is_supported(code))
        {
            bail!("Backup contains an unsupported language.");
        }
        let mut words = HashSet::new();
        for card in &backup.cards {
            if card
                .translations
                .keys()
                .any(|code| !crate::language::is_supported(code))
            {
                bail!("Backup contains an unsupported translation language.");
            }
            if card.word.trim().is_empty()
                || !words.insert(card.word.to_lowercase())
                || card.mastery > 5
                || card.correct_count > card.review_count
                || card.interval_days > 180
                || card
                    .due_at
                    .as_deref()
                    .is_some_and(|due| chrono::DateTime::parse_from_rfc3339(due).is_err())
            {
                bail!("Backup contains invalid or duplicate word data.");
            }
        }
        let mut phrases = HashSet::new();
        for phrase in &backup.phrases {
            if phrase
                .translations
                .keys()
                .any(|code| !crate::language::is_supported(code))
            {
                bail!("Backup contains an unsupported translation language.");
            }
            if phrase.phrase.trim().is_empty() || !phrases.insert(phrase.phrase.to_lowercase()) {
                bail!("Backup contains invalid or duplicate phrases.");
            }
        }
        for review in &backup.reviews {
            if !words.contains(&review.word.to_lowercase())
                || chrono::DateTime::parse_from_rfc3339(&review.reviewed_at).is_err()
            {
                bail!("Backup contains invalid review history.");
            }
        }
        Ok(())
    }

    pub fn restore_backup(&self, backup: &Backup) -> Result<()> {
        Self::validate_backup(backup)?;
        let transaction = self.connection.unchecked_transaction()?;
        let mut ids = HashMap::new();
        for card in &backup.cards {
            let id = self.upsert_card(card)?;
            self.connection.execute(
                "UPDATE cards SET translations_json = ?1, last_reviewed_at = ?2, review_count = ?3,
                 correct_count = ?4, mastery = ?5, due_at = ?6, interval_days = ?7, favorite = ?8,
                 notes = ?9, collocations_json = ?10, archived = ?11, created_at = ?12 WHERE id = ?13",
                params![serde_json::to_string(&card.translations)?, card.last_reviewed_at, card.review_count, card.correct_count,
                    card.mastery, card.due_at, card.interval_days, card.favorite, card.notes,
                    serde_json::to_string(&card.collocations)?, card.archived, card.created_at, id],
            )?;
            self.connection
                .execute("DELETE FROM review_log WHERE card_id = ?1", [id])?;
            ids.insert(card.word.to_lowercase(), id);
        }
        for review in &backup.reviews {
            self.connection.execute(
                "INSERT INTO review_log (card_id, rating, reviewed_at) VALUES (?1, ?2, ?3)",
                params![
                    ids[&review.word.to_lowercase()],
                    review.rating,
                    review.reviewed_at
                ],
            )?;
        }
        for phrase in &backup.phrases {
            let id = self.upsert_phrase(phrase)?;
            let mut entry = phrase.clone();
            entry.id = id;
            self.update_phrase(&entry)?;
            self.set_favorite(true, id, phrase.favorite)?;
            self.set_known(id, phrase.known)?;
            self.set_archived(true, id, phrase.archived)?;
        }
        let mut preferences = backup.preferences.clone();
        preferences.daily_goal = preferences.daily_goal.clamp(1, 200);
        self.save_preferences(&preferences)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Database;
    use crate::content;
    use crate::model::ReviewRating;

    fn path() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lexicards-test-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("lexicards.db")
    }

    fn cleanup(path: &std::path::Path) {
        let directory = path.parent().unwrap();
        assert_eq!(directory.parent().unwrap(), std::env::temp_dir());
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("lexicards-test-")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn refresh_preserves_edits_progress_and_archiving_preserves_history() {
        let path = path();
        let db = Database::open(&path).unwrap();
        let word = content::words()[0].card();
        let id = db.upsert_card(&word).unwrap();
        db.record_review(id, ReviewRating::Good).unwrap();
        db.update_word_notes(id, "tr", "benim çevirim", "My own context")
            .unwrap();
        db.upsert_card(&word).unwrap();
        let saved = db.load_cards().unwrap().remove(0);
        assert_eq!(saved.translation("tr"), "benim çevirim");
        assert_eq!(saved.notes, "My own context");
        assert_eq!(saved.review_count, 1);
        assert_eq!(saved.interval_days, 1);
        assert!(!saved.is_due(chrono::Utc::now()));
        db.set_archived(false, id, true).unwrap();
        assert!(db.load_cards().unwrap().is_empty());
        assert_eq!(db.stats().unwrap().total_reviews, 1);
        db.set_archived(false, id, false).unwrap();
        assert_eq!(db.load_cards().unwrap()[0].review_count, 1);
        drop(db);
        cleanup(&path);
    }

    #[test]
    fn seed_is_not_reinserted_after_removal_and_backup_roundtrip_keeps_learning_state() {
        let path = path();
        let db = Database::open(&path).unwrap();
        assert_eq!(db.load_phrases().unwrap().len(), 150);
        let phrase_id = db.load_phrases().unwrap()[0].id;
        db.set_archived(true, phrase_id, true).unwrap();
        db.set_known(phrase_id, true).unwrap();
        db.set_favorite(true, phrase_id, true).unwrap();
        let id = db.upsert_card(&content::words()[0].card()).unwrap();
        db.record_review(id, ReviewRating::Easy).unwrap();
        let backup = db.backup().unwrap();
        drop(db);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.load_phrases().unwrap().len(), 149);
        let restored_path = self::path();
        let restored = Database::open(&restored_path).unwrap();
        restored.restore_backup(&backup).unwrap();
        assert_eq!(restored.load_phrases().unwrap().len(), 149);
        assert_eq!(restored.load_cards().unwrap()[0].interval_days, 4);
        assert_eq!(restored.stats().unwrap().total_reviews, 1);
        restored.restore_backup(&backup).unwrap();
        assert_eq!(restored.stats().unwrap().total_reviews, 1);
        assert!(
            restored
                .backup()
                .unwrap()
                .phrases
                .iter()
                .any(|phrase| phrase.archived && phrase.known && phrase.favorite)
        );
        drop(reopened);
        drop(restored);
        cleanup(&path);
        cleanup(&restored_path);
    }

    #[test]
    fn previous_schema_migrates_without_losing_words_or_reviews() {
        let path = path();
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch("CREATE TABLE cards (id INTEGER PRIMARY KEY AUTOINCREMENT, word TEXT COLLATE NOCASE UNIQUE,
            phonetic TEXT, translation_tr TEXT NOT NULL, definitions_json TEXT NOT NULL, parts_json TEXT NOT NULL,
            synonyms_json TEXT NOT NULL, created_at TEXT NOT NULL, last_reviewed_at TEXT,
            review_count INTEGER NOT NULL DEFAULT 0, correct_count INTEGER NOT NULL DEFAULT 0, mastery INTEGER NOT NULL DEFAULT 0);
            INSERT INTO cards (word, translation_tr, definitions_json, parts_json, synonyms_json, created_at, review_count, correct_count, mastery)
            VALUES ('legacy', 'eski', '[]', '[]', '[]', '2026-09-20T00:00:00Z', 7, 5, 3);").unwrap();
        drop(old);
        let db = Database::open(&path).unwrap();
        let card = db.load_cards().unwrap().remove(0);
        assert_eq!(card.word, "legacy");
        assert_eq!(card.review_count, 7);
        assert_eq!(card.mastery, 3);
        assert_eq!(card.translation("tr"), "eski");
        assert!(db.preferences().unwrap().translation_language.is_none());
        assert!(card.is_due(chrono::Utc::now()));
        drop(db);
        cleanup(&path);
    }

    #[test]
    fn invalid_backup_is_rejected_before_any_learning_state_changes() {
        let path = path();
        let db = Database::open(&path).unwrap();
        let id = db.upsert_card(&content::words()[0].card()).unwrap();
        db.record_review(id, ReviewRating::Good).unwrap();
        let mut backup = db.backup().unwrap();
        backup.cards[0].mastery = 99;
        assert!(db.restore_backup(&backup).is_err());
        assert_eq!(db.load_cards().unwrap()[0].mastery, 1);
        assert_eq!(db.stats().unwrap().total_reviews, 1);
        drop(db);
        cleanup(&path);
    }

    #[test]
    fn translations_remain_independent_across_refresh_and_backup_restore() {
        let path = path();
        let db = Database::open(&path).unwrap();
        let card = content::words()[0].card();
        let id = db.upsert_card(&card).unwrap();
        db.update_word_notes(id, "fr", "nuance personnelle", "My notes")
            .unwrap();
        db.record_review(id, ReviewRating::Good).unwrap();
        let mut refreshed = card.clone();
        refreshed
            .translations
            .insert("fr".into(), "automatic replacement".into());
        db.upsert_card(&refreshed).unwrap();
        let phrase = db.load_phrases().unwrap().remove(0);
        db.save_translation(true, phrase.id, "de", "meine Bedeutung")
            .unwrap();
        db.save_preferences(&crate::model::Preferences {
            translation_language: Some("fr".into()),
            ..Default::default()
        })
        .unwrap();
        let saved = db.load_cards().unwrap().remove(0);
        assert_eq!(saved.translation("tr"), card.translation("tr"));
        assert_eq!(saved.translation("fr"), "nuance personnelle");
        assert_eq!(saved.translation("es"), "");
        let backup = Database::read_backup(&db.export_backup().unwrap()).unwrap();
        let restored_path = self::path();
        let restored = Database::open(&restored_path).unwrap();
        restored.restore_backup(&backup).unwrap();
        let saved = restored.load_cards().unwrap().remove(0);
        assert_eq!(saved.translations.len(), 2);
        assert_eq!(saved.translation("fr"), "nuance personnelle");
        assert_eq!(saved.review_count, 1);
        assert_eq!(
            restored
                .preferences()
                .unwrap()
                .translation_language
                .as_deref(),
            Some("fr")
        );
        assert_eq!(
            restored
                .load_phrases()
                .unwrap()
                .iter()
                .find(|entry| entry.phrase == phrase.phrase)
                .unwrap()
                .translation("de"),
            "meine Bedeutung"
        );
        drop(db);
        drop(restored);
        cleanup(&path);
        cleanup(&restored_path);
    }

    #[test]
    fn version_two_library_migrates_once_with_translations_and_history_intact() {
        let path = path();
        let db = Database::open(&path).unwrap();
        let id = db.upsert_card(&content::words()[0].card()).unwrap();
        db.record_review(id, ReviewRating::Easy).unwrap();
        db.connection
            .execute_batch(
                "ALTER TABLE cards RENAME COLUMN translations_json TO translation_tr;
             ALTER TABLE phrases RENAME COLUMN translations_json TO translation_tr;
             UPDATE cards SET translation_tr = json_extract(translation_tr, '$.tr');
             UPDATE phrases SET translation_tr = json_extract(translation_tr, '$.tr');
             PRAGMA user_version = 2;",
            )
            .unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.load_cards().unwrap()[0].translation("tr"),
            content::words()[0].translation
        );
        assert_eq!(db.load_cards().unwrap()[0].review_count, 1);
        assert_eq!(db.load_phrases().unwrap().len(), 150);
        assert!(
            db.load_phrases()
                .unwrap()
                .iter()
                .all(|entry| !entry.translation("tr").is_empty())
        );
        let snapshots = path.parent().unwrap().join("backups");
        assert_eq!(std::fs::read_dir(&snapshots).unwrap().count(), 1);
        drop(db);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(std::fs::read_dir(&snapshots).unwrap().count(), 1);
        drop(reopened);
        cleanup(&path);
    }

    #[test]
    fn version_one_json_backups_are_upgraded_without_losing_turkish_meanings() {
        let path = path();
        let db = Database::open(&path).unwrap();
        db.upsert_card(&content::words()[0].card()).unwrap();
        let mut legacy = serde_json::to_value(db.backup().unwrap()).unwrap();
        legacy["version"] = 1.into();
        legacy["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("translation_language");
        for key in ["cards", "phrases"] {
            for entry in legacy[key].as_array_mut().unwrap() {
                let object = entry.as_object_mut().unwrap();
                let translations = object.remove("translations").unwrap();
                object.insert("translation_tr".into(), translations["tr"].clone());
            }
        }
        let backup_path = path.parent().unwrap().join("legacy.json");
        std::fs::write(&backup_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let backup = Database::read_backup(&backup_path).unwrap();
        assert_eq!(backup.version, 2);
        assert_eq!(
            backup.cards[0].translation("tr"),
            content::words()[0].translation
        );
        assert!(backup.preferences.translation_language.is_none());
        db.restore_backup(&backup).unwrap();
        assert_eq!(db.load_phrases().unwrap().len(), 150);
        drop(db);
        cleanup(&path);
    }
}
