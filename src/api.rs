use std::collections::BTreeSet;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::model::{Definition, PhraseEntry, WordCard};

#[derive(Deserialize)]
struct DictionaryEntry {
    word: String,
    phonetic: Option<String>,
    #[serde(default)]
    phonetics: Vec<Phonetic>,
    #[serde(default)]
    meanings: Vec<Meaning>,
}

#[derive(Deserialize)]
struct Phonetic {
    text: Option<String>,
}

#[derive(Deserialize)]
struct Meaning {
    #[serde(rename = "partOfSpeech")]
    part_of_speech: String,
    #[serde(default)]
    definitions: Vec<ApiDefinition>,
    #[serde(default)]
    synonyms: Vec<String>,
}

#[derive(Deserialize)]
struct ApiDefinition {
    definition: String,
    example: Option<String>,
    #[serde(default)]
    synonyms: Vec<String>,
}

#[derive(Deserialize)]
struct TranslationEnvelope {
    #[serde(rename = "responseData")]
    response_data: TranslationData,
    #[serde(rename = "responseStatus", default)]
    response_status: serde_json::Value,
    #[serde(rename = "responseDetails", default)]
    response_details: String,
}

#[derive(Deserialize)]
struct TranslationData {
    #[serde(rename = "translatedText")]
    translated_text: String,
}

#[derive(Deserialize)]
struct DatamuseEntry {
    word: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct WiktionaryEnvelope {
    parse: WiktionaryPage,
}

#[derive(Deserialize)]
struct WiktionaryPage {
    title: String,
    wikitext: String,
}

#[derive(Default)]
struct LexicalData {
    word: String,
    phonetic: Option<String>,
    definitions: Vec<Definition>,
    parts_of_speech: Vec<String>,
    synonyms: Vec<String>,
}

fn network_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("LexiCards/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Could not create the network client")
}

pub struct Translator {
    client: Client,
    language: String,
}

impl Translator {
    pub fn new(language: &str) -> Result<Self> {
        if !crate::language::is_supported(language) {
            bail!("Please choose a supported translation language.");
        }
        Ok(Self {
            client: network_client()?,
            language: language.to_owned(),
        })
    }

    pub fn translate(&self, text: &str) -> Result<String> {
        translate_with_client(&self.client, text, &self.language)
    }
}

fn translate_with_client(client: &Client, text: &str, language: &str) -> Result<String> {
    if !crate::language::is_supported(language) {
        bail!("Please choose a supported translation language.");
    }
    if text.is_empty() || text.len() > 500 {
        bail!("Translation text must contain between 1 and 500 bytes.");
    }
    let envelope = client
        .get("https://api.mymemory.translated.net/get")
        .query(&[("q", text), ("langpair", &format!("en|{language}"))])
        .send()
        .context("Translation service could not be reached")?
        .error_for_status()?
        .json::<TranslationEnvelope>()?;
    translation_result(envelope)
}

fn translation_result(envelope: TranslationEnvelope) -> Result<String> {
    if envelope.response_status != 200 && envelope.response_status != "200" {
        bail!("Translation unavailable: {}", envelope.response_details);
    }
    let translation = envelope.response_data.translated_text.trim().to_owned();
    if translation.is_empty() || translation.to_uppercase().starts_with("MYMEMORY WARNING") {
        bail!("Translation unavailable. Try again later or add your own meaning.");
    }
    Ok(translation)
}

