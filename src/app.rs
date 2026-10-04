use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke, Vec2};
use rand::seq::SliceRandom;

use crate::content::CuratedWord;
use crate::db::Database;
use crate::model::{Backup, Definition, PhraseEntry, Preferences, ReviewRating, Stats, WordCard};
use crate::{api, content, language, theme};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Today,
    Deck,
    Phrasebook,
    Collections,
    Study,
    Progress,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StudyOrder {
    Due,
    Smart,
    Random,
    Favorites,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StudyDirection {
    EnglishToTranslation,
    TranslationToEnglish,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeckFilter {
    All,
    Due,
    New,
    Favorites,
    Mastered,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeckSort {
    Newest,
    Alphabetical,
    Weakest,
}

#[derive(Default, Clone)]
pub struct LaunchOptions {
    pub page: Option<String>,
    pub capture: Option<PathBuf>,
}

#[derive(Default)]
struct Session {
    ids: Vec<i64>,
    index: usize,
    revealed: bool,
    retried: HashSet<i64>,
    first_pass: HashSet<i64>,
    good: usize,
    ratings: usize,
    started: Option<Instant>,
    elapsed_seconds: u64,
    complete: bool,
}

enum TranslationEvent {
    Ready {
        is_phrase: bool,
        id: i64,
        language: String,
        translation: String,
    },
    Failed(String),
}

pub struct LexiCardsApp {
    db: Database,
    cards: Vec<WordCard>,
    phrases: Vec<PhraseEntry>,
    words: Vec<CuratedWord>,
    stats: Stats,
    preferences: Preferences,
    language_choice: String,
    translation_receiver: Option<Receiver<TranslationEvent>>,
    translations_done: usize,
    translations_total: usize,
    translation_error: Option<String>,
    page: Page,
    new_word: String,
    search: String,
    deck_filter: DeckFilter,
    deck_sort: DeckSort,
    part_filter: String,
    add_receiver: Option<Receiver<anyhow::Result<WordCard>>>,
    busy: bool,
    refresh_lookup: bool,
    new_phrase: String,
    phrase_search: String,
    phrase_category: String,
    phrase_status: String,
    phrase_topic: String,
    phrase_receiver: Option<Receiver<anyhow::Result<PhraseEntry>>>,
    phrase_busy: bool,
    expanded: HashSet<i64>,
    revealed_phrases: HashSet<i64>,
    personal_examples: HashMap<i64, String>,
    phrase_editor: Option<PhraseEntry>,
    phrase_edit_definition: String,
    phrase_edit_example: String,
    phrase_edit_translation: String,
    phrase_edit_error: String,
    toast: Option<(String, bool, f64)>,
    undo: Option<(bool, i64)>,
    detail_id: Option<i64>,
    detail_translation: String,
    detail_notes: String,
    flipped: HashSet<i64>,
    collection: String,
    study_size: usize,
    custom_study_size: usize,
    study_order: StudyOrder,
    study_direction: StudyDirection,
    session: Session,
    backup_path: String,
    backup_preview: Option<Backup>,
    last_backup: Option<PathBuf>,
    launch: LaunchOptions,
    frames: usize,
    last_reload: Instant,
    focus_search: bool,
    focus_add: bool,
}

impl LexiCardsApp {
    pub fn new(ctx: &egui::Context, db: Database, launch: LaunchOptions) -> Self {
        let preferences = db.preferences().unwrap_or_default();
        theme::apply(ctx, preferences.translation_language.as_deref());
        let cards = db.load_cards().unwrap_or_default();
        let phrases = db.load_phrases().unwrap_or_default();
        let stats = db.stats().unwrap_or_default();
        let page = match launch.page.as_deref() {
            Some("deck") => Page::Deck,
            Some("phrases") => Page::Phrasebook,
            Some("collections") => Page::Collections,
            Some("study") => Page::Study,
            Some("progress") => Page::Progress,
            Some("settings") => Page::Settings,
            _ => Page::Today,
        };
        let mut app = Self {
            db,
            cards,
            phrases,
            stats,
            language_choice: preferences
                .translation_language
                .clone()
                .unwrap_or_else(|| "tr".into()),
            preferences,
            translation_receiver: None,
            translations_done: 0,
            translations_total: 0,
            translation_error: None,
            page,
            words: content::words(),
            new_word: String::new(),
            search: String::new(),
            deck_filter: DeckFilter::All,
            deck_sort: DeckSort::Newest,
            part_filter: "All word classes".into(),
            add_receiver: None,
            busy: false,
            refresh_lookup: false,
            new_phrase: String::new(),
            phrase_search: String::new(),
            phrase_category: "All".into(),
            phrase_status: "All".into(),
            phrase_topic: "All topics".into(),
            phrase_receiver: None,
            phrase_busy: false,
            expanded: HashSet::new(),
            revealed_phrases: HashSet::new(),
            personal_examples: HashMap::new(),
            phrase_editor: None,
            phrase_edit_definition: String::new(),
            phrase_edit_example: String::new(),
            phrase_edit_translation: String::new(),
            phrase_edit_error: String::new(),
            toast: None,
            undo: None,
            detail_id: None,
            detail_translation: String::new(),
            detail_notes: String::new(),
            flipped: HashSet::new(),
            collection: content::COLLECTIONS[0].into(),
            study_size: 10,
            custom_study_size: 15,
            study_order: StudyOrder::Due,
            study_direction: StudyDirection::EnglishToTranslation,
            session: Session::default(),
            backup_path: String::new(),
            backup_preview: None,
            last_backup: None,
            launch,
            frames: 0,
            last_reload: Instant::now(),
            focus_search: false,
            focus_add: false,
        };
        if app.launch.capture.is_some()
            && app.page == Page::Phrasebook
            && let Some(phrase) = app.phrases.first()
        {
            app.expanded.insert(phrase.id);
        }
        if app.launch.page.as_deref() == Some("study-active") {
            app.start_session(StudyOrder::Smart, ctx);
        }
        if app.launch.page.as_deref() == Some("study-answer") {
            app.start_session(StudyOrder::Smart, ctx);
            app.session.revealed = true;
        }
        if app.launch.page.as_deref() == Some("detail") {
            app.page = Page::Deck;
            if let Some(card) = app.cards.first() {
                app.open_word(card.id);
            }
        }
        app.queue_translations();
        app
    }

    fn language(&self) -> &str {
        self.preferences
            .translation_language
            .as_deref()
            .unwrap_or("tr")
    }

    fn choose_language(&mut self, ctx: &egui::Context) {
        let mut preferences = self.preferences.clone();
        preferences.translation_language = Some(self.language_choice.clone());
        match self.db.save_preferences(&preferences) {
            Ok(()) => {
                self.preferences = preferences;
                self.translation_receiver = None;
                self.translation_error = None;
                self.translations_done = 0;
                self.translations_total = 0;
                self.session = Session::default();
                self.detail_id = None;
                self.phrase_editor = None;
                self.flipped.clear();
                self.revealed_phrases.clear();
                theme::apply(ctx, Some(self.language()));
                self.queue_translations();
            }
            Err(error) => self.notify(format!("Could not save language: {error}"), true, ctx),
        }
    }

    fn queue_translations(&mut self) {
        if self.preferences.translation_language.is_none()
            || self.translation_receiver.is_some()
            || self.translation_error.is_some()
        {
            return;
        }
        let language = self.language().to_owned();
        let tasks: Vec<(bool, i64, String)> = self
            .cards
            .iter()
            .filter(|card| card.translation(&language).trim().is_empty())
            .map(|card| (false, card.id, card.word.clone()))
            .chain(
                self.phrases
                    .iter()
                    .filter(|phrase| phrase.translation(&language).trim().is_empty())
                    .map(|phrase| (true, phrase.id, phrase.phrase.clone())),
            )
            .collect();
        if tasks.is_empty() {
            return;
        }
        self.translations_done = 0;
        self.translations_total = tasks.len();
        let (sender, receiver) = mpsc::channel();
        self.translation_receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let translator = api::Translator::new(&language)?;
                for (is_phrase, id, text) in tasks {
                    let translation = translator.translate(&text)?;
                    if sender
                        .send(TranslationEvent::Ready {
                            is_phrase,
                            id,
                            language: language.clone(),
                            translation,
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                    // Pace anonymous requests; the UI stays responsive.
                    std::thread::sleep(Duration::from_millis(250));
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = sender.send(TranslationEvent::Failed(error.to_string()));
            }
        });
    }

    fn poll_translations(&mut self) {
        let mut changed = false;
        let mut disconnected = false;
        if let Some(receiver) = &self.translation_receiver {
            loop {
                match receiver.try_recv() {
                    Ok(TranslationEvent::Ready {
                        is_phrase,
                        id,
                        language,
                        translation,
                    }) => {
                        let already_saved =
                            if is_phrase {
                                self.phrases
                                    .iter()
                                    .find(|entry| entry.id == id)
                                    .is_some_and(|entry| {
                                        !entry.translation(&language).trim().is_empty()
                                    })
                            } else {
                                self.cards.iter().find(|entry| entry.id == id).is_some_and(
                                    |entry| !entry.translation(&language).trim().is_empty(),
                                )
                            };
                        if !already_saved {
                            match self
                                .db
                                .save_translation(is_phrase, id, &language, &translation)
                            {
                                Ok(()) => changed = true,
                                Err(error) => self.translation_error = Some(error.to_string()),
                            }
                        }
                        self.translations_done += 1;
                    }
                    Ok(TranslationEvent::Failed(error)) => self.translation_error = Some(error),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if changed {
            self.reload();
        }
        if disconnected {
            self.translation_receiver = None;
            self.queue_translations();
        }
    }

    fn language_setup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(28)))
            .show(ctx, |ui| {
                ui.add_space(((ui.available_height() - 320.0) / 2.0).max(12.0));
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    surface().show(ui, |ui| {
                        ui.set_width(420.0);
                        eyebrow(ui, "WELCOME TO LEXICARDS");
                        ui.label(RichText::new("Make English your own.").size(28.0).strong().color(theme::INK));
                        ui.add_space(12.0);
                        ui.label("Which language should your word and expression meanings use?");
                        ui.add_space(12.0);
                        language_picker(ui, &mut self.language_choice, "setup-language");
                        ui.add_space(16.0);
                        muted(ui, "Definitions and examples stay in English. You can change this later in Settings.");
                        muted_small(ui, "New translations need internet access. Saved translations are available offline.");
                        ui.add_space(18.0);
                        if primary_button(ui, "Start learning →").clicked() {
                            self.choose_language(ctx);
                        }
                    });
                });
            });
    }

    fn translation_status(&mut self, ctx: &egui::Context) {
        if self.translation_receiver.is_none() && self.translation_error.is_none() {
            return;
        }
        egui::TopBottomPanel::top("translation-status")
            .frame(Frame::new().fill(theme::ACCENT_SOFT).inner_margin(Margin::symmetric(28, 10)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.translation_receiver.is_some() {
                        ui.spinner();
                        ui.label(format!("Preparing {} meanings · {} / {}", language::name(self.language()), self.translations_done, self.translations_total));
                    } else if let Some(error) = &self.translation_error {
                        ui.label(RichText::new("Translations paused").color(theme::RED)).on_hover_text(error);
                        muted_small(ui, "Check your connection or try later. You can also write meanings in Details.");
                        if text_button(ui, "Retry").clicked() {
                            self.translation_error = None;
                            self.queue_translations();
                        }
                    }
                });
            });
    }

    fn reload(&mut self) {
        if let Ok(cards) = self.db.load_cards() {
            self.cards = cards;
        }
        if let Ok(phrases) = self.db.load_phrases() {
            self.phrases = phrases;
        }
        if let Ok(stats) = self.db.stats() {
            self.stats = stats;
        }
        self.last_reload = Instant::now();
    }

    fn notify(&mut self, message: impl Into<String>, error: bool, ctx: &egui::Context) {
        self.toast = Some((message.into(), error, ctx.input(|input| input.time)));
        ctx.request_repaint();
    }

    fn open_word(&mut self, id: i64) {
        if let Some(card) = self.cards.iter().find(|card| card.id == id) {
            self.detail_id = Some(id);
            self.detail_translation = card.translation(self.language()).to_owned();
            self.detail_notes = card.notes.clone();
        }
    }

    fn add_word(&mut self, ctx: &egui::Context, refresh: bool) {
        let word = self.new_word.trim().to_lowercase();
        if word.is_empty() || self.busy {
            return;
        }
        if !refresh {
            if let Some(card) = self
                .cards
                .iter()
                .find(|card| card.word.eq_ignore_ascii_case(&word))
            {
                self.open_word(card.id);
                self.new_word.clear();
                self.notify("This word is already in your deck.", false, ctx);
                return;
            }
            if let Some(entry) = self.words.iter().find(|entry| entry.word == word) {
                self.save_word(entry.card(), ctx);
                return;
            }
        }
        let (sender, receiver) = mpsc::channel();
        self.add_receiver = Some(receiver);
        self.busy = true;
        self.refresh_lookup = refresh;
        let language = self.language().to_owned();
        std::thread::spawn(move || {
            let _ = sender.send(api::lookup_word(&word, &language));
        });
    }

    fn save_word(&mut self, card: WordCard, ctx: &egui::Context) {
        match self.db.upsert_card(&card) {
            Ok(id) => {
                self.new_word.clear();
                self.reload();
                self.queue_translations();
                self.open_word(id);
                self.notify(
                    if self.refresh_lookup {
                        "Word data refreshed. Your progress and notes are kept."
                    } else {
                        "Word added. It is ready for your next session."
                    },
                    false,
                    ctx,
                );
            }
            Err(error) => self.notify(format!("Could not save word: {error}"), true, ctx),
        }
        self.refresh_lookup = false;
    }

    fn edit_phrase(&mut self, phrase: PhraseEntry) {
        self.phrase_edit_translation = phrase.translation(self.language()).to_owned();
        self.phrase_edit_definition = phrase.primary_definition().to_owned();
        if phrase.definitions.is_empty() {
            self.phrase_edit_definition.clear();
        }
        self.phrase_edit_example = phrase.examples().next().unwrap_or_default().to_owned();
        self.phrase_edit_error.clear();
        self.phrase_editor = Some(phrase);
    }

    fn add_phrase(&mut self, ctx: &egui::Context) {
        let phrase = self
            .new_phrase
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if phrase.is_empty() || self.phrase_busy {
            return;
        }
        if let Some(entry) = self
            .phrases
            .iter()
            .find(|entry| entry.phrase.eq_ignore_ascii_case(&phrase))
        {
            self.phrase_category = "All".into();
            self.phrase_status = "All".into();
            self.phrase_topic = "All topics".into();
            self.phrase_search = entry.phrase.clone();
            self.expanded.insert(entry.id);
            self.new_phrase.clear();
            self.notify("This expression is already in your phrasebook.", false, ctx);
            return;
        }
        if let Some(entry) = content::starter_phrases()
            .into_iter()
            .find(|entry| entry.phrase.eq_ignore_ascii_case(&phrase))
        {
            self.save_phrase(entry, ctx);
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.phrase_receiver = Some(receiver);
        self.phrase_busy = true;
        let language = self.language().to_owned();
        std::thread::spawn(move || {
            let _ = sender.send(api::lookup_phrase(&phrase, &language));
        });
    }

    fn save_phrase(&mut self, mut phrase: PhraseEntry, ctx: &egui::Context) -> bool {
        content::enrich_phrase(&mut phrase);
        match self.db.upsert_phrase(&phrase) {
            Ok(id) => {
                self.new_phrase.clear();
                self.reload();
                self.queue_translations();
                self.expanded.insert(id);
                self.phrase_category = "All".into();
                self.phrase_status = "All".into();
                self.phrase_topic = "All topics".into();
                self.phrase_search = phrase.phrase;
                self.notify("Expression added to your phrasebook.", false, ctx);
                true
            }
            Err(error) => {
                self.notify(format!("Could not save expression: {error}"), true, ctx);
                false
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_translations();
        if let Some(result) = self
            .add_receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.add_receiver = None;
            self.busy = false;
            match result {
                Ok(card) => self.save_word(card, ctx),
                Err(error) => {
                    self.refresh_lookup = false;
                    self.notify(error.to_string(), true, ctx);
                }
            }
        }
        if let Some(result) = self
            .phrase_receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.phrase_receiver = None;
            self.phrase_busy = false;
            match result {
                Ok(phrase) => {
                    self.save_phrase(phrase, ctx);
                }
                Err(_) => {
                    self.edit_phrase(PhraseEntry {
                        phrase: self.new_phrase.trim().into(),
                        category: api::classify_phrase(&self.new_phrase, &[]),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        ..Default::default()
                    });
                    self.phrase_edit_error = "No exact dictionary entry was available. You can save your own expression below.".into();
                }
            }
        }
        if self.busy || self.phrase_busy || self.translation_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
        if self.last_reload.elapsed() > Duration::from_secs(30) {
            self.reload();
        }
    }

    fn flag(&mut self, phrase: bool, id: i64, value: bool, ctx: &egui::Context) {
        if let Err(error) = self.db.set_favorite(phrase, id, value) {
            self.notify(error.to_string(), true, ctx);
        } else {
            self.reload();
        }
    }

    fn archive(&mut self, phrase: bool, id: i64, ctx: &egui::Context) {
        match self.db.set_archived(phrase, id, true) {
            Ok(()) => {
                self.undo = Some((phrase, id));
                self.detail_id = None;
                self.reload();
                self.notify(
                    "Removed from your list. Undo is available below.",
                    false,
                    ctx,
                );
            }
            Err(error) => self.notify(error.to_string(), true, ctx),
        }
    }

    fn persist_preferences(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.db.save_preferences(&self.preferences) {
            self.notify(error.to_string(), true, ctx);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let compact = ctx.content_rect().height() < 720.0;
        egui::SidePanel::left("sidebar")
            .exact_width(208.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.label(
                    RichText::new("lexi / cards")
                        .size(24.0)
                        .strong()
                        .color(theme::SIDEBAR_TEXT),
                );
                ui.label(
                    RichText::new("A little practice. Lasting recall.")
                        .size(11.0)
                        .color(theme::SIDEBAR_MUTED),
                );
                ui.add_space(if compact { 20.0 } else { 34.0 });
                for (page, number, label) in [
                    (Page::Today, "01", "Today"),
                    (Page::Deck, "02", "My deck"),
                    (Page::Phrasebook, "03", "Phrasebook"),
                    (Page::Collections, "04", "Collections"),
                    (Page::Study, "05", "Study"),
                    (Page::Progress, "06", "Progress"),
                ] {
                    let selected = self.page == page;
                    let response = ui.add(
                        egui::Button::new(
                            RichText::new(format!("{number}    {label}"))
                                .size(15.0)
                                .strong()
                                .color(if selected {
                                    theme::SIDEBAR
                                } else {
                                    theme::SIDEBAR_TEXT
                                }),
                        )
                        .fill(if selected {
                            theme::ACCENT_LIGHT
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(9.0)
                        .min_size(Vec2::new(
                            ui.available_width(),
                            if compact { 34.0 } else { 42.0 },
                        )),
                    );
                    if response.clicked() {
                        self.page = page;
                    }
                    ui.add_space(if compact { 1.0 } else { 3.0 });
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(concat!("OFFLINE READY   /   v", env!("CARGO_PKG_VERSION")))
                            .size(10.0)
                            .color(theme::SIDEBAR_MUTED),
                    );
                    ui.add_space(12.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Settings & backup").color(theme::SIDEBAR_TEXT),
                            )
                            .fill(Color32::from_rgb(35, 48, 73))
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(ui.available_width(), 36.0)),
                        )
                        .clicked()
                    {
                        self.page = Page::Settings;
                    }
                    ui.add_space(if compact { 12.0 } else { 20.0 });
                    ui.label(
                        RichText::new(format!(
                            "{} / {} words today",
                            self.stats.reviewed_today, self.preferences.daily_goal
                        ))
                        .size(13.0)
                        .color(theme::SIDEBAR_TEXT),
                    );
                    ui.add(
                        egui::ProgressBar::new(
                            (self.stats.reviewed_today as f32 / self.preferences.daily_goal as f32)
                                .min(1.0),
                        )
                        .fill(theme::ACCENT_LIGHT)
                        .desired_width(ui.available_width()),
                    );
                    ui.label(
                        RichText::new("DAILY GOAL")
                            .size(10.0)
                            .color(theme::SIDEBAR_MUTED),
                    );
                });
            });
    }

    fn today_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "YOUR DAILY SPACE",
            "Small steps, stronger English.",
            &chrono::Local::now().format("%A, %d %B").to_string(),
        );
        ui.add_space(18.0);
        surface().fill(theme::ACCENT_SOFT).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    eyebrow(ui, "MAKE IT STICK");
                    ui.label(
                        RichText::new(if self.stats.due_words > 0 {
                            format!("{} words are ready for another look.", self.stats.due_words)
                        } else if self.cards.is_empty() {
                            "Your next chapter starts with one word.".into()
                        } else {
                            "You’re up to date. Keep your momentum.".into()
                        })
                        .size(23.0)
                        .strong()
                        .color(theme::INK),
                    );
                    muted(
                        ui,
                        "Recall first, reveal second. A short session is enough to begin.",
                    );
                });
            });
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if primary_button(
                    ui,
                    if self.cards.is_empty() {
                        "Explore word collections  →"
                    } else {
                        "Start a focused session  →"
                    },
                )
                .clicked()
                {
                    if self.cards.is_empty() {
                        self.page = Page::Collections;
                    } else {
                        self.start_session(
                            if self.stats.due_words > 0 {
                                StudyOrder::Due
                            } else {
                                StudyOrder::Smart
                            },
                            ctx,
                        );
                    }
                }
                badge(
                    ui,
                    &format!("{} day streak", self.stats.streak),
                    theme::ACCENT_DARK,
                );
            });
        });
        ui.add_space(18.0);
        ui.columns(3, |columns| {
            metric(
                &mut columns[0],
                "TODAY",
                &format!(
                    "{} / {}",
                    self.stats.reviewed_today, self.preferences.daily_goal
                ),
                "different words recalled",
            );
            metric(
                &mut columns[1],
                "READY TO REVIEW",
                &self.stats.due_words.to_string(),
                "scheduled + new words",
            );
            metric(
                &mut columns[2],
                "YOUR VOCABULARY",
                &self.stats.total_words.to_string(),
                "words you chose to learn",
            );
        });
        ui.add_space(24.0);
        let index = chrono::Local::now()
            .format("%j")
            .to_string()
            .parse::<usize>()
            .unwrap_or(1);
        if !self.phrases.is_empty() {
            let phrase = self.phrases[index % self.phrases.len()].clone();
            surface().show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "EXPRESSION OF THE DAY");
                ui.label(
                    RichText::new(&phrase.phrase)
                        .size(24.0)
                        .strong()
                        .color(theme::INK),
                );
                ui.horizontal_wrapped(|ui| {
                    badge(ui, &phrase.category, theme::ACCENT_DARK);
                    if !phrase.register.is_empty() {
                        badge(ui, &phrase.register, theme::MUTED);
                    }
                });
                ui.label(RichText::new(phrase.primary_definition()).color(theme::INK));
                if let Some(example) = phrase.examples().next() {
                    example_block(ui, example);
                }
                if text_button(ui, "Explore in Phrasebook  →").clicked() {
                    self.page = Page::Phrasebook;
                    self.phrase_search = phrase.phrase;
                    self.phrase_category = "All".into();
                    self.phrase_status = "All".into();
                    self.phrase_topic = "All topics".into();
                    self.expanded.insert(phrase.id);
                }
            });
        }
        ui.add_space(18.0);
        activity_chart(ui, &self.stats);
    }

    fn deck_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "YOUR WORDS",
            "My deck",
            &format!(
                "Add a word. Its English definition and {} meaning arrive automatically.",
                language::name(self.language())
            ),
        );
        ui.add_space(16.0);
        surface().show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 155.0).max(170.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.new_word)
                        .id(egui::Id::new("word-add"))
                        .hint_text("Type an English word…")
                        .desired_width(width)
                        .margin(Vec2::new(12.0, 10.0)),
                );
                if self.focus_add {
                    response.request_focus();
                    self.focus_add = false;
                }
                let enter =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if ui.add_enabled(!self.busy, primary("+  Add word")).clicked() || enter {
                    self.add_word(ctx, false);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });
        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .id(egui::Id::new("deck-search"))
                    .hint_text("Search words, meanings, notes…")
                    .desired_width(250.0),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            egui::ComboBox::from_id_salt("deck-sort")
                .selected_text(match self.deck_sort {
                    DeckSort::Newest => "Newest first",
                    DeckSort::Alphabetical => "A → Z",
                    DeckSort::Weakest => "Needs practice first",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.deck_sort, DeckSort::Newest, "Newest first");
                    ui.selectable_value(&mut self.deck_sort, DeckSort::Alphabetical, "A → Z");
                    ui.selectable_value(
                        &mut self.deck_sort,
                        DeckSort::Weakest,
                        "Needs practice first",
                    );
                });
            egui::ComboBox::from_id_salt("deck-part")
                .selected_text(&self.part_filter)
                .show_ui(ui, |ui| {
                    for part in ["All word classes", "noun", "verb", "adjective", "adverb"] {
                        ui.selectable_value(&mut self.part_filter, part.into(), part);
                    }
                });
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for (filter, label) in [
                (DeckFilter::All, "All"),
                (DeckFilter::Due, "Due now"),
                (DeckFilter::New, "New"),
                (DeckFilter::Favorites, "Favorites"),
                (DeckFilter::Mastered, "Mastered"),
            ] {
                if choice_chip(ui, self.deck_filter == filter, label) {
                    self.deck_filter = filter;
                }
            }
        });
        ui.add_space(14.0);
        let search = self.search.trim().to_lowercase();
        let now = chrono::Utc::now();
        let mut visible: Vec<&WordCard> = self
            .cards
            .iter()
            .filter(|card| {
                (match self.deck_filter {
                    DeckFilter::All => true,
                    DeckFilter::Due => card.is_due(now),
                    DeckFilter::New => card.review_count == 0,
                    DeckFilter::Favorites => card.favorite,
                    DeckFilter::Mastered => card.mastery >= 4,
                }) && (self.part_filter == "All word classes"
                    || card.parts_of_speech.contains(&self.part_filter))
                    && (search.is_empty()
                        || format!(
                            "{} {} {} {}",
                            card.word,
                            card.translation(self.language()),
                            card.primary_definition(),
                            card.notes
                        )
                        .to_lowercase()
                        .contains(&search))
            })
            .collect();
        match self.deck_sort {
            DeckSort::Newest => {}
            DeckSort::Alphabetical => visible.sort_by(|a, b| a.word.cmp(&b.word)),
            DeckSort::Weakest => visible.sort_by_key(|card| (card.mastery, card.review_count)),
        }
        let ids: Vec<i64> = visible.iter().map(|card| card.id).collect();
        muted(ui, &format!("{} of {} words", ids.len(), self.cards.len()));
        if ids.is_empty() {
            empty_state(
                ui,
                if self.cards.is_empty() {
                    "A deck that grows with you."
                } else {
                    "No matching words."
                },
                if self.cards.is_empty() {
                    "Start with a word above or choose a ready-made collection."
                } else {
                    "Try another search or filter."
                },
            );
            if self.cards.is_empty() && primary_button(ui, "Browse collections  →").clicked() {
                self.page = Page::Collections;
            }
            return;
        }
        let width = ui.available_width();
        let columns: usize = if width > 1000.0 {
            3
        } else if width > 580.0 {
            2
        } else {
            1
        };
        let card_width = (width - (columns - 1) as f32 * 12.0) / columns as f32;
        for row in ids.chunks(columns) {
            ui.horizontal_top(|ui| {
                for id in row {
                    ui.allocate_ui_with_layout(
                        Vec2::new(card_width, 235.0),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.set_width(card_width);
                            self.deck_card(ui, *id, ctx);
                        },
                    );
                }
            });
            ui.add_space(4.0);
        }
    }

    fn deck_card(&mut self, ui: &mut egui::Ui, id: i64, ctx: &egui::Context) {
        let Some(card) = self.cards.iter().find(|card| card.id == id).cloned() else {
            return;
        };
        let flipped = self.flipped.contains(&id);
        surface().inner_margin(Margin::same(18)).show(ui, |ui| {
            ui.set_min_height(192.0);
            let width = ui.available_width();
            ui.set_width(width);
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::Label::new(
                        RichText::new(&card.word)
                            .size(23.0)
                            .strong()
                            .color(theme::INK),
                    )
                    .sense(egui::Sense::click()),
                );
                if response.clicked() {
                    self.open_word(id);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if favorite_button(ui, card.favorite).clicked() {
                        self.flag(false, id, !card.favorite, ctx);
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                for part in card.parts_of_speech.iter().take(3) {
                    badge(ui, part, part_color(part));
                }
            });
            ui.add_space(6.0);
            let body_start = ui.cursor().min;
            let body = if flipped {
                if card.translation(self.language()).is_empty() {
                    "Meaning not available yet. Add one in Details.".into()
                } else {
                    card.translation(self.language()).to_owned()
                }
            } else {
                preview(card.primary_definition(), 120)
            };
            ui.label(
                RichText::new(body)
                    .size(if flipped { 20.0 } else { 15.0 })
                    .color(theme::INK),
            );
            let body_end = ui.cursor().top();
            let response = ui
                .interact(
                    egui::Rect::from_min_max(
                        body_start,
                        egui::pos2(ui.max_rect().right(), body_end),
                    ),
                    ui.make_persistent_id(("word-body", id)),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.open_word(id);
            }
            ui.add_space(6.0);
            ui.label(RichText::new(card.due_label()).size(11.0).color(
                if card.is_due(chrono::Utc::now()) {
                    theme::ACCENT_DARK
                } else {
                    theme::MUTED
                },
            ));
            mastery_dots(ui, card.mastery);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if text_button(
                    ui,
                    if flipped {
                        "Front · EN"
                    } else {
                        "Flip · Meaning"
                    },
                )
                .clicked()
                {
                    if flipped {
                        self.flipped.remove(&id);
                    } else {
                        self.flipped.insert(id);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if text_button(ui, "Details  ↗").clicked() {
                        self.open_word(id);
                    }
                });
            });
        });
    }

    fn collections_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "GO BEYOND THE BASICS",
            "Word collections",
            "36 carefully chosen B2+ words. Examples, common combinations, and usage notes included.",
        );
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            for collection in content::COLLECTIONS {
                if choice_chip(ui, self.collection == collection, collection) {
                    self.collection = collection.into();
                }
            }
        });
        ui.add_space(18.0);
        surface().fill(theme::ACCENT_SOFT).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.collection).size(22.0).strong().color(theme::INK));
                badge(ui, "12 words · works offline", theme::ACCENT_DARK);
            });
            muted(ui, "Each entry focuses on a useful sense. Refresh a word to explore its full dictionary entry.");
            ui.add_space(8.0);
            let entries: Vec<CuratedWord> = self.words.iter().filter(|word| word.collection == self.collection).cloned().collect();
            let existing: HashSet<String> = self.cards.iter().map(|card| card.word.clone()).collect();
            let remaining = entries.iter().filter(|entry| !existing.contains(&entry.word)).count();
            if ui.add_enabled(remaining > 0, primary(&format!("Add {remaining} new words to my deck  →"))).clicked() {
                for entry in entries.iter().filter(|entry| !existing.contains(&entry.word)) {
                    if let Err(error) = self.db.upsert_card(&entry.card()) { self.notify(error.to_string(), true, ctx); return; }
                }
                self.reload(); self.queue_translations(); self.notify(format!("{remaining} words added to your deck."), false, ctx);
            }
        });
        ui.add_space(18.0);
        let entries: Vec<CuratedWord> = self
            .words
            .iter()
            .filter(|word| word.collection == self.collection)
            .cloned()
            .collect();
        for entry in entries {
            surface().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&entry.word)
                            .size(22.0)
                            .strong()
                            .color(theme::INK),
                    );
                    badge(ui, &entry.part, part_color(&entry.part));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let existing = self.cards.iter().any(|card| card.word == entry.word);
                        if ui
                            .add_enabled(
                                !existing,
                                primary(if existing { "In your deck" } else { "+ Add" }),
                            )
                            .clicked()
                        {
                            match self.db.upsert_card(&entry.card()) {
                                Ok(_) => {
                                    self.reload();
                                    self.queue_translations();
                                    self.notify(
                                        format!("{} added to your deck.", entry.word),
                                        false,
                                        ctx,
                                    );
                                }
                                Err(error) => self.notify(error.to_string(), true, ctx),
                            }
                        }
                    });
                });
                ui.label(RichText::new(&entry.definition).color(theme::INK));
                example_block(ui, &entry.example);
                ui.horizontal_wrapped(|ui| {
                    for pair in &entry.collocations {
                        badge(ui, pair, theme::ACCENT_DARK);
                    }
                });
                muted(ui, &entry.usage);
            });
            ui.add_space(10.0);
        }
    }

    fn phrasebook_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "ENGLISH IN CONTEXT",
            "Phrasebook",
            "150 B2+ expressions, with examples and practical usage notes. Add your own as you grow.",
        );
        ui.add_space(16.0);
        surface().show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 180.0).max(160.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.new_phrase)
                        .id(egui::Id::new("phrase-add"))
                        .hint_text("Add an idiom, phrasal verb, or pattern…")
                        .desired_width(width)
                        .margin(Vec2::new(12.0, 10.0)),
                );
                if self.focus_add {
                    response.request_focus();
                    self.focus_add = false;
                }
                let enter =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if ui
                    .add_enabled(!self.phrase_busy, primary("+  Add expression"))
                    .clicked()
                    || enter
                {
                    self.add_phrase(ctx);
                }
                if self.phrase_busy {
                    ui.spinner();
                }
            });
            ui.horizontal_wrapped(|ui| {
                muted(
                    ui,
                    "Dictionary lookup is automatic. Custom patterns can also be written directly.",
                );
                if text_button(ui, "Write my own").clicked() {
                    self.edit_phrase(PhraseEntry {
                        phrase: self.new_phrase.trim().into(),
                        category: "Reusable patterns".into(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        ..Default::default()
                    });
                }
            });
        });
        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            for category in ["All", "Idioms", "Phrasal verbs", "Reusable patterns"] {
                let count = self
                    .phrases
                    .iter()
                    .filter(|phrase| category == "All" || phrase.category == category)
                    .count();
                if choice_chip(
                    ui,
                    self.phrase_category == category,
                    &format!("{category}  {count}"),
                ) {
                    self.phrase_category = category.into();
                }
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.phrase_search)
                    .id(egui::Id::new("phrase-search"))
                    .hint_text("Search expressions, topics, examples…")
                    .desired_width(265.0),
            );
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            egui::ComboBox::from_id_salt("phrase-status")
                .selected_text(&self.phrase_status)
                .show_ui(ui, |ui| {
                    for status in ["All", "Not yet known", "Known", "Favorites"] {
                        ui.selectable_value(&mut self.phrase_status, status.into(), status);
                    }
                });
            let topics: std::collections::BTreeSet<String> = self
                .phrases
                .iter()
                .filter(|phrase| !phrase.topic.is_empty())
                .map(|phrase| phrase.topic.clone())
                .collect();
            egui::ComboBox::from_id_salt("phrase-topic")
                .selected_text(&self.phrase_topic)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.phrase_topic, "All topics".into(), "All topics");
                    for topic in topics {
                        ui.selectable_value(&mut self.phrase_topic, topic.clone(), topic);
                    }
                });
            if ui
                .checkbox(
                    &mut self.preferences.hide_phrase_translations,
                    "Hide meanings",
                )
                .changed()
            {
                self.revealed_phrases.clear();
                self.persist_preferences(ctx);
            }
        });
        ui.add_space(12.0);
        let query = self.phrase_search.trim().to_lowercase();
        let ids: Vec<i64> = self
            .phrases
            .iter()
            .filter(|phrase| {
                (self.phrase_category == "All" || phrase.category == self.phrase_category)
                    && (match self.phrase_status.as_str() {
                        "Not yet known" => !phrase.known,
                        "Known" => phrase.known,
                        "Favorites" => phrase.favorite,
                        _ => true,
                    })
                    && (self.phrase_topic == "All topics" || phrase.topic == self.phrase_topic)
                    && (query.is_empty()
                        || format!(
                            "{} {} {} {} {} {}",
                            phrase.phrase,
                            phrase.translation(self.language()),
                            phrase.primary_definition(),
                            phrase.topic,
                            phrase.usage_note,
                            phrase.examples().collect::<Vec<_>>().join(" ")
                        )
                        .to_lowercase()
                        .contains(&query))
            })
            .map(|phrase| phrase.id)
            .collect();
        let known = self.phrases.iter().filter(|phrase| phrase.known).count();
        muted(
            ui,
            &format!(
                "{} shown · {} learned of {} expressions",
                ids.len(),
                known,
                self.phrases.len()
            ),
        );
        ui.add_space(8.0);
        if ids.is_empty() {
            empty_state(
                ui,
                "No matching expressions.",
                "Try another category, topic, or search.",
            );
        }
        for id in ids {
            self.phrase_row(ui, id, ctx);
            ui.add_space(8.0);
        }
    }

    fn phrase_row(&mut self, ui: &mut egui::Ui, id: i64, ctx: &egui::Context) {
        let Some(phrase) = self.phrases.iter().find(|phrase| phrase.id == id).cloned() else {
            return;
        };
        let expanded = self.expanded.contains(&id);
        surface()
            .inner_margin(Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 235.0).max(120.0);
                    let title = ui
                        .allocate_ui_with_layout(
                            Vec2::new(width, 25.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(width);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&phrase.phrase)
                                            .size(20.0)
                                            .strong()
                                            .color(theme::INK),
                                    )
                                    .truncate()
                                    .halign(Align::LEFT)
                                    .sense(egui::Sense::click()),
                                )
                            },
                        )
                        .inner;
                    if title.on_hover_text(&phrase.phrase).clicked() {
                        toggle(&mut self.expanded, id);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if text_button(ui, if expanded { "Close  −" } else { "Explore  +" })
                            .clicked()
                        {
                            toggle(&mut self.expanded, id);
                        }
                        if choice_chip(
                            ui,
                            phrase.known,
                            if phrase.known {
                                "✓ Known"
                            } else {
                                "Mark known"
                            },
                        ) {
                            if let Err(error) = self.db.set_known(id, !phrase.known) {
                                self.notify(error.to_string(), true, ctx);
                            } else {
                                self.reload();
                            }
                        }
                        if favorite_button(ui, phrase.favorite).clicked() {
                            self.flag(true, id, !phrase.favorite, ctx);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    badge(
                        ui,
                        &phrase.category,
                        phrase_category_color(&phrase.category),
                    );
                    if !phrase.register.is_empty() {
                        badge(ui, &phrase.register, theme::MUTED);
                    }
                    if !phrase.topic.is_empty() {
                        muted_small(ui, &phrase.topic);
                    }
                });
                let show_translation = !self.preferences.hide_phrase_translations
                    || self.revealed_phrases.contains(&id);
                if show_translation && !phrase.translation(self.language()).is_empty() {
                    ui.label(
                        RichText::new(phrase.translation(self.language()))
                            .size(16.0)
                            .color(theme::ACCENT_DARK),
                    );
                } else if self.preferences.hide_phrase_translations
                    && text_button(ui, "Reveal meaning").clicked()
                {
                    self.revealed_phrases.insert(id);
                }
                if expanded {
                    ui.label(RichText::new(phrase.primary_definition()).color(theme::INK));
                } else {
                    ui.add(
                        egui::Label::new(
                            RichText::new(phrase.primary_definition()).color(theme::MUTED),
                        )
                        .truncate(),
                    );
                }
                if expanded {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);
                    for definition in &phrase.definitions {
                        if definition.text != phrase.primary_definition() {
                            ui.label(RichText::new(&definition.text).color(theme::INK));
                        }
                        if let Some(example) = &definition.example {
                            example_block(ui, example);
                        }
                    }
                    if !phrase.usage_note.is_empty() {
                        eyebrow(ui, "USE IT NATURALLY");
                        ui.label(RichText::new(&phrase.usage_note).color(theme::INK));
                    }
                    ui.add_space(10.0);
                    eyebrow(ui, "MAKE IT YOURS");
                    let mut personal = self
                        .personal_examples
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| phrase.personal_example.clone());
                    ui.add(
                        egui::TextEdit::multiline(&mut personal)
                            .desired_width(f32::INFINITY)
                            .desired_rows(2)
                            .hint_text(
                                "Write one sentence about your own life using this expression…",
                            ),
                    );
                    self.personal_examples.insert(id, personal.clone());
                    ui.horizontal_wrapped(|ui| {
                        if text_button(ui, "Save my sentence").clicked() {
                            let mut entry = phrase.clone();
                            entry.personal_example = personal.trim().into();
                            match self.db.update_phrase(&entry) {
                                Ok(()) => {
                                    self.reload();
                                    self.notify("Your sentence is saved.", false, ctx);
                                }
                                Err(error) => self.notify(error.to_string(), true, ctx),
                            }
                        }
                        if text_button(ui, "Edit expression").clicked() {
                            self.edit_phrase(phrase.clone());
                        }
                        if text_button(ui, "Remove").clicked() {
                            self.archive(true, id, ctx);
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("YouGlish ↗", youglish_url(&phrase.phrase));
                        ui.hyperlink_to("WordHippo examples ↗", wordhippo_url(&phrase.phrase));
                        ui.hyperlink_to("Cambridge ↗", cambridge_url(&phrase.phrase));
                    });
                }
            });
    }

    fn phrase_editor(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.phrase_editor.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new(if draft.id == 0 {
            "Add your own expression"
        } else {
            "Edit expression"
        })
        .id(egui::Id::new("phrase-editor"))
        .fade_in(false)
        .fade_out(false)
        .open(&mut open)
        .collapsible(false)
        .default_width(590.0)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(560.0)
                .show(ui, |ui| {
                    if !self.phrase_edit_error.is_empty() {
                        ui.label(RichText::new(&self.phrase_edit_error).color(theme::RED));
                        ui.add_space(8.0);
                    }
                    field_label(ui, "Expression");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.phrase).desired_width(f32::INFINITY),
                    );
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("edit-category")
                            .selected_text(&draft.category)
                            .show_ui(ui, |ui| {
                                for category in ["Idioms", "Phrasal verbs", "Reusable patterns"] {
                                    ui.selectable_value(
                                        &mut draft.category,
                                        category.into(),
                                        category,
                                    );
                                }
                            });
                        egui::ComboBox::from_id_salt("edit-register")
                            .selected_text(if draft.register.is_empty() {
                                "Register (optional)"
                            } else {
                                &draft.register
                            })
                            .show_ui(ui, |ui| {
                                for register in ["Neutral", "Formal", "Informal"] {
                                    ui.selectable_value(
                                        &mut draft.register,
                                        register.into(),
                                        register,
                                    );
                                }
                            });
                    });
                    field_label(ui, &format!("{} meaning", language::name(self.language())));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.phrase_edit_translation)
                            .desired_width(f32::INFINITY),
                    );
                    field_label(ui, "English definition");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.phrase_edit_definition)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    field_label(ui, "English example");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.phrase_edit_example)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    field_label(ui, "Usage note / grammar");
                    ui.add(
                        egui::TextEdit::multiline(&mut draft.usage_note)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    field_label(ui, "Topic (optional)");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.topic)
                            .desired_width(f32::INFINITY)
                            .hint_text("e.g. Communication"),
                    );
                    ui.add_space(12.0);
                    save = primary_button(ui, "Save expression").clicked();
                });
        });
        if save {
            draft.phrase = draft
                .phrase
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let duplicate = self.phrases.iter().any(|phrase| {
                phrase.id != draft.id && phrase.phrase.eq_ignore_ascii_case(&draft.phrase)
            });
            if draft.phrase.split_whitespace().count() < 2 {
                self.phrase_edit_error = "Enter an expression of at least two words.".into();
            } else if duplicate {
                self.phrase_edit_error = "That expression is already in your phrasebook.".into();
            } else if self.phrase_edit_definition.trim().is_empty() {
                self.phrase_edit_error = "Add a short English definition.".into();
            } else {
                draft.translations.insert(
                    self.language().to_owned(),
                    self.phrase_edit_translation.trim().to_owned(),
                );
                let definition = Definition {
                    part_of_speech: "phrase".into(),
                    text: self.phrase_edit_definition.trim().into(),
                    example: (!self.phrase_edit_example.trim().is_empty())
                        .then(|| self.phrase_edit_example.trim().into()),
                };
                if draft.definitions.is_empty() {
                    draft.definitions.push(definition);
                } else {
                    draft.definitions[0] = definition;
                }
                if draft.id == 0 {
                    if self.save_phrase(draft.clone(), ctx) {
                        open = false;
                    }
                } else {
                    match self.db.update_phrase(&draft) {
                        Ok(()) => {
                            self.reload();
                            self.notify("Expression updated.", false, ctx);
                            open = false;
                        }
                        Err(error) => self.phrase_edit_error = error.to_string(),
                    }
                }
            }
        }
        if open {
            self.phrase_editor = Some(draft);
        }
    }

    fn start_session(&mut self, order: StudyOrder, ctx: &egui::Context) {
        let now = chrono::Utc::now();
        let mut cards: Vec<&WordCard> = self
            .cards
            .iter()
            .filter(|card| {
                (match order {
                    StudyOrder::Due => card.is_due(now),
                    StudyOrder::Favorites => card.favorite,
                    _ => true,
                }) && !card.translation(self.language()).trim().is_empty()
            })
            .collect();
        cards.shuffle(&mut rand::rng());
        if order == StudyOrder::Due {
            cards.sort_by_key(|card| (card.review_count == 0, card.due_at.clone()));
        }
        if order == StudyOrder::Smart {
            cards.sort_by_key(|card| (!card.is_due(now), card.mastery, card.review_count));
        }
        let ids: Vec<i64> = cards
            .iter()
            .take(self.study_size)
            .map(|card| card.id)
            .collect();
        if ids.is_empty() {
            self.notify(
                "No translated words match this session yet. Choose another mix, wait for translations, or add a meaning in Details.",
                false,
                ctx,
            );
            self.page = Page::Study;
            return;
        }
        self.study_order = order;
        self.session = Session {
            ids,
            started: Some(Instant::now()),
            ..Default::default()
        };
        self.page = Page::Study;
    }

    fn grade(&mut self, rating: ReviewRating, ctx: &egui::Context) {
        let Some(id) = self.session.ids.get(self.session.index).copied() else {
            return;
        };
        if let Err(error) = self.db.record_review(id, rating) {
            self.notify(format!("Review was not saved: {error}"), true, ctx);
            return;
        }
        if self.session.first_pass.insert(id) && rating.is_correct() {
            self.session.good += 1;
        }
        self.session.ratings += 1;
        if rating == ReviewRating::Again && self.session.retried.insert(id) {
            self.session.ids.push(id);
        }
        self.session.index += 1;
        self.session.revealed = false;
        if self.session.index >= self.session.ids.len() {
            self.finish_session();
        }
        self.reload();
    }

    fn finish_session(&mut self) {
        self.session.elapsed_seconds = self
            .session
            .started
            .map_or(0, |started| started.elapsed().as_secs());
        self.session.complete = true;
    }

    fn study_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "RECALL, DON'T JUST RECOGNIZE",
            "Study",
            "Space reveals the answer. Keys 1–4 rate your recall.",
        );
        ui.add_space(18.0);
        if self.session.complete {
            surface().fill(theme::ACCENT_SOFT).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "PROGRESS SAVED");
                ui.label(
                    RichText::new(if self.session.index >= self.session.ids.len() {
                        "One session closer."
                    } else {
                        "Good place to pause."
                    })
                    .size(30.0)
                    .strong()
                    .color(theme::INK),
                );
                muted(
                    ui,
                    "Your next review dates are set. Return when you're ready.",
                );
                ui.add_space(18.0);
                ui.columns(3, |columns| {
                    metric(
                        &mut columns[0],
                        "WORDS PRACTICED",
                        &self.session.first_pass.len().to_string(),
                        "unique words this session",
                    );
                    let recall = if self.session.first_pass.is_empty() {
                        0
                    } else {
                        100 * self.session.good / self.session.first_pass.len()
                    };
                    metric(
                        &mut columns[1],
                        "FIRST RECALL",
                        &format!("{recall}%"),
                        "before any retry",
                    );
                    metric(
                        &mut columns[2],
                        "RETRIED",
                        &self.session.retried.len().to_string(),
                        "difficult words revisited",
                    );
                });
                ui.add_space(14.0);
                let seconds = self.session.elapsed_seconds;
                muted_small(
                    ui,
                    &format!(
                        "{} ratings saved · {} min {} sec",
                        self.session.ratings,
                        seconds / 60,
                        seconds % 60
                    ),
                );
                ui.horizontal_wrapped(|ui| {
                    if primary_button(ui, "Set up another session  →").clicked() {
                        self.session = Session::default();
                    }
                    if text_button(ui, "Back to Today").clicked() {
                        self.session = Session::default();
                        self.page = Page::Today;
                    }
                });
            });
            return;
        }
        if self.session.ids.is_empty() {
            surface().show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "SESSION SIZE");
                ui.horizontal_wrapped(|ui| {
                    for size in [5, 10, 20] {
                        if choice_chip(ui, self.study_size == size, &format!("{size} words")) {
                            self.study_size = size;
                        }
                    }
                    muted(ui, "Custom");
                    ui.add(egui::DragValue::new(&mut self.custom_study_size).range(1..=200));
                    if text_button(ui, "Apply").clicked() {
                        self.study_size = self.custom_study_size;
                    }
                });
                ui.add_space(20.0);
                eyebrow(ui, "CHOOSE YOUR MIX");
                ui.horizontal_wrapped(|ui| {
                    for (order, label) in [
                        (StudyOrder::Due, "Due reviews"),
                        (StudyOrder::Smart, "Smart focus"),
                        (StudyOrder::Random, "Random"),
                        (StudyOrder::Favorites, "Favorites"),
                    ] {
                        if choice_chip(ui, self.study_order == order, label) {
                            self.study_order = order;
                        }
                    }
                });
                muted(
                    ui,
                    match self.study_order {
                        StudyOrder::Due => {
                            "Only scheduled and new words. Overdue reviews come first."
                        }
                        StudyOrder::Smart => {
                            "Difficult and due words first, with the rest of your deck available."
                        }
                        StudyOrder::Random => "A fresh shuffle of your whole deck.",
                        StudyOrder::Favorites => "Only words you have starred.",
                    },
                );
                ui.add_space(20.0);
                eyebrow(ui, "RECALL DIRECTION");
                ui.horizontal_wrapped(|ui| {
                    if choice_chip(
                        ui,
                        self.study_direction == StudyDirection::EnglishToTranslation,
                        &format!("English → {}", language::name(self.language())),
                    ) {
                        self.study_direction = StudyDirection::EnglishToTranslation;
                    }
                    if choice_chip(
                        ui,
                        self.study_direction == StudyDirection::TranslationToEnglish,
                        &format!("{} → English", language::name(self.language())),
                    ) {
                        self.study_direction = StudyDirection::TranslationToEnglish;
                    }
                });
                ui.add_space(20.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.cards.is_empty(),
                            primary(&format!("Start {}-word session  →", self.study_size)),
                        )
                        .clicked()
                    {
                        self.start_session(self.study_order, ctx);
                    }
                    muted(
                        ui,
                        &format!(
                            "{} ready to review · {} in your deck",
                            self.stats.due_words,
                            self.cards.len()
                        ),
                    );
                });
            });
            ui.add_space(18.0);
            surface().show(ui, |ui| {
                eyebrow(ui, "HOW YOUR NEXT REVIEW IS CHOSEN");
                ui.label("Again → 10 minutes · Hard → a shorter interval · Good → a longer interval · Easy → longer still");
                muted(ui, "An 'Again' word returns once later in the session. Each rating is saved immediately.");
            });
            return;
        }
        let Some(id) = self.session.ids.get(self.session.index).copied() else {
            self.session = Session::default();
            return;
        };
        let Some(card) = self.cards.iter().find(|card| card.id == id).cloned() else {
            self.session.ids.remove(self.session.index);
            return;
        };
        ui.horizontal_wrapped(|ui| {
            badge(
                ui,
                &format!(
                    "STEP {} / {}",
                    self.session.index + 1,
                    self.session.ids.len()
                ),
                theme::ACCENT_DARK,
            );
            if self.session.first_pass.contains(&id) {
                badge(ui, "RETRY", theme::VERB);
            }
            ui.add(
                egui::ProgressBar::new(self.session.index as f32 / self.session.ids.len() as f32)
                    .desired_width(200.0)
                    .fill(theme::ACCENT),
            );
            if text_button(ui, "End session").clicked() {
                self.finish_session();
            }
        });
        ui.add_space(18.0);
        let was_revealed = self.session.revealed;
        // Session pages have no editable fields. Button focus must not swallow
        // recall shortcuts; editors and detail windows suspend them.
        let shortcuts = self.detail_id.is_none() && self.phrase_editor.is_none();
        if shortcuts
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.session.revealed = true;
        }
        surface().inner_margin(Margin::same(30)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(300.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                eyebrow(
                    ui,
                    if self.study_direction == StudyDirection::EnglishToTranslation {
                        "ENGLISH → RECALL THE MEANING"
                    } else {
                        "MEANING → RECALL THE ENGLISH"
                    },
                );
                ui.add_space(22.0);
                let prompt = if self.study_direction == StudyDirection::EnglishToTranslation {
                    &card.word
                } else {
                    card.translation(self.language())
                };
                ui.label(RichText::new(prompt).size(42.0).strong().color(theme::INK));
                if self.study_direction == StudyDirection::EnglishToTranslation
                    && let Some(phonetic) = &card.phonetic
                {
                    muted(ui, phonetic);
                }
                ui.add_space(24.0);
                if self.session.revealed {
                    let answer = if self.study_direction == StudyDirection::EnglishToTranslation {
                        card.translation(self.language())
                    } else {
                        &card.word
                    };
                    ui.label(
                        RichText::new(if answer.is_empty() {
                            "Translation missing — edit it in word details."
                        } else {
                            answer
                        })
                        .size(27.0)
                        .strong()
                        .color(theme::ACCENT_DARK),
                    );
                    ui.add_space(12.0);
                    ui.label(RichText::new(card.primary_definition()).color(theme::INK));
                    if let Some(example) = card
                        .definitions
                        .iter()
                        .find_map(|definition| definition.example.as_deref())
                    {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!("“{example}”"))
                                .italics()
                                .color(theme::MUTED),
                        );
                    }
                    if !card.collocations.is_empty() {
                        ui.add_space(8.0);
                        muted(ui, &card.collocations.join("  ·  "));
                    }
                } else {
                    if primary_button(ui, "Reveal answer  ·  Space").clicked() {
                        self.session.revealed = true;
                    }
                    ui.add_space(12.0);
                    muted_small(ui, "Try to say the meaning before you reveal it.");
                }
            });
        });
        if self.session.revealed {
            ui.add_space(18.0);
            let mut grade = None;
            ui.columns(4, |columns| {
                for (index, rating) in ReviewRating::ALL.into_iter().enumerate() {
                    let color = match rating {
                        ReviewRating::Again => theme::RED,
                        ReviewRating::Hard => theme::VERB,
                        ReviewRating::Good => theme::GREEN,
                        ReviewRating::Easy => theme::ACCENT_DARK,
                    };
                    let interval = rating.next_interval(card.interval_days);
                    let delay = if rating == ReviewRating::Again {
                        "10 min".into()
                    } else {
                        format!("{interval} day{}", if interval == 1 { "" } else { "s" })
                    };
                    if columns[index]
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "{}  {}\n{delay}",
                                    index + 1,
                                    rating.label()
                                ))
                                .color(Color32::WHITE),
                            )
                            .fill(color)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(columns[index].available_width(), 60.0)),
                        )
                        .clicked()
                    {
                        grade = Some(rating);
                    }
                }
            });
            if shortcuts && was_revealed {
                for (key, rating) in [
                    (egui::Key::Num1, ReviewRating::Again),
                    (egui::Key::Num2, ReviewRating::Hard),
                    (egui::Key::Num3, ReviewRating::Good),
                    (egui::Key::Num4, ReviewRating::Easy),
                ] {
                    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key)) {
                        grade = Some(rating);
                    }
                }
            }
            if let Some(rating) = grade {
                self.grade(rating, ctx);
            }
        }
    }

    fn progress_page(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "THE LONG VIEW",
            "Your progress",
            "Consistency adds up. These numbers reflect the work you've actually done.",
        );
        ui.add_space(18.0);
        ui.columns(3, |columns| {
            metric(
                &mut columns[0],
                "TOTAL REVIEWS",
                &self.stats.total_reviews.to_string(),
                "every saved rating",
            );
            metric(
                &mut columns[1],
                "CURRENT STREAK",
                &format!("{} days", self.stats.streak),
                "today or yesterday included",
            );
            metric(
                &mut columns[2],
                "MASTERED",
                &format!("{} / {}", self.stats.mastered_words, self.cards.len()),
                "at mastery level 4–5",
            );
        });
        ui.add_space(18.0);
        activity_chart(ui, &self.stats);
        ui.add_space(18.0);
        surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "VOCABULARY AT A GLANCE");
            for (label, low, high, color) in [
                ("Getting started", 0, 0, theme::MUTED),
                ("Building recall", 1, 3, theme::ACCENT),
                ("Feeling confident", 4, 5, theme::GREEN),
            ] {
                let count = self
                    .cards
                    .iter()
                    .filter(|card| (low..=high).contains(&card.mastery))
                    .count();
                ui.horizontal(|ui| {
                    ui.add_sized(
                        Vec2::new(150.0, 24.0),
                        egui::Label::new(RichText::new(label).color(theme::INK)),
                    );
                    ui.add(
                        egui::ProgressBar::new(count as f32 / self.cards.len().max(1) as f32)
                            .desired_width((ui.available_width() - 50.0).max(100.0))
                            .fill(color),
                    );
                    ui.label(RichText::new(count.to_string()).strong().color(theme::INK));
                });
            }
            ui.add_space(16.0);
            eyebrow(ui, "PHRASEBOOK");
            let known = self.phrases.iter().filter(|phrase| phrase.known).count();
            ui.label(format!(
                "{known} of {} expressions marked known · {} personal example sentences",
                self.phrases.len(),
                self.phrases
                    .iter()
                    .filter(|phrase| !phrase.personal_example.trim().is_empty())
                    .count()
            ));
            ui.add(
                egui::ProgressBar::new(known as f32 / self.phrases.len().max(1) as f32)
                    .fill(theme::ACCENT),
            );
        });
        ui.add_space(18.0);
        surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "GIVE THESE ANOTHER LOOK");
            let words: Vec<(i64, String)> = self
                .cards
                .iter()
                .filter(|card| card.review_count > 0 && card.mastery <= 2)
                .take(8)
                .map(|card| (card.id, card.word.clone()))
                .collect();
            if words.is_empty() {
                muted(ui, "Difficult words will appear here after a few sessions.");
            }
            ui.horizontal_wrapped(|ui| {
                for (id, word) in words {
                    if text_button(ui, &word).clicked() {
                        self.open_word(id);
                    }
                }
            });
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        page_title(
            ui,
            "MAKE IT YOURS",
            "Settings & backup",
            "Your deck, notes, examples, and learning history stay on this computer.",
        );
        ui.add_space(18.0);
        surface().show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "PRACTICE PREFERENCES");
            ui.horizontal_wrapped(|ui| {
                ui.label("Meaning language");
                language_picker(ui, &mut self.language_choice, "settings-language");
                if self.preferences.translation_language.as_deref() != Some(&self.language_choice)
                    && primary_button(ui, "Apply language").clicked()
                {
                    self.choose_language(ctx);
                }
            });
            muted_small(ui, "Saved translations are kept for each language. New translations need internet access.");
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Daily goal");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.preferences.daily_goal)
                            .range(1..=200)
                            .suffix(" words"),
                    )
                    .changed()
                {
                    self.persist_preferences(ctx);
                }
                muted_small(ui, "Repeated attempts at the same word count once per day.");
            });
            if ui
                .checkbox(
                    &mut self.preferences.hide_phrase_translations,
                    "Hide meanings in Phrasebook until revealed",
                )
                .changed()
            {
                self.revealed_phrases.clear();
                self.persist_preferences(ctx);
            }
        });
        ui.add_space(18.0);
        surface().show(ui, |ui| {
            ui.set_width(ui.available_width()); eyebrow(ui, "TAKE YOUR PROGRESS WITH YOU");
            ui.label(RichText::new("One file. Your entire learning library.").size(21.0).strong().color(theme::INK));
            muted(ui, "A backup includes words, phrases, notes, favorites, examples, and review dates.");
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if primary_button(ui, "Create backup").clicked() {
                    match self.db.export_backup() { Ok(path) => { self.last_backup = Some(path); self.notify("Backup created.", false, ctx); },
                        Err(error) => self.notify(format!("Backup failed: {error}"), true, ctx) }
                }
                if text_button(ui, "Open data folder").clicked()
                    && let Some(parent) = self.db.path().parent()
                        && let Err(error) = std::process::Command::new("explorer.exe").arg(parent).spawn() { self.notify(error.to_string(), true, ctx); }
            });
            if let Some(path) = &self.last_backup {
                ui.add_space(8.0); ui.label(RichText::new(path.display().to_string()).small().color(theme::MUTED));
                if text_button(ui, "Copy backup path").clicked() { ctx.copy_text(path.display().to_string()); }
            }
            ui.add_space(18.0); ui.separator(); ui.add_space(10.0);
            field_label(ui, "Restore a LexiCards backup");
            muted_small(ui, "Drop a JSON backup onto this window, or paste its path below.");
            ui.horizontal(|ui| {
                let width = (ui.available_width()-150.0).max(160.0);
                if ui.add(egui::TextEdit::singleline(&mut self.backup_path).hint_text("C:\\…\\LexiCards-backup.json").desired_width(width)).changed() { self.backup_preview = None; }
                if text_button(ui, "Preview backup").clicked() { self.preview_backup(ctx); }
            });
            if let Some(backup) = &self.backup_preview {
                ui.add_space(8.0);
                ui.label(format!("{} words · {} phrases · {} reviews", backup.cards.len(), backup.phrases.len(), backup.reviews.len()));
                muted(ui, "Matching entries will use the backup's data and progress. Other entries stay in your library. A safety backup is created first.");
                if primary_button(ui, "Restore this backup").clicked() {
                    let backup = self.backup_preview.take().unwrap();
                    match self.db.export_backup().and_then(|safety| { self.db.restore_backup(&backup)?; Ok(safety) }) {
                        Ok(safety) => { self.last_backup = Some(safety); self.preferences = self.db.preferences().unwrap_or_default();
                            self.language_choice = self.preferences.translation_language.clone().unwrap_or_else(|| "tr".into());
                            self.translation_receiver = None; self.translation_error = None;
                            theme::apply(ctx, self.preferences.translation_language.as_deref());
                            self.session = Session::default(); self.detail_id = None; self.phrase_editor = None;
                            self.personal_examples.clear(); self.expanded.clear(); self.flipped.clear(); self.reload();
                            self.queue_translations();
                            self.notify("Backup restored. A copy of your previous library was saved first.", false, ctx); },
                        Err(error) => self.notify(format!("Restore failed: {error}"), true, ctx),
                    }
                }
            }
        });
        ui.add_space(18.0);
        surface().show(ui, |ui| {
            ui.set_width(ui.available_width()); eyebrow(ui, "KEYBOARD SHORTCUTS");
            ui.label("Ctrl + F  Search    ·    Ctrl + N  Add a word    ·    Space  Reveal    ·    1–4  Rate recall");
            ui.add_space(10.0); eyebrow(ui, "ABOUT THE CONTENT");
            muted(ui, "Starter examples and usage notes are written for upper-intermediate and advanced practice. Level focus is editorial, not a certified CEFR assessment.");
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("Wiktionary", "https://en.wiktionary.org"); ui.hyperlink_to("Free Dictionary API", "https://dictionaryapi.dev");
                ui.hyperlink_to("Datamuse", "https://www.datamuse.com/api/"); ui.hyperlink_to("MyMemory", "https://mymemory.translated.net");
            });
        });
    }

    fn preview_backup(&mut self, ctx: &egui::Context) {
        let path = PathBuf::from(self.backup_path.trim().trim_matches('"'));
        match Database::read_backup(&path) {
            Ok(backup) => self.backup_preview = Some(backup),
            Err(error) => {
                self.backup_preview = None;
                self.notify(format!("Could not read backup: {error}"), true, ctx);
            }
        }
    }

    fn detail_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.detail_id else {
            return;
        };
        let Some(card) = self.cards.iter().find(|card| card.id == id).cloned() else {
            self.detail_id = None;
            return;
        };
        let mut open = true;
        let mut refresh = false;
        let mut remove = false;
        egui::Window::new("Word details")
            .id(egui::Id::new("word-details"))
            .fade_in(false)
            .fade_out(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::same(20)),
            )
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(730.0, 660.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&card.word)
                                .size(36.0)
                                .strong()
                                .color(theme::INK),
                        );
                        if let Some(phonetic) = &card.phonetic {
                            muted(ui, phonetic);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if favorite_button(ui, card.favorite).clicked() {
                                self.flag(false, id, !card.favorite, ctx);
                            }
                            if ui
                                .add_enabled(!self.busy, secondary("Refresh word data"))
                                .clicked()
                            {
                                refresh = true;
                            }
                        });
                    });
                    muted_small(
                        ui,
                        &format!(
                            "{}  ·  {} reviews  ·  {:.0}% recall  ·  mastery {}/5",
                            card.due_label(),
                            card.review_count,
                            card.accuracy() * 100.0,
                            card.mastery
                        ),
                    );
                    ui.add_space(18.0);
                    field_label(ui, &format!("{} meaning", language::name(self.language())));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.detail_translation)
                            .desired_width(f32::INFINITY)
                            .hint_text("Write a meaning in your selected language"),
                    );
                    ui.add_space(16.0);
                    eyebrow(ui, "MEANING & USE");
                    let mut parts = card.parts_of_speech.clone();
                    for definition in &card.definitions {
                        if !parts.contains(&definition.part_of_speech) {
                            parts.push(definition.part_of_speech.clone());
                        }
                    }
                    for (index, part) in parts.iter().enumerate() {
                        surface().inner_margin(Margin::same(14)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                badge(ui, part, part_color(part));
                                if index == 0 {
                                    muted_small(ui, "PRIMARY SENSE");
                                }
                            });
                            for definition in card
                                .definitions
                                .iter()
                                .filter(|definition| definition.part_of_speech == *part)
                            {
                                ui.label(RichText::new(&definition.text).color(theme::INK));
                                if let Some(example) = &definition.example {
                                    example_block(ui, example);
                                }
                            }
                        });
                        ui.add_space(8.0);
                    }
                    if !card.collocations.is_empty() {
                        ui.add_space(8.0);
                        eyebrow(ui, "COMMON COMBINATIONS");
                        ui.horizontal_wrapped(|ui| {
                            for pair in &card.collocations {
                                badge(ui, pair, theme::ACCENT_DARK);
                            }
                        });
                    }
                    if !card.synonyms.is_empty() {
                        ui.add_space(12.0);
                        eyebrow(ui, "RELATED WORDS");
                        muted(ui, &card.synonyms.join(" · "));
                    }
                    ui.add_space(16.0);
                    field_label(ui, "My notes & memory hooks");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.detail_notes)
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .hint_text(
                                "A useful distinction, a sentence of your own, or a memory hook…",
                            ),
                    );
                    if primary_button(ui, "Save meaning & notes").clicked() {
                        match self.db.update_word_notes(
                            id,
                            self.language(),
                            self.detail_translation.trim(),
                            self.detail_notes.trim(),
                        ) {
                            Ok(()) => {
                                self.reload();
                                self.notify("Meaning and notes saved.", false, ctx);
                            }
                            Err(error) => self.notify(error.to_string(), true, ctx),
                        }
                    }
                    ui.add_space(16.0);
                    eyebrow(ui, "EXPLORE THE FULL ENTRY");
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("Cambridge ↗", cambridge_url(&card.word));
                        ui.hyperlink_to("Oxford Learner’s ↗", oxford_url(&card.word));
                        ui.hyperlink_to("Merriam-Webster ↗", merriam_webster_url(&card.word));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("Hear it on YouGlish ↗", youglish_url(&card.word));
                        ui.hyperlink_to("WordHippo examples ↗", wordhippo_url(&card.word));
                    });
                    ui.add_space(18.0);
                    if text_button(ui, "Remove from deck").clicked() {
                        remove = true;
                    }
                });
            });
        if remove {
            self.archive(false, id, ctx);
            open = false;
        }
        if refresh {
            self.new_word = card.word;
            self.add_word(ctx, true);
            open = false;
        }
        if !open {
            self.detail_id = None;
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        if let Some((message, error, started)) = self.toast.clone() {
            let elapsed = ctx.input(|input| input.time) - started;
            if elapsed > 6.0 {
                self.toast = None;
                return;
            }
            egui::Area::new(egui::Id::new("toast"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-20.0, -20.0))
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(if error { theme::RED } else { theme::INK })
                        .corner_radius(10.0)
                        .inner_margin(Margin::symmetric(18, 12))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            ui.label(RichText::new(message).color(Color32::WHITE));
                        });
                });
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.detail_id.is_some() || self.phrase_editor.is_some() {
            return;
        }
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::F) {
                if self.page != Page::Phrasebook {
                    self.page = Page::Deck;
                }
                self.focus_search = true;
            }
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::N) {
                if self.page != Page::Phrasebook {
                    self.page = Page::Deck;
                }
                self.focus_add = true;
            }
        });
    }

    fn capture(&mut self, ctx: &egui::Context) {
        if let Some(path) = &self.launch.capture {
            let screenshot = ctx.input(|input| {
                input.events.iter().find_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = screenshot {
                let bytes: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                if let Err(error) = image::save_buffer(
                    path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("Screenshot failed: {error}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if self.frames == 8 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

impl LexiCardsApp {
    fn render(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        self.poll(ctx);
        if self.preferences.translation_language.is_none() {
            self.language_setup(ctx);
            self.toast(ctx);
            self.capture(ctx);
            return;
        }
        self.shortcuts(ctx);
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .and_then(|file| file.path.clone())
        }) {
            self.backup_path = path.display().to_string();
            self.page = Page::Settings;
            self.preview_backup(ctx);
        }
        self.sidebar(ctx);
        self.translation_status(ctx);
        if let Some((phrase, id)) = self.undo {
            egui::TopBottomPanel::bottom("undo-bar")
                .frame(
                    Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(Margin::symmetric(24, 10)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        muted(ui, "Item removed. Your history is kept.");
                        if text_button(ui, "Undo removal").clicked() {
                            match self.db.set_archived(phrase, id, false) {
                                Ok(()) => {
                                    self.undo = None;
                                    self.reload();
                                    self.notify("Item restored.", false, ctx);
                                }
                                Err(error) => self.notify(error.to_string(), true, ctx),
                            }
                        }
                        if text_button(ui, "Dismiss").clicked() {
                            self.undo = None;
                        }
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(28)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("page-scroll", self.page as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.page {
                            Page::Today => self.today_page(ui, ctx),
                            Page::Deck => self.deck_page(ui, ctx),
                            Page::Phrasebook => self.phrasebook_page(ui, ctx),
                            Page::Collections => self.collections_page(ui, ctx),
                            Page::Study => self.study_page(ui, ctx),
                            Page::Progress => self.progress_page(ui),
                            Page::Settings => self.settings_page(ui, ctx),
                        }
                        ui.add_space(16.0);
                    });
            });
        self.detail_window(ctx);
        self.phrase_editor(ctx);
        self.toast(ctx);
        self.capture(ctx);
    }
}

impl eframe::App for LexiCardsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }
}

fn surface() -> Frame {
    Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(14.0)
        .inner_margin(Margin::same(20))
}

fn language_picker(ui: &mut egui::Ui, selected: &mut String, id: &str) {
    egui::ComboBox::from_id_salt(id)
        .width(220.0)
        .height(280.0)
        .selected_text(language::name(selected))
        .show_ui(ui, |ui| {
            for (code, name) in language::LANGUAGES {
                ui.selectable_value(selected, (*code).to_owned(), *name);
            }
        });
}

fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
        .fill(theme::ACCENT_DARK)
        .stroke(Stroke::NONE)
        .corner_radius(8.0)
        .min_size(Vec2::new(100.0, 38.0))
}

fn secondary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::INK))
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(8.0)
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(primary(label))
}
fn text_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(13.0)
                .strong()
                .color(theme::ACCENT_DARK),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(0.0, 28.0)),
    )
}
fn favorite_button(ui: &mut egui::Ui, favorite: bool) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(if favorite { "★" } else { "☆" })
                .size(20.0)
                .color(if favorite {
                    theme::ACCENT_DARK
                } else {
                    theme::MUTED
                }),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::splat(28.0)),
    )
    .on_hover_text(if favorite {
        "Remove from favorites"
    } else {
        "Add to favorites"
    })
}
fn choice_chip(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).strong().color(if selected {
            Color32::WHITE
        } else {
            theme::INK
        }))
        .fill(if selected {
            theme::ACCENT_DARK
        } else {
            theme::PANEL
        })
        .stroke(if selected {
            Stroke::NONE
        } else {
            Stroke::new(1.0, theme::BORDER)
        })
        .corner_radius(8.0)
        .min_size(Vec2::new(0.0, 34.0)),
    )
    .clicked()
}
fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .strong()
            .color(theme::ACCENT_DARK),
    );
}
fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED));
}
fn muted_small(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(theme::MUTED));
}
fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(13.0).strong().color(theme::INK));
}
fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .corner_radius(5.0)
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).strong().color(color));
        });
}
fn page_title(ui: &mut egui::Ui, eyebrow_text: &str, title: &str, subtitle: &str) {
    eyebrow(ui, eyebrow_text);
    ui.add_space(5.0);
    ui.label(
        RichText::new(title)
            .size(32.0)
            .family(egui::FontFamily::Name("heading".into()))
            .color(theme::INK),
    );
    ui.add_space(3.0);
    muted(ui, subtitle);
}
fn metric(ui: &mut egui::Ui, label: &str, value: &str, note: &str) {
    surface().inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, label);
        ui.label(
            RichText::new(value)
                .size(30.0)
                .family(egui::FontFamily::Name("heading".into()))
                .color(theme::INK),
        );
        muted_small(ui, note);
    });
}
fn example_block(ui: &mut egui::Ui, example: &str) {
    Frame::new()
        .fill(theme::BG)
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("“{example}”"))
                    .italics()
                    .color(theme::INK),
            );
        });
}
fn empty_state(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.add_space(30.0);
    surface().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add_space(16.0);
        ui.label(RichText::new(title).size(24.0).strong().color(theme::INK));
        muted(ui, text);
        ui.add_space(16.0);
    });
    ui.add_space(12.0);
}
fn mastery_dots(ui: &mut egui::Ui, mastery: u8) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(68.0, 8.0), egui::Sense::hover());
    for index in 0..5 {
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 4.0 + index as f32 * 14.0, rect.center().y),
            3.0,
            if index < mastery {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        );
    }
}
fn activity_chart(ui: &mut egui::Ui, stats: &Stats) {
    surface().show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "YOUR LAST 14 DAYS");
        muted_small(
            ui,
            "Different words reviewed each day. Hover a bar for the exact count.",
        );
        ui.add_space(12.0);
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 86.0), egui::Sense::hover());
        let max = stats
            .activity
            .iter()
            .map(|day| day.count)
            .max()
            .unwrap_or(0)
            .max(5) as f32;
        let step = width / stats.activity.len().max(1) as f32;
        for (index, day) in stats.activity.iter().enumerate() {
            let height = (day.count as f32 / max * 58.0).max(3.0);
            let x = rect.left() + index as f32 * step;
            let bar = egui::Rect::from_min_max(
                egui::pos2(x + 4.0, rect.top() + 60.0 - height),
                egui::pos2(x + step - 4.0, rect.top() + 60.0),
            );
            ui.painter().rect_filled(
                bar,
                4.0,
                if day.count > 0 {
                    theme::ACCENT
                } else {
                    theme::BORDER
                },
            );
            let label = chrono::NaiveDate::parse_from_str(&day.date, "%Y-%m-%d")
                .map(|date| date.format("%d").to_string())
                .unwrap_or_default();
            ui.painter().text(
                egui::pos2(x + step / 2.0, rect.top() + 74.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                theme::MUTED,
            );
            ui.interact(
                egui::Rect::from_min_max(
                    egui::pos2(x, rect.top()),
                    egui::pos2(x + step, rect.bottom()),
                ),
                ui.make_persistent_id(("activity", index)),
                egui::Sense::hover(),
            )
            .on_hover_text(format!("{} · {} words", day.date, day.count));
        }
    });
}
fn preview(value: &str, limit: usize) -> String {
    if value.chars().count() > limit {
        format!("{}…", value.chars().take(limit).collect::<String>())
    } else {
        value.into()
    }
}
fn toggle(set: &mut HashSet<i64>, id: i64) {
    if !set.remove(&id) {
        set.insert(id);
    }
}
fn part_color(part: &str) -> Color32 {
    match part {
        "noun" | "proper noun" => theme::NOUN,
        "verb" => theme::VERB,
        "adjective" => theme::ADJECTIVE,
        _ => theme::ACCENT_DARK,
    }
}
fn phrase_category_color(category: &str) -> Color32 {
    match category {
        "Phrasal verbs" => theme::VERB,
        "Idioms" => theme::ADJECTIVE,
        _ => theme::NOUN,
    }
}
fn dictionary_slug(word: &str) -> String {
    urlencoding::encode(
        &resource_query(word)
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-"),
    )
    .into_owned()
}
fn cambridge_url(word: &str) -> String {
    format!(
        "https://dictionary.cambridge.org/dictionary/english/{}",
        dictionary_slug(word)
    )
}
fn oxford_url(word: &str) -> String {
    format!(
        "https://www.oxfordlearnersdictionaries.com/definition/english/{}",
        dictionary_slug(word)
    )
}
fn merriam_webster_url(word: &str) -> String {
    format!(
        "https://www.merriam-webster.com/dictionary/{}",
        urlencoding::encode(&resource_query(word))
    )
}
fn youglish_url(word: &str) -> String {
    format!(
        "https://youglish.com/pronounce/{}/english",
        urlencoding::encode(&resource_query(word))
    )
}
fn wordhippo_url(word: &str) -> String {
    let slug = resource_query(word)
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    format!(
        "https://www.wordhippo.com/what-is/sentences-with-the-word/{}.html",
        urlencoding::encode(&slug)
    )
}

