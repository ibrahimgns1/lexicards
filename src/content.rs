use serde::Deserialize;

use crate::model::{Definition, PhraseEntry, WordCard};

pub const COLLECTIONS: [&str; 3] = ["Precise thinking", "Work & impact", "People & perspectives"];

#[derive(Clone, Debug, Deserialize)]
pub struct CuratedWord {
    pub collection: String,
    pub word: String,
    pub translation: String,
    pub part: String,
    pub definition: String,
    pub example: String,
    pub collocations: Vec<String>,
    pub usage: String,
}

impl CuratedWord {
    pub fn card(&self) -> WordCard {
        WordCard {
            word: self.word.clone(),
            translations: std::collections::BTreeMap::from([(
                "tr".into(),
                self.translation.clone(),
            )]),
            definitions: vec![Definition {
                part_of_speech: self.part.clone(),
                text: self.definition.clone(),
                example: Some(self.example.clone()),
            }],
            parts_of_speech: vec![self.part.clone()],
            notes: self.usage.clone(),
            collocations: self.collocations.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        }
    }
}

pub fn words() -> Vec<CuratedWord> {
    serde_json::from_str(include_str!("../data/word_collections.json"))
        .expect("bundled word collections must be valid")
}

pub fn starter_phrases() -> Vec<PhraseEntry> {
    include_str!("../data/phrase_starter_pack.tsv")
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            assert_eq!(columns.len(), 5, "starter phrase must contain five fields");
            let mut entry = PhraseEntry {
                category: columns[0].into(),
                phrase: columns[1].into(),
                translations: std::collections::BTreeMap::from([("tr".into(), columns[2].into())]),
                definitions: vec![Definition {
                    part_of_speech: "phrase".into(),
                    text: columns[3].into(),
                    example: Some(columns[4].into()),
                }],
                created_at: "2026-09-20T00:00:00Z".into(),
                ..Default::default()
            };
            enrich_phrase(&mut entry);
            entry
        })
        .collect()
}

pub fn enrich_phrase(entry: &mut PhraseEntry) {
    if let Some(line) = include_str!("../data/phrase_usage.tsv")
        .lines()
        .skip(1)
        .find(|line| {
            line.split('\t')
                .next()
                .is_some_and(|phrase| phrase.eq_ignore_ascii_case(&entry.phrase))
        })
    {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() == 4 {
            if entry.register.is_empty() {
                entry.register = columns[1].into();
            }
            if entry.topic.is_empty() {
                entry.topic = columns[2].into();
            }
            if entry.usage_note.is_empty() {
                entry.usage_note = columns[3].into();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{COLLECTIONS, starter_phrases, words};
    use std::collections::HashSet;

    #[test]
    fn bundled_content_is_complete_unique_and_has_practical_usage() {
        let phrases = starter_phrases();
        assert_eq!(phrases.len(), 150);
        let mut unique = HashSet::new();
        for phrase in &phrases {
            assert!(unique.insert(phrase.phrase.to_lowercase()));
            assert!(
                !phrase.usage_note.is_empty(),
                "missing usage note: {}",
                phrase.phrase
            );
            assert!(!phrase.register.is_empty());
            assert!(!phrase.topic.is_empty());
            assert!(phrase.examples().next().is_some());
        }
        for category in ["Idioms", "Phrasal verbs", "Reusable patterns"] {
            assert_eq!(
                phrases
                    .iter()
                    .filter(|entry| entry.category == category)
                    .count(),
                50
            );
        }
        let words = words();
        assert_eq!(words.len(), 36);
        let mut unique = HashSet::new();
        for word in &words {
            assert!(unique.insert(word.word.to_lowercase()));
            assert!(word.collocations.len() >= 2);
            assert!(!word.example.is_empty());
            assert!(!word.usage.is_empty());
        }
        for collection in COLLECTIONS {
            assert_eq!(
                words
                    .iter()
                    .filter(|word| word.collection == collection)
                    .count(),
                12
            );
        }
    }
}
