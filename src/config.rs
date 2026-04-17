use std::collections::HashMap;
use std::path::PathBuf;

// ── Structs ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ThemeConfig {
    pub background:      String,
    pub accent:          String,
    pub text:            String,
    pub text_muted:      String,
    pub bar_count_color: String,
    pub font_family:     String,
    pub font_size:       u32,
}

#[derive(Clone)]
pub struct CalEntry {
    pub name:  String,
    pub color: String,
    pub icon:  String,
}

#[derive(Clone)]
pub struct Config {
    pub theme:       ThemeConfig,
    pub default_cal: CalEntry,
    pub calendars:   Vec<CalEntry>,
}

// ── Defaults ──────────────────────────────────────────────────────────────────

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            background:      "#1a2125".into(),
            accent:          "#8FBC8F".into(),
            text:            "#c9d1d9".into(),
            text_muted:      "#6a7a71".into(),
            bar_count_color: "#f38ba8".into(),
            font_family:     "CaskaydiaMono Nerd Font, monospace".into(),
            font_size:       13,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme:       ThemeConfig::default(),
            default_cal: CalEntry {
                name:  String::new(),
                color: "#cdd6f4".into(),
                icon:  "\u{f0486}".into(),
            },
            calendars: Vec::new(),
        }
    }
}

// ── Preset table ──────────────────────────────────────────────────────────────

fn theme_preset(name: &str) -> ThemeConfig {
    match name {
        "catppuccin-mocha" => ThemeConfig {
            background:      "#1e1e2e".into(),
            accent:          "#a6e3a1".into(),
            text:            "#cdd6f4".into(),
            text_muted:      "#6c7086".into(),
            bar_count_color: "#f38ba8".into(),
            ..ThemeConfig::default()
        },
        "catppuccin-latte" => ThemeConfig {
            background:      "#eff1f5".into(),
            accent:          "#40a02b".into(),
            text:            "#4c4f69".into(),
            text_muted:      "#9ca0b0".into(),
            bar_count_color: "#d20f39".into(),
            ..ThemeConfig::default()
        },
        "tokyonight-storm" => ThemeConfig {
            background:      "#24283b".into(),
            accent:          "#7aa2f7".into(),
            text:            "#c0caf5".into(),
            text_muted:      "#565f89".into(),
            bar_count_color: "#f7768e".into(),
            ..ThemeConfig::default()
        },
        "gruvbox" => ThemeConfig {
            background:      "#282828".into(),
            accent:          "#8ec07c".into(),
            text:            "#ebdbb2".into(),
            text_muted:      "#928374".into(),
            bar_count_color: "#fb4934".into(),
            ..ThemeConfig::default()
        },
        "dracula" => ThemeConfig {
            background:      "#282a36".into(),
            accent:          "#50fa7b".into(),
            text:            "#f8f8f2".into(),
            text_muted:      "#6272a4".into(),
            bar_count_color: "#ff5555".into(),
            ..ThemeConfig::default()
        },
        _ => ThemeConfig::default(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn hex_to_rgba(hex: &str, alpha: f32) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&h[0..2], 16),
            u8::from_str_radix(&h[2..4], 16),
            u8::from_str_radix(&h[4..6], 16),
        ) {
            return format!("rgba({}, {}, {}, {:.2})", r, g, b, alpha);
        }
    }
    hex.to_string()
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("waycal").join("config")
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn load() -> Config {
    let path = config_path();
    if !path.exists() {
        let _ = write_default(&path);
        return Config::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(_)   => Config::default(),
    }
}

// ── INI parser ────────────────────────────────────────────────────────────────

enum Section {
    None,
    Theme,
    Default,
    Calendar(usize),
}

fn parse(text: &str) -> Config {
    let mut theme_raw: HashMap<String, String> = HashMap::new();
    let mut default_color = String::new();
    let mut default_icon  = String::new();
    let mut calendars: Vec<CalEntry> = Vec::new();
    let mut section = Section::None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }

        if line.starts_with('[') && line.ends_with(']') {
            let inner = &line[1..line.len() - 1];
            let lower = inner.to_ascii_lowercase();
            if lower == "theme" {
                section = Section::Theme;
            } else if lower == "default" {
                section = Section::Default;
            } else if lower.starts_with("calendar ") {
                let name = inner[9..].trim().trim_matches('"').trim_matches('\'').to_string();
                calendars.push(CalEntry { name, color: String::new(), icon: String::new() });
                section = Section::Calendar(calendars.len() - 1);
            } else {
                section = Section::None;
            }
            continue;
        }

        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_ascii_lowercase();
            let val = line[eq + 1..].trim().to_string();
            match &mut section {
                Section::Theme => { theme_raw.insert(key, val); }
                Section::Default => match key.as_str() {
                    "color" => default_color = val,
                    "icon"  => default_icon  = val,
                    _ => {}
                },
                Section::Calendar(idx) => {
                    if let Some(cal) = calendars.get_mut(*idx) {
                        match key.as_str() {
                            "color" => cal.color = val,
                            "icon"  => cal.icon  = val,
                            _ => {}
                        }
                    }
                }
                Section::None => {}
            }
        }
    }

    // Start from preset (or built-in default), then overlay explicit keys
    let preset_name = theme_raw.get("preset").map(|s| s.trim()).unwrap_or("default");
    let mut theme = theme_preset(preset_name);
    macro_rules! apply {
        ($key:literal, $field:expr) => {
            if let Some(v) = theme_raw.get($key) { $field = v.clone(); }
        };
    }
    apply!("background",      theme.background);
    apply!("accent",           theme.accent);
    apply!("text",             theme.text);
    apply!("text_muted",       theme.text_muted);
    apply!("bar_count_color",  theme.bar_count_color);
    apply!("font_family",      theme.font_family);
    if let Some(v) = theme_raw.get("font_size") {
        if let Ok(n) = v.trim().parse::<u32>() { theme.font_size = n; }
    }

    let default_cal = CalEntry {
        name:  String::new(),
        color: if default_color.is_empty() { "#cdd6f4".into() } else { default_color },
        icon:  if default_icon.is_empty()  { "\u{f0486}".into() } else { default_icon },
    };

    Config { theme, default_cal, calendars }
}

// ── Write default config on first run ─────────────────────────────────────────

fn write_default(path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DEFAULT_CONFIG)
}

const DEFAULT_CONFIG: &str = "\
# waycal configuration  ~/.config/waycal/config
#
# [theme] preset choices:
#   default | catppuccin-mocha | catppuccin-latte | tokyonight-storm | gruvbox | dracula
# Individual color keys below override the chosen preset.

[theme]
preset = default
# background      = #1a2125
# accent          = #8FBC8F
# text            = #c9d1d9
# text_muted      = #6a7a71
# bar_count_color = #f38ba8
# font_family     = CaskaydiaMono Nerd Font, monospace
# font_size       = 13

# Fallback color/icon for calendars not listed below
[default]
color = #cdd6f4
icon  = \u{f0486}

# Add a [calendar \"Name\"] section per Google Calendar.
# The name must match exactly what Google Calendar shows.
#
# [calendar \"Personal\"]
# color = #dd7878
# icon  = \u{f06eb}
#
# [calendar \"Work\"]
# color = #89b4fa
# icon  = \u{f0237}
";
