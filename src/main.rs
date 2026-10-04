#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod content;
mod db;
mod language;
mod model;
mod theme;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use eframe::egui;

fn main() -> Result<()> {
    let mut data_directory: Option<std::path::PathBuf> = None;
    let mut launch = app::LaunchOptions::default();
    let mut demo = false;
    let mut translation_language: Option<String> = None;
    let mut size = [1280.0, 860.0];
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--data-dir" => {
                data_directory = Some(args.next().context("--data-dir requires a path")?.into())
            }
            "--page" => launch.page = Some(args.next().context("--page requires a page name")?),
            "--capture" => {
                launch.capture = Some(args.next().context("--capture requires a PNG path")?.into())
            }
            "--demo" => demo = true,
            "--language" => {
                let code = args.next().context("--language requires a language code")?;
                if !language::is_supported(&code) {
                    anyhow::bail!("Unsupported language code: {code}");
                }
                translation_language = Some(code);
            }
            "--size" => {
                let dimensions = args.next().context("--size requires WIDTHxHEIGHT")?;
                let (width, height) = dimensions.split_once('x').context("Use WIDTHxHEIGHT")?;
                size = [
                    width.parse::<f32>()?.max(880.0),
                    height.parse::<f32>()?.max(600.0),
                ];
            }
            _ => anyhow::bail!("Unknown argument: {argument}"),
        }
    }
    if (demo || launch.capture.is_some()) && data_directory.is_none() {
        anyhow::bail!(
            "Demo and capture modes require --data-dir so your real library is never changed."
        );
    }
    let data_directory = if let Some(path) = data_directory {
        path
    } else {
        ProjectDirs::from("com", "LexiCards", "LexiCards")
            .context("Could not resolve the application data directory")?
            .data_local_dir()
            .to_owned()
    };
    let database_path = data_directory.join("lexicards.db");
    let database = db::Database::open(&database_path)?;
    if translation_language.is_some() || demo {
        let mut preferences = database.preferences()?;
        if translation_language.is_some() {
            preferences.translation_language = translation_language;
        } else if preferences.translation_language.is_none() {
            preferences.translation_language = Some("tr".into());
        }
        database.save_preferences(&preferences)?;
    }
    if demo {
        for (index, word) in content::words().iter().take(15).enumerate() {
            let id = database.upsert_card(&word.card())?;
            if index < 6
                && database
                    .load_cards()?
                    .iter()
                    .any(|card| card.id == id && card.review_count == 0)
            {
                database.record_review(id, model::ReviewRating::ALL[index % 4])?;
            }
            if index % 4 == 0 {
                database.set_favorite(false, id, true)?;
            }
        }
        for phrase in database.load_phrases()?.iter().take(3) {
            database.set_favorite(true, phrase.id, true)?;
            database.set_known(phrase.id, true)?;
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LexiCards")
            .with_inner_size(size)
            .with_min_inner_size([880.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LexiCards",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::LexiCardsApp::new(
                &cc.egui_ctx,
                database,
                launch,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
