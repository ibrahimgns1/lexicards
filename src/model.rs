use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Definition {
    pub part_of_speech: String,
    pub text: String,
    pub example: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WordCard {
    pub id: i64,
    pub word: String,
    pub phonetic: Option<String>,
    pub translations: BTreeMap<String, String>,
    pub definitions: Vec<Definition>,
    pub parts_of_speech: Vec<String>,
    pub synonyms: Vec<String>,
    pub created_at: String,
    pub last_reviewed_at: Option<String>,
    pub review_count: u32,
    pub correct_count: u32,
    pub mastery: u8,
    pub due_at: Option<String>,
    pub interval_days: u32,
    pub favorite: bool,
    pub notes: String,
    pub collocations: Vec<String>,
    pub archived: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhraseEntry {
    pub id: i64,
    pub phrase: String,
    pub category: String,
    pub translations: BTreeMap<String, String>,
    pub definitions: Vec<Definition>,
    pub created_at: String,
    pub usage_note: String,
    pub register: String,
    pub topic: String,
    pub personal_example: String,
    pub favorite: bool,
    pub known: bool,
    pub archived: bool,
}

impl PhraseEntry {
    pub fn translation(&self, language: &str) -> &str {
        self.translations.get(language).map_or("", String::as_str)
    }

    pub fn primary_definition(&self) -> &str {
        self.definitions
            .first()
            .map(|definition| definition.text.as_str())
            .unwrap_or("No definition available.")
    }

    pub fn examples(&self) -> impl Iterator<Item = &str> {
        self.definitions
            .iter()
            .filter_map(|definition| definition.example.as_deref())
    }
}

impl WordCard {
    pub fn translation(&self, language: &str) -> &str {
        self.translations.get(language).map_or("", String::as_str)
    }

    pub fn primary_definition(&self) -> &str {
        self.definitions
            .first()
            .map(|definition| definition.text.as_str())
            .unwrap_or("No definition available.")
    }

    pub fn accuracy(&self) -> f32 {
        if self.review_count == 0 {
            0.0
        } else {
            self.correct_count as f32 / self.review_count as f32
        }
    }

    pub fn is_due(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.due_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|due| due <= now)
    }

    pub fn due_label(&self) -> String {
        if self.review_count == 0 {
            return "New word".to_owned();
        }
        if self.is_due(chrono::Utc::now()) {
            return "Ready to review".to_owned();
        }
        self.due_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|due| {
                format!(
                    "Next · {}",
                    due.with_timezone(&chrono::Local).format("%d %b")
                )
            })
            .unwrap_or_else(|| "Ready to review".to_owned())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewRating {
    Again,
    Hard,
    Good,
    Easy,
}

impl ReviewRating {
    pub const ALL: [Self; 4] = [Self::Again, Self::Hard, Self::Good, Self::Easy];

    pub fn label(self) -> &'static str {
        match self {
            Self::Again => "Again",
            Self::Hard => "Hard",
            Self::Good => "Good",
            Self::Easy => "Easy",
        }
    }

    pub fn next_interval(self, current: u32) -> u32 {
        match self {
            Self::Again => 0,
            Self::Hard => ((current as f32 * 1.2).ceil() as u32).max(1),
            Self::Good => {
                if current == 0 {
                    1
                } else {
                    current.saturating_mul(2)
                }
            }
            Self::Easy => {
                if current == 0 {
                    4
                } else {
                    current.saturating_mul(3).max(4)
                }
            }
        }
        .min(180)
    }

    pub fn next_due(
        self,
        current: u32,
        now: chrono::DateTime<chrono::Utc>,
    ) -> chrono::DateTime<chrono::Utc> {
        if self == Self::Again {
            now + chrono::Duration::minutes(10)
        } else {
            now + chrono::Duration::days(i64::from(self.next_interval(current)))
        }
    }

    pub fn mastery_delta(self) -> i8 {
        match self {
            Self::Again => -2,
            Self::Hard => 0,
            Self::Good => 1,
            Self::Easy => 2,
        }
    }

    pub fn is_correct(self) -> bool {
        !matches!(self, Self::Again)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub total_words: usize,
    pub total_reviews: u32,
    pub mastered_words: usize,
    pub reviewed_today: u32,
    pub due_words: usize,
    pub streak: usize,
    pub activity: Vec<ActivityDay>,
}

#[derive(Clone, Debug, Default)]
pub struct ActivityDay {
    pub date: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub translation_language: Option<String>,
    pub daily_goal: u32,
    pub hide_phrase_translations: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            translation_language: None,
            daily_goal: 10,
            hide_phrase_translations: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub word: String,
    pub rating: i8,
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backup {
    pub version: u32,
    pub created_at: String,
    pub cards: Vec<WordCard>,
    pub phrases: Vec<PhraseEntry>,
    pub reviews: Vec<ReviewRecord>,
    pub preferences: Preferences,
}

#[cfg(test)]
mod tests {
    use super::ReviewRating;

    #[test]
    fn successful_recall_extends_intervals_and_a_lapse_is_due_soon() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-28T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(ReviewRating::Good.next_interval(0), 1);
        assert_eq!(ReviewRating::Good.next_interval(4), 8);
        assert_eq!(ReviewRating::Easy.next_interval(4), 12);
        assert_eq!(ReviewRating::Easy.next_interval(100), 180);
        assert_eq!(
            ReviewRating::Again.next_due(30, now),
            now + chrono::Duration::minutes(10)
        );
        assert_eq!(
            ReviewRating::Hard.next_due(0, now),
            now + chrono::Duration::days(1)
        );
    }
}
