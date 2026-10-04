use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const BG: Color32 = Color32::from_rgb(245, 247, 251);
pub const PANEL: Color32 = Color32::from_rgb(255, 255, 255);
pub const INK: Color32 = Color32::from_rgb(27, 36, 55);
pub const MUTED: Color32 = Color32::from_rgb(82, 96, 116);
pub const ACCENT: Color32 = Color32::from_rgb(55, 82, 189);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(39, 60, 143);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(196, 210, 252);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(233, 238, 253);
pub const SIDEBAR: Color32 = Color32::from_rgb(23, 33, 53);
pub const SIDEBAR_TEXT: Color32 = Color32::from_rgb(244, 247, 246);
pub const SIDEBAR_MUTED: Color32 = Color32::from_rgb(168, 182, 208);
pub const BORDER: Color32 = Color32::from_rgb(218, 226, 238);
pub const NOUN: Color32 = Color32::from_rgb(38, 91, 151);
pub const VERB: Color32 = Color32::from_rgb(181, 91, 32);
pub const ADJECTIVE: Color32 = Color32::from_rgb(112, 68, 153);
pub const GREEN: Color32 = Color32::from_rgb(27, 128, 87);
pub const RED: Color32 = Color32::from_rgb(181, 51, 65);

pub fn apply(ctx: &egui::Context, language: Option<&str>) {
    ctx.set_theme(egui::Theme::Light);
    let mut fonts = egui::FontDefinitions::default();
    for (name, path) in [
        ("Segoe UI", r"C:\Windows\Fonts\segoeui.ttf"),
        ("Segoe UI Semibold", r"C:\Windows\Fonts\seguisb.ttf"),
        ("Segoe UI Symbols", r"C:\Windows\Fonts\seguisym.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
        }
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if fonts.font_data.contains_key("Segoe UI Symbols") {
        proportional.insert(0, "Segoe UI Symbols".into());
    }
    if fonts.font_data.contains_key("Segoe UI") {
        proportional.insert(0, "Segoe UI".into());
    }
    let international_font = match language {
        Some("zh-CN" | "zh-TW") => Some(r"C:\Windows\Fonts\msyh.ttc"),
        Some("ja") => Some(r"C:\Windows\Fonts\YuGothM.ttc"),
        Some("ko") => Some(r"C:\Windows\Fonts\malgun.ttf"),
        Some("hi") => Some(r"C:\Windows\Fonts\Nirmala.ttf"),
        _ => None,
    };
    if let Some(path) = international_font
        && let Ok(bytes) = std::fs::read(path)
    {
        fonts.font_data.insert(
            "Translation font".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        proportional.push("Translation font".into());
    }
    let mut heading_fonts = proportional.clone();
    if fonts.font_data.contains_key("Segoe UI Semibold") {
        heading_fonts.insert(0, "Segoe UI Semibold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name("heading".into()), heading_fonts);
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Name("heading".into())),
        ),
        (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(14.0, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style.visuals = Visuals::light();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = PANEL;
    style.visuals.window_corner_radius = CornerRadius::same(16);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(9);
    style.visuals.widgets.inactive.bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.25_f32, INK);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.bg_fill = BG;
    style.visuals.widgets.noninteractive.weak_bg_fill = BG;
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.25_f32, ACCENT_DARK);
    style.visuals.widgets.hovered.weak_bg_fill = ACCENT_SOFT;
    style.visuals.widgets.active.bg_fill = ACCENT_SOFT;
    style.visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, INK);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = Stroke::new(1.5_f32, Color32::WHITE);
    style.visuals.hyperlink_color = ACCENT_DARK;
    style.visuals.faint_bg_color = BG;
    style.visuals.extreme_bg_color = PANEL;
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}
