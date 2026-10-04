pub const LANGUAGES: &[(&str, &str)] = &[
    ("ar", "Arabic"),
    ("bg", "Bulgarian"),
    ("zh-CN", "Chinese (Simplified)"),
    ("zh-TW", "Chinese (Traditional)"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("de", "German"),
    ("el", "Greek"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("es", "Spanish"),
    ("sv", "Swedish"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
];

pub fn name(code: &str) -> &str {
    LANGUAGES
        .iter()
        .find(|(language, _)| *language == code)
        .map_or(code, |(_, name)| name)
}

pub fn is_supported(code: &str) -> bool {
    LANGUAGES.iter().any(|(language, _)| *language == code)
}
