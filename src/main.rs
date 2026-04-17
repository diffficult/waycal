mod config;
mod gcal;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Datelike, Local, NaiveDate};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use gcal::MonthCache;

const APP_ID: &str = "com.forrestknight.waycal";

fn build_css(t: &config::ThemeConfig) -> String {
    let bg_rgba       = config::hex_to_rgba(&t.background, 0.96);
    let accent_sel    = config::hex_to_rgba(&t.accent, 0.22);
    let accent_border = config::hex_to_rgba(&t.accent, 0.18);
    format!(
        "window.waycal {{ background: transparent; }}\
         .waycal-root {{\
             background-color: {bg};\
             border: 2px solid {accent};\
             border-radius: 0;\
             padding: 14px 18px;\
             color: {text};\
             font-family: {font};\
             font-size: {fsize}px;\
         }}\
         .waycal-root.rounded {{\
             background-color: {bg_rgba};\
             border: 2px solid transparent;\
             border-radius: 16px;\
         }}\
         .waycal-header {{ font-weight: bold; font-size: 15px; padding-bottom: 6px; }}\
         .waycal-weekday {{ color: {accent}; font-weight: bold; padding: 2px 6px; }}\
         .waycal-day {{ padding: 2px 4px; min-width: 26px; border-radius: 0; }}\
         .waycal-day.dim {{ opacity: 0.3; }}\
         .waycal-day-num {{ padding: 2px 5px; }}\
         .waycal-day.today > .waycal-day-num {{\
             background-color: {accent};\
             color: {bg};\
             border-radius: 0;\
             font-weight: bold;\
         }}\
         .waycal-root.rounded .waycal-day.today > .waycal-day-num {{ border-radius: 8px; }}\
         .waycal-day.selected > .waycal-day-num {{\
             background-color: {accent_sel};\
             border-radius: 0;\
         }}\
         .waycal-root.rounded .waycal-day.selected > .waycal-day-num {{ border-radius: 8px; }}\
         .waycal-day.today.selected > .waycal-day-num {{\
             background-color: {accent};\
             color: {bg};\
         }}\
         .waycal-dots-row {{ min-height: 7px; }}\
         .waycal-panel {{\
             min-width: 190px;\
             padding-left: 14px;\
             margin-left: 6px;\
             border-left: 1px solid {accent_border};\
         }}\
         .waycal-panel-header {{\
             font-weight: bold;\
             font-size: 13px;\
             padding-bottom: 8px;\
             color: {accent};\
         }}\
         .waycal-panel-empty {{ color: {muted}; font-size: 12px; }}\
         .waycal-event-row {{ padding: 2px 0; }}\
         .waycal-event-icon {{ min-width: 20px; }}\
         .waycal-event-time {{ color: {muted}; min-width: 46px; font-size: 12px; }}\
         .waycal-event-title {{ font-size: 12px; }}\
         .waycal-footer {{\
             color: {muted};\
             font-size: 10px;\
             padding-top: 8px;\
             margin-top: 6px;\
             border-top: 1px solid {accent_border};\
         }}",
        bg            = t.background,
        accent        = t.accent,
        text          = t.text,
        muted         = t.text_muted,
        font          = t.font_family,
        fsize         = t.font_size,
        bg_rgba       = bg_rgba,
        accent_sel    = accent_sel,
        accent_border = accent_border,
    )
}

// ── Anchor argument ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Anchor { Left, Center, Right }

impl Anchor {
    fn from_str(s: &str) -> Self {
        match s {
            "left"  => Anchor::Left,
            "right" => Anchor::Right,
            _       => Anchor::Center,
        }
    }
}

// ── View state ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct ViewDate { year: i32, month: u32 }

impl ViewDate {
    fn today() -> Self {
        let now = Local::now().date_naive();
        Self { year: now.year(), month: now.month() }
    }
    fn shift_month(self, delta: i32) -> Self {
        let total = self.year * 12 + (self.month as i32 - 1) + delta;
        Self { year: total.div_euclid(12), month: total.rem_euclid(12) as u32 + 1 }
    }
    fn shift_year(self, delta: i32) -> Self {
        Self { year: self.year + delta, month: self.month }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap()
        .signed_duration_since(NaiveDate::from_ymd_opt(y, m, 1).unwrap())
        .num_days() as u32
}

fn style_state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("waycal").join("style"))
}

fn load_rounded() -> bool {
    style_state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim() == "rounded")
        .unwrap_or(false)
}

fn save_rounded(rounded: bool) {
    if let Some(path) = style_state_path() {
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        let _ = std::fs::write(path, if rounded { "rounded" } else { "sharp" });
    }
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "",
    }
}