fn resource_query(expression: &str) -> String {
    expression
        .split("...")
        .next()
        .unwrap_or(expression)
        .split('…')
        .next()
        .unwrap_or(expression)
        .trim()
        .trim_end_matches([',', '.', ';'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        LaunchOptions, LexiCardsApp, Page, StudyOrder, cambridge_url, merriam_webster_url,
        oxford_url, wordhippo_url, youglish_url,
    };
    use eframe::egui;

    fn fixture() -> (egui::Context, LexiCardsApp, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "lexicards-ui-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let db = crate::db::Database::open(&path).unwrap();
        db.save_preferences(&crate::model::Preferences {
            translation_language: Some("tr".into()),
            ..Default::default()
        })
        .unwrap();
        db.upsert_card(&crate::content::words()[0].card()).unwrap();
        let ctx = egui::Context::default();
        let app = LexiCardsApp::new(&ctx, db, LaunchOptions::default());
        (ctx, app, path)
    }

    fn cleanup(app: LexiCardsApp, path: &std::path::Path) {
        drop(app);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    fn draw(
        ctx: &egui::Context,
        app: &mut LexiCardsApp,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 860.0),
            )),
            time: Some(app.frames as f64 / 60.0),
            events,
            focused: true,
            ..Default::default()
        };
        ctx.run(input, |ctx| app.render(ctx))
    }

    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, label))
                }
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .unwrap_or_else(|| panic!("UI label not found: {label}"))
    }

    fn click(ctx: &egui::Context, app: &mut LexiCardsApp, pos: egui::Pos2) {
        draw(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        draw(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    fn key(ctx: &egui::Context, app: &mut LexiCardsApp, key: egui::Key) {
        draw(
            ctx,
            app,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        draw(
            ctx,
            app,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    #[test]
    fn flip_reveals_only_translation_without_opening_details() {
        let (ctx, mut app, path) = fixture();
        app.page = Page::Deck;
        draw(&ctx, &mut app, vec![]);
        let output = draw(&ctx, &mut app, vec![]);
        let pos = text_center(&output, "Flip · Meaning");
        let id = app.cards[0].id;
        click(&ctx, &mut app, pos);
        assert!(app.flipped.contains(&id));
        assert!(app.detail_id.is_none());
        let output = draw(&ctx, &mut app, vec![]);
        text_center(&output, app.cards[0].translation("tr"));
        let pos = text_center(&output, "Front · EN");
        click(&ctx, &mut app, pos);
        assert!(!app.flipped.contains(&id));
        assert!(app.detail_id.is_none());
        cleanup(app, &path);
    }

    #[test]
    fn recall_shortcuts_retry_once_and_count_each_word_once_per_day() {
        let (ctx, mut app, path) = fixture();
        app.start_session(StudyOrder::Smart, &ctx);
        draw(&ctx, &mut app, vec![]);
        key(&ctx, &mut app, egui::Key::Num3);
        assert_eq!(
            app.stats.total_reviews, 0,
            "rating before reveal must be ignored"
        );
        key(&ctx, &mut app, egui::Key::Space);
        assert!(app.session.revealed);
        key(&ctx, &mut app, egui::Key::Num1);
        assert_eq!(app.stats.total_reviews, 1);
        assert_eq!(
            app.session.ids.len(),
            2,
            "a lapse returns once at the end of the session"
        );
        key(&ctx, &mut app, egui::Key::Space);
        key(&ctx, &mut app, egui::Key::Num3);
        assert!(app.session.complete);
        assert_eq!(app.stats.total_reviews, 2);
        assert_eq!(app.stats.reviewed_today, 1);
        assert_eq!(
            app.session.good, 0,
            "a successful retry must not inflate first-recall accuracy"
        );
        assert_eq!(app.cards[0].interval_days, 1);
        cleanup(app, &path);
    }

    #[test]
    fn first_launch_requires_a_language_and_switching_uses_its_cached_meanings() {
        let (ctx, mut app, path) = fixture();
        app.preferences.translation_language = None;
        draw(&ctx, &mut app, vec![]);
        let output = draw(&ctx, &mut app, vec![]);
        let pos = text_center(&output, "Start learning →");
        click(&ctx, &mut app, pos);
        assert_eq!(
            app.db
                .preferences()
                .unwrap()
                .translation_language
                .as_deref(),
            Some("tr")
        );
        assert!(app.translation_receiver.is_none());

        // Prime the cache so this UI test does not call external services.
        for phrase in &app.phrases {
            app.db
                .save_translation(true, phrase.id, "fr", "sens français")
                .unwrap();
        }
        let id = app.cards[0].id;
        app.db
            .save_translation(false, id, "fr", "nuance française")
            .unwrap();
        app.reload();
        app.language_choice = "fr".into();
        app.choose_language(&ctx);
        assert_eq!(
            app.db
                .preferences()
                .unwrap()
                .translation_language
                .as_deref(),
            Some("fr")
        );
        assert!(app.translation_receiver.is_none());
        app.page = Page::Deck;
        app.flipped.insert(id);
        draw(&ctx, &mut app, vec![]);
        let output = draw(&ctx, &mut app, vec![]);
        text_center(&output, "nuance française");
        assert!(!app.cards[0].translation("tr").is_empty());
        cleanup(app, &path);
    }

    #[test]
    fn study_only_uses_words_translated_into_the_selected_language() {
        let (ctx, mut app, path) = fixture();
        app.preferences.translation_language = Some("fr".into());
        app.start_session(StudyOrder::Smart, &ctx);
        assert!(app.session.ids.is_empty());
        app.db
            .save_translation(false, app.cards[0].id, "fr", "nuance française")
            .unwrap();
        app.reload();
        app.start_session(StudyOrder::Smart, &ctx);
        assert_eq!(app.session.ids.len(), 1);
        app.study_direction = super::StudyDirection::TranslationToEnglish;
        draw(&ctx, &mut app, vec![]);
        let output = draw(&ctx, &mut app, vec![]);
        text_center(&output, "nuance française");
        cleanup(app, &path);
    }

    #[test]
    fn resource_links_target_the_selected_word() {
        assert_eq!(
            wordhippo_url("take off"),
            "https://www.wordhippo.com/what-is/sentences-with-the-word/take_off.html"
        );
        assert_eq!(
            cambridge_url("ice cream"),
            "https://dictionary.cambridge.org/dictionary/english/ice-cream"
        );
        assert_eq!(
            oxford_url("example"),
            "https://www.oxfordlearnersdictionaries.com/definition/english/example"
        );
        assert_eq!(
            merriam_webster_url("example"),
            "https://www.merriam-webster.com/dictionary/example"
        );
        assert_eq!(
            youglish_url("It remains to be seen whether..."),
            "https://youglish.com/pronounce/It%20remains%20to%20be%20seen%20whether/english"
        );
        assert_eq!(
            wordhippo_url("With regard to..."),
            "https://www.wordhippo.com/what-is/sentences-with-the-word/with_regard_to.html"
        );
    }
}