pub fn lookup_word(raw_word: &str, language: &str) -> Result<WordCard> {
    let word = raw_word.trim().to_lowercase();
    if word.is_empty() {
        bail!("Please enter a word.");
    }

    if !crate::language::is_supported(language) {
        bail!("Please choose a supported translation language.");
    }
    let client = network_client()?;

    // Independent services run together on the lookup worker, never on the UI.
    let (lexical, translation, common_parts) = std::thread::scope(|scope| {
        let translation =
            scope.spawn(|| translate_with_client(&client, &word, language).unwrap_or_default());
        let parts = scope.spawn(|| {
            if word.contains(char::is_whitespace) {
                Vec::new()
            } else {
                lookup_current_parts(&client, &word).unwrap_or_default()
            }
        });
        let lexical = lookup_wiktionary(&client, &word).or_else(|wiktionary_error| {
            lookup_dictionary_api(&client, &word).map_err(|fallback_error| anyhow!(
                "No definition could be loaded for “{word}”. Wiktionary: {wiktionary_error}. Fallback: {fallback_error}"
            ))
        });
        (
            lexical,
            translation.join().unwrap_or_default(),
            parts.join().unwrap_or_default(),
        )
    });
    let mut lexical = lexical?;
    if !common_parts.is_empty() {
        apply_part_filter(&mut lexical, &common_parts);
    }
    let curated = crate::content::words()
        .into_iter()
        .find(|entry| entry.word.eq_ignore_ascii_case(&lexical.word));
    let mut card = WordCard {
        word: lexical.word.to_lowercase(),
        phonetic: lexical.phonetic,
        translations: if translation.is_empty() {
            Default::default()
        } else {
            std::collections::BTreeMap::from([(language.to_owned(), translation)])
        },
        definitions: lexical.definitions,
        parts_of_speech: lexical.parts_of_speech,
        synonyms: lexical.synonyms,
        created_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };
    if let Some(entry) = curated {
        card.notes = entry.usage;
        card.collocations = entry.collocations;
    }
    Ok(card)
}

pub fn lookup_phrase(raw_phrase: &str, language: &str) -> Result<PhraseEntry> {
    let normalized = raw_phrase.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.split_whitespace().count() < 2 {
        bail!("Please enter a phrase with at least two words.");
    }
    let card = lookup_word(&normalized, language)?;
    Ok(PhraseEntry {
        phrase: card.word,
        category: classify_phrase(&normalized, &card.parts_of_speech),
        translations: card.translations,
        definitions: card.definitions,
        created_at: card.created_at,
        ..Default::default()
    })
}

fn lookup_wiktionary(client: &Client, word: &str) -> Result<LexicalData> {
    let response = client
        .get("https://en.wiktionary.org/w/api.php")
        .query(&[
            ("action", "parse"),
            ("page", word),
            ("prop", "wikitext"),
            ("format", "json"),
            ("formatversion", "2"),
        ])
        .send()
        .context("Wiktionary could not be reached")?
        .error_for_status()
        .context("Wiktionary returned an error")?;
    let envelope: WiktionaryEnvelope = response
        .json()
        .context("No Wiktionary page was found for this word")?;

    let (definitions, parts_of_speech) = parse_wiktionary(&envelope.parse.wikitext);
    if definitions.is_empty() {
        bail!("The page has no English definitions");
    }
    Ok(LexicalData {
        word: envelope.parse.title,
        definitions,
        parts_of_speech,
        ..Default::default()
    })
}

fn lookup_dictionary_api(client: &Client, word: &str) -> Result<LexicalData> {
    let dictionary_url = format!(
        "https://api.dictionaryapi.dev/api/v2/entries/en/{}",
        urlencoding::encode(word)
    );
    let response = client
        .get(dictionary_url)
        .send()
        .context("The Free Dictionary API could not be reached")?;
    if response.status().as_u16() == 404 {
        bail!("The word was not found");
    }
    let entries: Vec<DictionaryEntry> = response
        .error_for_status()
        .context("The Free Dictionary API returned an error")?
        .json()
        .context("The Free Dictionary API response could not be read")?;
    let entry = entries
        .first()
        .ok_or_else(|| anyhow!("No dictionary entry was returned"))?;

    let mut definitions: Vec<Definition> = Vec::new();
    let mut parts = BTreeSet::new();
    let mut synonyms = BTreeSet::new();
    for meaning in &entry.meanings {
        parts.insert(meaning.part_of_speech.clone());
        synonyms.extend(meaning.synonyms.iter().cloned());
        for definition in meaning.definitions.iter().take(3) {
            synonyms.extend(definition.synonyms.iter().cloned());
            definitions.push(Definition {
                part_of_speech: meaning.part_of_speech.clone(),
                text: definition.definition.clone(),
                example: definition.example.clone(),
            });
        }
    }
    if definitions.is_empty() {
        bail!("The entry has no definitions");
    }
    let phonetic = entry.phonetic.clone().or_else(|| {
        entry
            .phonetics
            .iter()
            .find_map(|phonetic| phonetic.text.clone())
    });
    Ok(LexicalData {
        word: entry.word.clone(),
        phonetic,
        definitions,
        parts_of_speech: parts.into_iter().collect(),
        synonyms: synonyms.into_iter().take(8).collect(),
    })
}