fn weekday_name(d: NaiveDate) -> &'static str {
    match d.weekday() {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

fn day_label_text(d: NaiveDate) -> String {
    format!("{}, {} {}", weekday_name(d), month_name(d.month()), d.day())
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> glib::ExitCode {
    let raw_args: Vec<String> = std::env::args().collect();
    let bar_output = raw_args.iter().any(|a| a == "--bar-output");
    let anchor = raw_args.windows(2)
        .find(|w| w[0] == "--anchor")
        .map(|w| Anchor::from_str(&w[1]))
        .unwrap_or(Anchor::Center);

    let cfg = config::load();

    if bar_output {
        gcal::bar_output(&cfg);
        return glib::ExitCode::SUCCESS;
    }

    let css = build_css(&cfg.theme);
    let app = gtk4::Application::builder().application_id(APP_ID).build();
    app.connect_startup(move |_| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(&css);
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(move |app| build_ui(app, anchor, cfg.clone()));
    // Pass empty args so GTK doesn't choke on our custom flags
    app.run_with_args(&[] as &[&str])
}

// ── UI builder ────────────────────────────────────────────────────────────────

fn build_ui(app: &gtk4::Application, anchor: Anchor, config: config::Config) {
    let config = std::rc::Rc::new(config);
    let window = gtk4::ApplicationWindow::new(app);
    window.set_decorated(false);
    window.set_resizable(false);
    window.add_css_class("waycal");

    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_anchor(Edge::Top, true);
    window.set_margin(Edge::Top, 0);
    match anchor {
        Anchor::Left  => window.set_anchor(Edge::Left, true),
        Anchor::Right => window.set_anchor(Edge::Right, true),
        Anchor::Center => {}
    }

    // Shared state
    let view_state   = Rc::new(RefCell::new(ViewDate::today()));
    let selected_day = Rc::new(RefCell::new(Local::now().date_naive()));
    let month_data: Rc<RefCell<Option<MonthCache>>> = Rc::new(RefCell::new(None));

    // Root horizontal box
    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    outer.add_css_class("waycal-root");
    if load_rounded() { outer.add_css_class("rounded"); }

    // Left column
    let left_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    let header = gtk4::Label::new(None);
    header.add_css_class("waycal-header");
    header.set_halign(gtk4::Align::Center);

    let grid = gtk4::Grid::new();
    grid.set_row_spacing(2);
    grid.set_column_spacing(2);
    grid.set_halign(gtk4::Align::Center);

    let footer = gtk4::Label::new(Some(
        "\u{2190}\u{2192} mo   \u{2191}\u{2193} yr   \u{23CE} today   s style"
    ));
    footer.add_css_class("waycal-footer");
    footer.set_halign(gtk4::Align::Center);

    left_box.append(&header);
    left_box.append(&grid);
    left_box.append(&footer);

    // Right panel
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.add_css_class("waycal-panel");
    panel.set_valign(gtk4::Align::Start);

    outer.append(&left_box);
    outer.append(&panel);
    window.set_child(Some(&outer));

    // Initial render (no data yet)
    render_all(&grid, &header, &panel, *view_state.borrow(), *selected_day.borrow(),
               &month_data, &selected_day);

    // Kick off background load
    let v = *view_state.borrow();
    spawn_load(v.year, v.month, (*config).clone(), &month_data, &grid, &header, &panel, &view_state, &selected_day);

    // Keyboard handler
    let key = gtk4::EventControllerKey::new();
    {
        let config       = config.clone();
        let view_state   = view_state.clone();
        let selected_day = selected_day.clone();
        let month_data   = month_data.clone();
        let grid         = grid.clone();
        let header       = header.clone();
        let panel        = panel.clone();
        let window       = window.clone();
        let outer        = outer.clone();

        key.connect_key_pressed(move |_, keyval, _, _| {
            let current = *view_state.borrow();
            let (next, month_changed) = match keyval {
                gdk::Key::Left  => (current.shift_month(-1), true),
                gdk::Key::Right => (current.shift_month(1),  true),
                gdk::Key::Up    => (current.shift_year(-1),  true),
                gdk::Key::Down  => (current.shift_year(1),   true),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    *selected_day.borrow_mut() = Local::now().date_naive();
                    (ViewDate::today(), true)
                }
                gdk::Key::Escape => { window.close(); return glib::Propagation::Stop; }
                gdk::Key::s | gdk::Key::S => {
                    let now_rounded = !outer.has_css_class("rounded");
                    if now_rounded { outer.add_css_class("rounded"); }
                    else           { outer.remove_css_class("rounded"); }
                    save_rounded(now_rounded);
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            };

            *view_state.borrow_mut() = next;

            if month_changed {
                *month_data.borrow_mut() = None;
                render_all(&grid, &header, &panel, next, *selected_day.borrow(),
                           &month_data, &selected_day);
                spawn_load(next.year, next.month, (*config).clone(), &month_data, &grid, &header, &panel,
                           &view_state, &selected_day);
            } else {
                render_all(&grid, &header, &panel, next, *selected_day.borrow(),
                           &month_data, &selected_day);
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(key);
    window.present();
}

// ── Background data loading ───────────────────────────────────────────────────

fn spawn_load(
    year: i32,
    month: u32,
    config:       config::Config,
    month_data:   &Rc<RefCell<Option<MonthCache>>>,
    grid:         &gtk4::Grid,
    header:       &gtk4::Label,
    panel:        &gtk4::Box,
    view_state:   &Rc<RefCell<ViewDate>>,
    selected_day: &Rc<RefCell<NaiveDate>>,
) {
    let (tx, rx) = std::sync::mpsc::channel::<Result<MonthCache, String>>();

    std::thread::spawn(move || {
        let result = gcal::load_or_fetch(year, month, &config).map_err(|e| e.to_string());
        let _ = tx.send(result);
    });

    let rx = Rc::new(rx);
    let month_data   = month_data.clone();
    let grid         = grid.clone();
    let header       = header.clone();
    let panel        = panel.clone();
    let view_state   = view_state.clone();
    let selected_day = selected_day.clone();

    glib::idle_add_local(move || {
        match rx.try_recv() {
            Ok(result) => {
                let current = *view_state.borrow();
                if current.year == year && current.month == month {
                    if let Ok(cache) = result {
                        *month_data.borrow_mut() = Some(cache);
                    }
                    render_all(&grid, &header, &panel, current, *selected_day.borrow(),
                               &month_data, &selected_day);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
}

// ── Render helpers ────────────────────────────────────────────────────────────

fn render_all(
    grid:         &gtk4::Grid,
    header:       &gtk4::Label,
    panel:        &gtk4::Box,
    v:            ViewDate,
    sel:          NaiveDate,
    month_data:   &Rc<RefCell<Option<MonthCache>>>,
    selected_day: &Rc<RefCell<NaiveDate>>,
) {
    render_grid(grid, header, v, sel, month_data, selected_day, panel);
    render_panel(panel, sel, month_data);
}

// ── Grid ──────────────────────────────────────────────────────────────────────

fn render_grid(
    grid:         &gtk4::Grid,
    header:       &gtk4::Label,
    v:            ViewDate,
    sel:          NaiveDate,
    month_data:   &Rc<RefCell<Option<MonthCache>>>,
    selected_day: &Rc<RefCell<NaiveDate>>,
    panel:        &gtk4::Box,
) {
    header.set_text(&format!("{} {}", month_name(v.month), v.year));
    while let Some(c) = grid.first_child() { grid.remove(&c); }

    for (i, name) in ["Mo","Tu","We","Th","Fr","Sa","Su"].iter().enumerate() {
        let lbl = gtk4::Label::new(Some(name));
        lbl.add_css_class("waycal-weekday");
        grid.attach(&lbl, i as i32, 0, 1, 1);
    }

    let first      = NaiveDate::from_ymd_opt(v.year, v.month, 1).unwrap();
    let lead       = first.weekday().num_days_from_monday() as i32;
    let days       = days_in_month(v.year, v.month) as i32;
    let today      = Local::now().date_naive();
    let is_cur_mo  = today.year() == v.year && today.month() == v.month;

    let prev = v.shift_month(-1);
    let prev_days = days_in_month(prev.year, prev.month) as i32;
    for i in 0..lead {
        let d    = prev_days - lead + 1 + i;
        let date = NaiveDate::from_ymd_opt(prev.year, prev.month, d as u32).unwrap();
        let cell = make_day_cell(d, date, false, false, false, &[], selected_day, month_data, panel);
        grid.attach(&cell, i, 1, 1, 1);
    }

    for d in 1..=days {
        let idx  = lead + d - 1;
        let col  = idx % 7;
        let row  = idx / 7 + 1;
        let date = NaiveDate::from_ymd_opt(v.year, v.month, d as u32).unwrap();
        let date_str = date.format("%Y-%m-%d").to_string();

        let dots: Vec<(String, String)> = {
            let md = month_data.borrow();
            if let Some(cache) = md.as_ref() {
                let evs = cache.events_for_date(&date_str);
                let mut seen: Vec<(String, String)> = Vec::new();
                let mut seen_colors = std::collections::HashSet::new();
                for e in evs {
                    if seen_colors.insert(e.color.clone()) {
                        seen.push((e.color.clone(), e.icon.clone()));
                        if seen.len() == 2 { break; }
                    }
                }
                seen
            } else { vec![] }
        };

        let is_today = is_cur_mo && d == today.day() as i32;
        let cell = make_day_cell(d, date, true, is_today, date == sel,
                                 &dots, selected_day, month_data, panel);
        grid.attach(&cell, col, row, 1, 1);
    }

    let total    = lead + days;
    let trailing = (7 - total % 7) % 7;
    let next     = v.shift_month(1);
    for i in 0..trailing {
        let idx  = total + i;
        let col  = idx % 7;
        let row  = idx / 7 + 1;
        let date = NaiveDate::from_ymd_opt(next.year, next.month, (i + 1) as u32).unwrap();
        let cell = make_day_cell(i + 1, date, false, false, false, &[], selected_day, month_data, panel);
        grid.attach(&cell, col, row, 1, 1);
    }
}

fn make_day_cell(
    day_num:      i32,
    date:         NaiveDate,
    in_month:     bool,
    is_today:     bool,
    is_selected:  bool,
    dots:         &[(String, String)],
    selected_day: &Rc<RefCell<NaiveDate>>,
    month_data:   &Rc<RefCell<Option<MonthCache>>>,
    panel:        &gtk4::Box,
) -> gtk4::Box {
    let cell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    cell.set_halign(gtk4::Align::Center);
    cell.add_css_class("waycal-day");
    if !in_month  { cell.add_css_class("dim"); }
    if is_today   { cell.add_css_class("today"); }
    if is_selected { cell.add_css_class("selected"); }

    // Day number
    let num_lbl = gtk4::Label::new(Some(&day_num.to_string()));
    num_lbl.add_css_class("waycal-day-num");
    num_lbl.set_halign(gtk4::Align::Center);
    cell.append(&num_lbl);

    // Event dots — use Pango markup for color (no deprecated style_context)
    let dots_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    dots_row.add_css_class("waycal-dots-row");
    dots_row.set_halign(gtk4::Align::Center);
    for (color, _icon) in dots {
        let dot = gtk4::Label::new(None);
        dot.set_markup(&format!("<span color='{}'>⬤</span>", color));
        dots_row.append(&dot);
    }
    cell.append(&dots_row);

    // Click handler (in-month only)
    if in_month {
        let gesture      = gtk4::GestureClick::new();
        let selected_day = selected_day.clone();
        let month_data   = month_data.clone();
        let panel        = panel.clone();
        let cell_ref     = cell.clone();

        gesture.connect_released(move |_, _, _, _| {
            *selected_day.borrow_mut() = date;
            render_panel(&panel, date, &month_data);

            // Update .selected class on all day cells (direct children of the grid)
            if let Some(grid_widget) = cell_ref.parent() {
                let mut sib = grid_widget.first_child();
                while let Some(w) = sib {
                    sib = w.next_sibling();
                    if w.has_css_class("waycal-day") {
                        w.remove_css_class("selected");
                    }
                }
            }
            cell_ref.add_css_class("selected");
        });
        cell.add_controller(gesture);
    }

    cell
}

// ── Event panel ───────────────────────────────────────────────────────────────

fn render_panel(panel: &gtk4::Box, sel: NaiveDate, month_data: &Rc<RefCell<Option<MonthCache>>>) {
    while let Some(c) = panel.first_child() { panel.remove(&c); }

    let hdr = gtk4::Label::new(Some(&day_label_text(sel)));
    hdr.add_css_class("waycal-panel-header");
    hdr.set_halign(gtk4::Align::Start);
    hdr.set_wrap(true);
    panel.append(&hdr);

    let md = month_data.borrow();
    match md.as_ref() {
        None => {
            let lbl = gtk4::Label::new(Some("Loading…"));
            lbl.add_css_class("waycal-panel-empty");
            lbl.set_halign(gtk4::Align::Start);
            panel.append(&lbl);
        }
        Some(cache) => {
            let date_str = sel.format("%Y-%m-%d").to_string();
            let events   = cache.events_for_date(&date_str);
            if events.is_empty() {
                let lbl = gtk4::Label::new(Some("No events"));
                lbl.add_css_class("waycal-panel-empty");
                lbl.set_halign(gtk4::Align::Start);
                panel.append(&lbl);
            } else {
                for ev in events {
                    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                    row.add_css_class("waycal-event-row");

                    // Colored icon via Pango markup
                    let icon_lbl = gtk4::Label::new(None);
                    icon_lbl.set_markup(&format!("<span color='{}'>{}</span>", ev.color, ev.icon));
                    icon_lbl.add_css_class("waycal-event-icon");

                    let time_lbl = gtk4::Label::new(Some(&ev.start_time));
                    time_lbl.add_css_class("waycal-event-time");
                    time_lbl.set_halign(gtk4::Align::Start);

                    let title_lbl = gtk4::Label::new(Some(&ev.title));
                    title_lbl.add_css_class("waycal-event-title");
                    title_lbl.set_halign(gtk4::Align::Start);
                    title_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    title_lbl.set_max_width_chars(22);

                    row.append(&icon_lbl);
                    row.append(&time_lbl);
                    row.append(&title_lbl);
                    panel.append(&row);
                }
            }
        }
    }
}