fn lookup_current_parts(client: &Client, word: &str) -> Result<Vec<String>> {
    let entries: Vec<DatamuseEntry> = client
        .get("https://api.datamuse.com/words")
        .query(&[("sp", word), ("md", "p"), ("max", "1")])
        .send()
        .context("Datamuse could not be reached")?
        .error_for_status()
        .context("Datamuse returned an error")?
        .json()
        .context("Datamuse metadata could not be read")?;
    let entry = entries
        .into_iter()
        .find(|entry| entry.word.eq_ignore_ascii_case(word))
        .ok_or_else(|| anyhow!("No corpus metadata was found"))?;
    Ok(entry
        .tags
        .into_iter()
        .filter_map(|tag| match tag.as_str() {
            "n" => Some("noun".to_owned()),
            "v" => Some("verb".to_owned()),
            "adj" => Some("adjective".to_owned()),
            "adv" => Some("adverb".to_owned()),
            _ => None,
        })
        .collect())
}

fn apply_part_filter(lexical: &mut LexicalData, common_parts: &[String]) {
    let filtered_definitions: Vec<Definition> = lexical
        .definitions
        .iter()
        .filter(|definition| {
            common_parts
                .iter()
                .any(|part| part.eq_ignore_ascii_case(&definition.part_of_speech))
        })
        .cloned()
        .collect();
    // Corpus metadata can occasionally be incomplete or use a broader sense
    // than the dictionary page. Keep the source data intact if it would leave
    // us with no definitions at all.
    if filtered_definitions.is_empty() {
        return;
    }
    lexical.definitions = filtered_definitions;
    lexical.parts_of_speech = common_parts
        .iter()
        .filter(|part| {
            lexical
                .definitions
                .iter()
                .any(|definition| definition.part_of_speech.eq_ignore_ascii_case(part))
        })
        .cloned()
        .collect();
    lexical.definitions.sort_by_key(|definition| {
        common_parts
            .iter()
            .position(|part| part.eq_ignore_ascii_case(&definition.part_of_speech))
            .unwrap_or(usize::MAX)
    });
}

fn parse_wiktionary(wikitext: &str) -> (Vec<Definition>, Vec<String>) {
    let mut in_english = false;
    let mut current_part: Option<String> = None;
    let mut current_definition: Option<usize> = None;
    let mut definitions: Vec<Definition> = Vec::new();
    let mut parts = BTreeSet::new();

    for raw_line in wikitext.lines() {
        let line = raw_line.trim();
        if line == "==English==" {
            in_english = true;
            current_part = None;
            current_definition = None;
            continue;
        }
        if !in_english {
            continue;
        }
        if line.starts_with("==") && line.ends_with("==") {
            let heading_level = line
                .chars()
                .take_while(|character| *character == '=')
                .count();
            if heading_level == 2 {
                break;
            }
            let heading = line.trim_matches('=').trim().to_lowercase();
            current_definition = None;
            let normalized_heading = heading
                .trim_end_matches(|character: char| {
                    character.is_ascii_digit() || character.is_whitespace()
                })
                .to_owned();
            if heading_level >= 3 && is_part_of_speech(&normalized_heading) {
                parts.insert(normalized_heading.clone());
                current_part = Some(normalized_heading);
            } else {
                current_part = None;
            }
            continue;
        }
        let Some(part_of_speech) = &current_part else {
            continue;
        };
        if let Some(raw_example) = line
            .strip_prefix("#: ")
            .or_else(|| line.strip_prefix("#:"))
            .or_else(|| line.strip_prefix("#* "))
            .or_else(|| line.strip_prefix("#*"))
        {
            if let Some(example) = extract_example(raw_example)
                && let Some(definition) =
                    current_definition.and_then(|index| definitions.get_mut(index))
                && definition.example.is_none()
            {
                definition.example = Some(example);
            }
            continue;
        }
        let Some(raw_definition) = line.strip_prefix("# ") else {
            continue;
        };
        current_definition = None;
        if is_non_current_definition(raw_definition) {
            continue;
        }
        let text = clean_wikitext(raw_definition);
        let definitions_for_part = definitions
            .iter()
            .filter(|definition: &&Definition| {
                definition
                    .part_of_speech
                    .eq_ignore_ascii_case(part_of_speech)
            })
            .count();
        if !text.is_empty() && definitions_for_part < 3 {
            definitions.push(Definition {
                part_of_speech: part_of_speech.clone(),
                text,
                example: None,
            });
            current_definition = Some(definitions.len() - 1);
        }
    }
    parts.retain(|part| {
        definitions
            .iter()
            .any(|definition| definition.part_of_speech.eq_ignore_ascii_case(part))
    });
    (definitions, parts.into_iter().collect())
}

fn is_non_current_definition(value: &str) -> bool {
    let lowercase = value.to_lowercase();
    [
        "|obsolete",
        "|archaic",
        "|rare",
        "|historical",
        "|dialectal",
        "|nonstandard",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
}

fn is_part_of_speech(heading: &str) -> bool {
    matches!(
        heading,
        "noun"
            | "proper noun"
            | "verb"
            | "adjective"
            | "adverb"
            | "pronoun"
            | "preposition"
            | "conjunction"
            | "interjection"
            | "determiner"
            | "article"
            | "numeral"
            | "particle"
            | "phrase"
            | "prepositional phrase"
            | "adverbial phrase"
            | "contraction"
            | "idiom"
            | "proverb"
    )
}

fn extract_example(value: &str) -> Option<String> {
    let usage =
        Regex::new(r"\{\{(?:ux|uxi|usex)\|en\|([^|}]+)").expect("valid usage-example regex");
    let passage = Regex::new(r"\|passage=([^|}]+)").expect("valid quotation-passage regex");
    let candidate = usage
        .captures(value)
        .and_then(|captures| captures.get(1))
        .or_else(|| passage.captures(value).and_then(|captures| captures.get(1)))
        .map(|capture| capture.as_str())
        .or_else(|| (!value.contains("{{")).then_some(value))?;
    let cleaned = clean_wikitext(candidate);
    (!cleaned.is_empty()).then_some(cleaned)
}

pub fn classify_phrase(phrase: &str, parts_of_speech: &[String]) -> String {
    if let Some(entry) = crate::content::starter_phrases()
        .iter()
        .find(|entry| entry.phrase.eq_ignore_ascii_case(phrase.trim()))
    {
        return entry.category.clone();
    }
    let lowercase_phrase = phrase.to_lowercase();
    let words: Vec<&str> = lowercase_phrase.split_whitespace().collect();
    let first = words.first().copied().unwrap_or_default();
    let last = words.last().copied().unwrap_or_default();
    let verbs = [
        "account", "back", "bear", "boil", "brush", "catch", "crack", "draw", "dwell", "figure",
        "follow", "iron", "lay", "live", "narrow", "phase", "point", "pull", "rule", "stand",
        "stem", "weigh", "zero", "be", "break", "bring", "call", "carry", "come", "cut", "do",
        "fall", "get", "give", "go", "hold", "keep", "look", "make", "put", "run", "set", "take",
        "turn", "work",
    ];
    let particles = [
        "about", "across", "after", "around", "away", "back", "by", "down", "for", "in", "into",
        "off", "on", "out", "over", "through", "to", "up", "with",
    ];
    let prepositions = [
        "at", "by", "for", "from", "in", "into", "of", "on", "to", "under", "with", "without",
    ];

    if verbs.contains(&first) && particles.contains(&last) {
        "Phrasal verbs".to_owned()
    } else if prepositions.contains(&first)
        || phrase.contains("...")
        || phrase.contains('…')
        || [
            "it",
            "there",
            "this",
            "what",
            "provided",
            "regardless",
            "while",
            "whereas",
        ]
        .contains(&first)
    {
        "Reusable patterns".to_owned()
    } else if parts_of_speech
        .iter()
        .any(|part| matches!(part.as_str(), "idiom" | "proverb"))
        || words.len() >= 4
    {
        "Idioms".to_owned()
    } else {
        "Reusable patterns".to_owned()
    }
}

fn clean_wikitext(value: &str) -> String {
    static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{[^{}]*\}\}").unwrap());
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[\[(?:[^\]|]*\|)?([^\]]+)\]\]").unwrap());
    static EXTERNAL_LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[https?://[^\s\]]+\s+([^\]]+)\]").unwrap());
    static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

    let mut cleaned = value.to_owned();
    loop {
        let next = TEMPLATE.replace_all(&cleaned, "").into_owned();
        if next == cleaned {
            break;
        }
        cleaned = next;
    }
    cleaned = LINK.replace_all(&cleaned, "$1").into_owned();
    cleaned = EXTERNAL_LINK.replace_all(&cleaned, "$1").into_owned();
    cleaned = HTML_TAG.replace_all(&cleaned, "").into_owned();
    cleaned = cleaned
        .replace("'''", "")
        .replace("''", "")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        Definition, LexicalData, apply_part_filter, classify_phrase, lookup_phrase, lookup_word,
        parse_wiktionary,
    };

    #[test]
    fn wiktionary_parser_extracts_english_parts_and_definitions() {
        let source = r#"==French==
===Noun===
# Not English.
==English==
===Noun===
# A [[representative]] {{lb|en|countable}} form.
===Verb===
# To [[illustrate|show]] by example.
#: {{ux|en|She showed the idea by example.}}
====Synonyms====
* demonstrate
===Etymology 2===
====Adjective 1====
# {{lb|en|rare}} Serving as an example.
==Turkish==
===Noun===
# örnek
"#;
        let (definitions, parts) = parse_wiktionary(source);
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].text, "A representative form.");
        assert_eq!(definitions[1].text, "To show by example.");
        assert_eq!(
            definitions[1].example.as_deref(),
            Some("She showed the idea by example.")
        );
        assert_eq!(parts, vec!["noun", "verb"]);
    }

    #[test]
    fn phrases_are_classified_into_useful_groups() {
        assert_eq!(classify_phrase("keep it up", &[]), "Phrasal verbs");
        assert_eq!(
            classify_phrase("on the other hand", &[]),
            "Reusable patterns"
        );
        assert_eq!(classify_phrase("a blessing in disguise", &[]), "Idioms");
        assert_eq!(classify_phrase("strong coffee", &[]), "Reusable patterns");
    }

    #[test]
    fn obsolete_senses_do_not_create_misleading_word_classes() {
        let source = r#"==English==
===Adjective===
# Generous or abundant.
===Noun===
# {{lb|en|obsolete}} Excessive abundance.
"#;
        let (definitions, parts) = parse_wiktionary(source);
        assert_eq!(definitions.len(), 1);
        assert_eq!(parts, vec!["adjective"]);
    }

    #[test]
    fn a_skipped_old_sense_cannot_supply_the_example_for_a_current_sense() {
        let source = "==English==\n===Verb===\n# A current use.\n# {{lb|en|obsolete}} An old use.\n#: {{ux|en|An obsolete example.}}\n";
        let (definitions, _) = parse_wiktionary(source);
        assert_eq!(definitions.len(), 1);
        assert!(definitions[0].example.is_none());
    }

    #[test]
    fn corpus_order_keeps_current_primary_use_and_drops_unmatched_senses() {
        let mut lexical = LexicalData {
            definitions: vec![
                Definition {
                    part_of_speech: "noun".into(),
                    text: "An obsolete noun sense.".into(),
                    example: None,
                },
                Definition {
                    part_of_speech: "adjective".into(),
                    text: "Very generous or abundant.".into(),
                    example: None,
                },
                Definition {
                    part_of_speech: "verb".into(),
                    text: "To give or spend generously.".into(),
                    example: None,
                },
            ],
            parts_of_speech: vec!["noun".into(), "adjective".into(), "verb".into()],
            ..Default::default()
        };
        apply_part_filter(&mut lexical, &["adjective".into(), "verb".into()]);
        assert_eq!(lexical.parts_of_speech, vec!["adjective", "verb"]);
        assert_eq!(
            lexical
                .definitions
                .iter()
                .map(|definition| definition.part_of_speech.as_str())
                .collect::<Vec<_>>(),
            vec!["adjective", "verb"]
        );
    }

    #[test]
    #[ignore = "calls the public dictionary and translation services"]
    fn live_lookup_returns_a_complete_card() {
        let card = lookup_word("example", "tr").expect("live lookup should succeed");
        assert_eq!(card.word, "example");
        assert!(!card.definitions.is_empty());
        assert!(!card.parts_of_speech.is_empty());
        assert!(!card.translation("tr").trim().is_empty());
    }

    #[test]
    fn translation_service_errors_cannot_be_saved_as_word_meanings() {
        let envelope = serde_json::from_value(serde_json::json!({
            "responseData": {"translatedText": "MYMEMORY WARNING: quota exceeded"},
            "responseStatus": 403,
            "responseDetails": "Daily limit reached"
        }))
        .unwrap();
        assert!(super::translation_result(envelope).is_err());
        let envelope = serde_json::from_value(serde_json::json!({
            "responseData": {"translatedText": "exemple"},
            "responseStatus": "200",
            "responseDetails": ""
        }))
        .unwrap();
        assert_eq!(super::translation_result(envelope).unwrap(), "exemple");
    }

    #[test]
    #[ignore = "calls the public dictionary and translation services"]
    fn live_lookup_uses_the_selected_translation_language() {
        let card = lookup_word("example", "fr").expect("French lookup should succeed");
        assert!(card.translation("fr").to_lowercase().contains("exemple"));
        assert!(card.translation("tr").is_empty());
        assert!(!card.definitions.is_empty());
        let translation = super::Translator::new("es")
            .unwrap()
            .translate("example")
            .unwrap();
        assert!(translation.to_lowercase().contains("ejemplo"));
    }

    #[test]
    #[ignore = "calls the public dictionary and translation services"]
    fn live_phrase_lookup_returns_examples_and_category() {
        let phrase = lookup_phrase("keep it up", "tr").expect("live phrase lookup should succeed");
        assert_eq!(phrase.category, "Phrasal verbs");
        assert!(!phrase.definitions.is_empty());
        assert!(phrase.examples().next().is_some());
        assert!(!phrase.translation("tr").trim().is_empty());
    }

    #[test]
    #[ignore = "calls public dictionary and corpus services"]
    fn live_lavish_lookup_prioritizes_current_usage() {
        let card = lookup_word("lavish", "tr").expect("live word lookup should succeed");
        assert_eq!(
            card.parts_of_speech.first().map(String::as_str),
            Some("adjective")
        );
        assert!(!card.parts_of_speech.iter().any(|part| part == "noun"));
    }
}
