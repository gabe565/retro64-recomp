//! The settings overlay: a menu drawn by the runtime over the game, opened with
//! the "Open settings" binding (F10 or a controller's Back button by default).
//!
//! While it is open the game is frozen: the guest thread stays inside the `Flip`
//! that presented the last frame (`run`), so no virtual time passes, and the
//! audio mixer is paused. Closing it resumes exactly where the game stopped.
//!
//! The difficulty is set directly (0x4370e4, read by gameplay every frame), and the
//! title screen's "F3: DIFFICULTY - NORMAL" text is updated as the game's own F3
//! handler does (`set_difficulty`), so it works on any screen and mid-game. The
//! high scores are reached by pressing the game's F2 once it resumes
//! (`taps`): changing screens happens only inside the game's input handling.
//! Text uses the game's title font (`Fonts/nj*.bmp`), with a few drawn glyphs for
//! characters it lacks.

use crate::bindings::{Bindings, Input, ACTIONS};
use crate::Runtime;
use sdl3_sys::events::*;
use sdl3_sys::gamepad::*;
use std::collections::HashMap;

/// Difficulty (float): 4 Normal, 1 Hard, 8 Easy; F3 cycles Normal → Hard → Easy.
const DIFFICULTY: u32 = 0x4370e4;
/// Nonzero while the title menu accepts F2 (high scores) and F3 (difficulty).
const TITLE_MENU: u32 = 0x8429d0;
/// The screen's text objects: list head (linked through each object's first word).
const TEXT_LIST: u32 = 0x84223c;
/// In a text object: letter records of 164 bytes, the character first, from
/// record 1; the letter count is at +0x55b9c.
const LETTER: u32 = 164;
const LETTER_COUNT: u32 = 0x55b9c;
/// Labels as the F3 handler writes them (0x437e60..), six letters each.
const DIFFICULTIES: [(f32, &[u8; 6]); 3] = [(4.0, b"NORMAL"), (1.0, b"HARD  "), (8.0, b"EASY  ")];

// SDL scancodes.
const SC_RETURN: i32 = 40;
const SC_ESCAPE: i32 = 41;
const SC_BACKSPACE: i32 = 42;
const SC_SPACE: i32 = 44;
const SC_F2: i32 = 59;
const SC_DELETE: i32 = 76;
const SC_RIGHT: i32 = 79;
const SC_LEFT: i32 = 80;
const SC_DOWN: i32 = 81;
const SC_UP: i32 = 82;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Controls,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Resume,
    Difficulty,
    HighScores,
    Controls,
    Quit,
    Action(usize),
    ResetControls,
    Back,
}

/// What a menu input means.
enum Nav {
    Up,
    Down,
    Left,
    Right,
    Select,
    Remove,
    Back,
    Close,
}

#[derive(Default)]
pub struct Overlay {
    /// Open, or requested to open at the next flip.
    pub open: bool,
    page: Option<Page>,
    sel: usize,
    /// Waiting for a key or button to add to the selected action.
    capture: bool,
    /// Game keys to tap after resuming, one at a time (scancode).
    taps: Vec<i32>,
    /// Flips into the current tap: down at 0, up at 2, next tap from 4.
    tap_phase: u32,
    font: Option<HashMap<char, [u8; 256]>>,
    dirty: bool,
}

impl Overlay {
    fn page(&self) -> Page {
        self.page.unwrap_or(Page::Main)
    }

    fn items(&self) -> Vec<Item> {
        match self.page() {
            Page::Main => vec![Item::Resume, Item::Difficulty, Item::HighScores, Item::Controls, Item::Quit],
            Page::Controls => (0..ACTIONS.len()).map(Item::Action).chain([Item::ResetControls, Item::Back]).collect(),
        }
    }
}

fn title_menu(rt: &Runtime) -> bool {
    rt.mem.r32(TITLE_MENU) != 0
}

/// The game's current difficulty, as an index into DIFFICULTIES.
fn current_difficulty(rt: &Runtime) -> usize {
    let v = f32::from_bits(rt.mem.r32(DIFFICULTY));
    DIFFICULTIES.iter().position(|&(d, _)| d == v).unwrap_or(0)
}

/// Ask for the overlay at the next flip (F10 or controller Back, from `user::pump`).
pub fn request(rt: &mut Runtime) {
    if !rt.user.headless && !rt.user.ignore_host_input && rt.gpu.is_some() {
        rt.overlay.open = true;
    }
}

/// After every flip: run the overlay if it was requested, else deliver pending taps.
pub(crate) fn after_flip(rt: &mut Runtime, front: u32) {
    if rt.overlay.open {
        run(rt, front);
    } else if !rt.overlay.taps.is_empty() {
        let o = &mut rt.overlay;
        let sc = o.taps[0];
        let phase = o.tap_phase;
        o.tap_phase += 1;
        match phase {
            0 => crate::user::key_input_rt(rt, sc, true, false, false),
            2 => crate::user::key_input_rt(rt, sc, false, false, false),
            p if p >= 4 => {
                rt.overlay.taps.remove(0);
                rt.overlay.tap_phase = 0;
            }
            _ => {}
        }
    }
}

/// The overlay's modal loop: show it over frame `front` until it is closed.
fn run(rt: &mut Runtime, front: u32) {
    // Release whatever the player was holding, so nothing sticks while frozen.
    for (sc, down, repeat) in rt.user.bindings.release_all() {
        crate::user::key_input_rt(rt, sc, down, repeat, false);
    }
    rt.fmod.mixer().paused = true;
    if rt.overlay.font.is_none() {
        rt.overlay.font = Some(load_font(rt));
    }
    let o = &mut rt.overlay;
    o.page = Some(Page::Main);
    o.sel = 0;
    o.capture = false;
    o.dirty = true;
    let (w, h) = { let s = rt.com.surface(front); (s.w, s.h) };
    let mut image = Vec::new();
    while rt.overlay.open {
        poll_events(rt);
        let changed = std::mem::take(&mut rt.overlay.dirty);
        if changed || image.len() != (w * h * 4) as usize {
            image = render(rt, w, h);
        }
        crate::render::gpu::present_overlay(rt, front, &image, changed);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    rt.fmod.mixer().paused = false;
    // No catch-up burst: the frozen time is not owed to the simulation.
    if let Some(c) = rt.clock.cadence.as_mut() {
        c.forgive_lag(std::time::Duration::ZERO);
    }
}

fn poll_events(rt: &mut Runtime) {
    // SAFETY: SDL_Event is plain data; SDL fills it.
    let mut ev: SDL_Event = unsafe { std::mem::zeroed() };
    while unsafe { SDL_PollEvent(&mut ev) } {
        let ty = unsafe { ev.r#type };
        if ty == SDL_EVENT_QUIT.0 || ty == SDL_EVENT_WINDOW_CLOSE_REQUESTED.0 {
            crate::user::close_requested_rt(rt);
            rt.overlay.open = false;
        } else if ty == SDL_EVENT_WINDOW_FOCUS_LOST.0 || ty == SDL_EVENT_WINDOW_FOCUS_GAINED.0 {
            if !rt.user.run_in_background {
                crate::user::focus_changed_rt(rt, ty == SDL_EVENT_WINDOW_FOCUS_GAINED.0);
            }
        } else if ty == SDL_EVENT_GAMEPAD_ADDED.0 {
            rt.user.gamepads.added(unsafe { ev.gdevice.which });
        } else if ty == SDL_EVENT_GAMEPAD_REMOVED.0 {
            rt.user.gamepads.removed(unsafe { ev.gdevice.which });
        } else if ty == SDL_EVENT_KEY_DOWN.0 || ty == SDL_EVENT_KEY_UP.0 {
            let k = unsafe { ev.key };
            if crate::user::runtime_shortcut_rt(rt, k.scancode.0, k.down, k.repeat, k.r#mod.0) || !k.down {
                continue;
            }
            input(rt, Input::Key(k.scancode.0));
        } else if ty == SDL_EVENT_GAMEPAD_BUTTON_DOWN.0 {
            input(rt, Input::Pad(unsafe { ev.gbutton.button } as i32));
        }
    }
}

fn nav(i: Input) -> Option<Nav> {
    let pad = |b: SDL_GamepadButton| Input::Pad(b.0);
    Some(match i {
        Input::Key(SC_UP) => Nav::Up,
        Input::Key(SC_DOWN) => Nav::Down,
        Input::Key(SC_LEFT) => Nav::Left,
        Input::Key(SC_RIGHT) => Nav::Right,
        Input::Key(SC_RETURN | SC_SPACE) => Nav::Select,
        Input::Key(SC_BACKSPACE | SC_DELETE) => Nav::Remove,
        Input::Key(SC_ESCAPE) => Nav::Back,
        _ if i == pad(SDL_GAMEPAD_BUTTON_DPAD_UP) => Nav::Up,
        _ if i == pad(SDL_GAMEPAD_BUTTON_DPAD_DOWN) => Nav::Down,
        _ if i == pad(SDL_GAMEPAD_BUTTON_DPAD_LEFT) => Nav::Left,
        _ if i == pad(SDL_GAMEPAD_BUTTON_DPAD_RIGHT) => Nav::Right,
        _ if i == pad(SDL_GAMEPAD_BUTTON_SOUTH) => Nav::Select,
        _ if i == pad(SDL_GAMEPAD_BUTTON_WEST) => Nav::Remove,
        _ if i == pad(SDL_GAMEPAD_BUTTON_EAST) => Nav::Back,
        _ if i == pad(SDL_GAMEPAD_BUTTON_START) => Nav::Close,
        _ => return None,
    })
}

fn input(rt: &mut Runtime, i: Input) {
    rt.overlay.dirty = true;
    if rt.overlay.capture {
        rt.overlay.capture = false;
        // Escape cancels.
        if i == Input::Key(SC_ESCAPE) {
            return;
        }
        if let Some(Item::Action(a)) = rt.overlay.items().get(rt.overlay.sel).copied() {
            let mut inputs = rt.user.bindings.inputs.clone();
            if !inputs[a].contains(&i) {
                inputs[a].push(i);
            }
            set_bindings(rt, inputs);
        }
        return;
    }
    // The settings binding closes the overlay as it opened it.
    let Some(n) = (if rt.user.bindings.opens_settings(i) { Some(Nav::Close) } else { nav(i) }) else { return };
    let items = rt.overlay.items();
    let item = items[rt.overlay.sel.min(items.len() - 1)];
    let (title_menu, current) = (title_menu(rt), current_difficulty(rt));
    let o = &mut rt.overlay;
    match n {
        Nav::Up => o.sel = (o.sel + items.len() - 1) % items.len(),
        Nav::Down => o.sel = (o.sel + 1) % items.len(),
        Nav::Close => o.open = false,
        Nav::Back if o.page() == Page::Controls => {
            o.page = Some(Page::Main);
            o.sel = 3; // Controls
        }
        Nav::Back => o.open = false,
        Nav::Left | Nav::Right if item == Item::Difficulty => {
            let step = if matches!(n, Nav::Right) { 1 } else { DIFFICULTIES.len() - 1 };
            set_difficulty(rt, (current + step) % DIFFICULTIES.len());
        }
        Nav::Remove => {
            if let Item::Action(a) = item {
                let mut inputs = rt.user.bindings.inputs.clone();
                // Keep a way to open the overlay.
                if ACTIONS[a].key != crate::bindings::SETTINGS || inputs[a].len() > 1 {
                    inputs[a].pop();
                    set_bindings(rt, inputs);
                }
            }
        }
        Nav::Select => match item {
            Item::Resume => o.open = false,
            Item::Difficulty => set_difficulty(rt, (current + 1) % DIFFICULTIES.len()),
            Item::HighScores if title_menu => {
                o.taps.push(SC_F2);
                o.open = false;
            }
            Item::Controls => {
                o.page = Some(Page::Controls);
                o.sel = 0;
            }
            Item::Quit => {
                crate::user::close_requested_rt(rt);
                rt.overlay.open = false;
            }
            Item::Action(_) => o.capture = true,
            Item::ResetControls => set_bindings(rt, Bindings::default().inputs),
            Item::Back => {
                o.page = Some(Page::Main);
                o.sel = 3; // Controls
            }
            _ => {}
        },
        _ => {}
    }
}

/// Set the difficulty and the title screen's label.
fn set_difficulty(rt: &mut Runtime, d: usize) {
    rt.mem.w32(DIFFICULTY, DIFFICULTIES[d].0.to_bits());
    write_difficulty_label(rt, DIFFICULTIES[d].1);
}

/// The title screen's difficulty label, as the game's F3 handler (0x413143) writes
/// it: the last six letters of every text object that starts with "F2" (the
/// title's second line, ending "F3: DIFFICULTY - NORMAL").
fn write_difficulty_label(rt: &mut Runtime, label: &[u8; 6]) {
    let mut obj = rt.mem.r32(TEXT_LIST);
    while obj != 0 {
        if rt.mem.r8(obj + LETTER) == b'F' && rt.mem.r8(obj + 2 * LETTER) == b'2' {
            let n = rt.mem.r32(obj + LETTER_COUNT);
            // Only a line that ends in one of the labels (the title's has 60-odd letters).
            let at = |k: u32| obj + LETTER * (n - 5 + k);
            if (8..1000).contains(&n) && DIFFICULTIES.iter().any(|(_, l)| (0..6).all(|k| rt.mem.r8(at(k)) == l[k as usize])) {
                for (k, &c) in label.iter().enumerate() {
                    rt.mem.w8(at(k as u32), c);
                }
            }
        }
        obj = rt.mem.r32(obj);
    }
}

/// Change the bindings and save them to `config.toml`.
fn set_bindings(rt: &mut Runtime, inputs: Vec<Vec<Input>>) {
    rt.user.bindings.set(inputs);
    crate::config::save_from(rt);
}

// ---- drawing -------------------------------------------------------------------

/// The glyphs: the game's `nj` font (16×16, brightness as coverage) where it has
/// the character, drawn ones otherwise.
fn load_font(rt: &mut Runtime) -> HashMap<char, [u8; 256]> {
    let mut font = HashMap::new();
    let files = ('A'..='Z').chain('0'..='9').map(|c| (c, c.to_string()))
        .chain([('!', "!"), ('.', "."), (',', ","), ('-', "-"), ('_', "_"), ('\'', "'"), ('/', "^"), (':', "`")].map(|(c, f)| (c, f.to_string())));
    for (c, name) in files {
        if let Some(g) = read_glyph(rt, &format!("Fonts\\nj{name}.bmp")) {
            font.insert(c, g);
        }
    }
    for (c, rows) in DRAWN {
        if font.contains_key(c) {
            continue;
        }
        let mut g = [0u8; 256];
        for (y, row) in rows.iter().enumerate() {
            for (x, b) in row.bytes().enumerate() {
                if b == b'#' {
                    for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        g[(y * 2 + dy) * 16 + x * 2 + dx] = 230;
                    }
                }
            }
        }
        font.insert(*c, g);
    }
    font
}

/// Characters the game's font lacks, as 8×8 drawings scaled to 16×16.
const DRAWN: &[(char, [&str; 8])] = &[
    ('=', ["        ", "        ", " ###### ", "        ", "        ", " ###### ", "        ", "        "]),
    ('+', ["        ", "   ##   ", "   ##   ", " ###### ", " ###### ", "   ##   ", "   ##   ", "        "]),
    ('[', ["  ####  ", "  ##    ", "  ##    ", "  ##    ", "  ##    ", "  ##    ", "  ##    ", "  ####  "]),
    (']', ["  ####  ", "    ##  ", "    ##  ", "    ##  ", "    ##  ", "    ##  ", "    ##  ", "  ####  "]),
    ('(', ["    ##  ", "   ##   ", "  ##    ", "  ##    ", "  ##    ", "  ##    ", "   ##   ", "    ##  "]),
    (')', ["  ##    ", "   ##   ", "    ##  ", "    ##  ", "    ##  ", "    ##  ", "   ##   ", "  ##    "]),
    ('<', ["     ## ", "    ##  ", "   ##   ", "  ##    ", "  ##    ", "   ##   ", "    ##  ", "     ## "]),
    ('>', [" ##     ", "  ##    ", "   ##   ", "    ##  ", "    ##  ", "   ##   ", "  ##    ", " ##     "]),
    ('?', ["  ####  ", " ##  ## ", "     ## ", "    ##  ", "   ##   ", "   ##   ", "        ", "   ##   "]),
    (';', ["        ", "   ##   ", "   ##   ", "        ", "        ", "   ##   ", "   ##   ", "  ##    "]),
    ('`', ["  ##    ", "   ##   ", "        ", "        ", "        ", "        ", "        ", "        "]),
    ('\\', [" ##     ", " ##     ", "  ##    ", "   ##   ", "    ##  ", "     ## ", "     ## ", "        "]),
];

/// A 16×16 glyph from a 24-bit BMP in the game folder; coverage is the brightest channel.
fn read_glyph(rt: &mut Runtime, name: &str) -> Option<[u8; 256]> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const OPEN_EXISTING: u32 = 3;
    let h = rt.files.create(name, GENERIC_READ, OPEN_EXISTING).ok()?;
    let size = rt.files.size(h).unwrap_or(0) as usize;
    let mut d = vec![0u8; size];
    let n = rt.files.read(h, &mut d);
    rt.files.close(h);
    if n != Some(size) || size < 54 {
        return None;
    }
    let off = u32::from_le_bytes(d[10..14].try_into().ok()?) as usize;
    let (w, ht) = (i32::from_le_bytes(d[18..22].try_into().ok()?), i32::from_le_bytes(d[22..26].try_into().ok()?));
    let bpp = u16::from_le_bytes(d[28..30].try_into().ok()?);
    if w != 16 || ht != 16 || bpp != 24 {
        return None;
    }
    let mut g = [0u8; 256];
    for y in 0..16 {
        for x in 0..16 {
            let p = off + (15 - y) * 48 + x * 3;
            g[y * 16 + x] = *d.get(p..p + 3)?.iter().max()?;
        }
    }
    Some(g)
}

const WHITE: [u8; 3] = [235, 235, 235];
const YELLOW: [u8; 3] = [255, 214, 64];
const GREY: [u8; 3] = [120, 120, 120];
const GREEN: [u8; 3] = [90, 220, 90];

struct Canvas {
    px: Vec<u8>,
    w: u32,
    h: u32,
    /// Glyph size in pixels (the 16×16 glyphs are resampled to it).
    glyph: f32,
}

impl Canvas {
    /// Straight-alpha "over": `rgb` at coverage `a` (0..=1) over the pixel.
    fn blend(&mut self, x: u32, y: u32, rgb: [u8; 3], a: f32) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let p = &mut self.px[((y * self.w + x) * 4) as usize..][..4];
        let da = p[3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for c in 0..3 {
            p[c] = ((rgb[c] as f32 * a + p[c] as f32 * da * (1.0 - a)) / oa).round() as u8;
        }
        p[3] = (oa * 255.0).round() as u8;
    }

    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3], a: f32) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.blend(xx, yy, rgb, a);
            }
        }
    }

    /// Letter pitch: a little wider than a glyph, as the game spaces its text.
    fn advance(&self) -> u32 {
        (self.glyph * 1.15).round() as u32
    }

    fn width(&self, text: &str) -> u32 {
        text.chars().count() as u32 * self.advance()
    }

    fn text(&mut self, font: &HashMap<char, [u8; 256]>, x: u32, y: u32, text: &str, rgb: [u8; 3]) {
        let size = self.glyph.ceil() as u32;
        let step = 16.0 / self.glyph;
        for (i, c) in text.chars().enumerate() {
            let Some(g) = font.get(&c.to_ascii_uppercase()) else { continue };
            let gx = x + i as u32 * self.advance();
            for py in 0..size {
                for px in 0..size {
                    let a = sample(g, (px as f32 + 0.5) * step - 0.5, (py as f32 + 0.5) * step - 0.5);
                    if a > 0.0 {
                        self.blend(gx + px, y + py, rgb, a);
                    }
                }
            }
        }
    }

    fn centered(&mut self, font: &HashMap<char, [u8; 256]>, y: u32, text: &str, rgb: [u8; 3]) {
        let x = self.w.saturating_sub(self.width(text)) / 2;
        self.text(font, x, y, text, rgb);
    }
}

/// Bilinear coverage (0..=1) of a 16×16 glyph at (x, y) in glyph pixels.
fn sample(g: &[u8; 256], x: f32, y: f32) -> f32 {
    let at = |x: i32, y: i32| if (0..16).contains(&x) && (0..16).contains(&y) { g[(y * 16 + x) as usize] as f32 / 255.0 } else { 0.0 };
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// `names` joined with ", " in at most `max` letters: as many as fit, then "+N".
fn fit(names: &[String], max: usize) -> String {
    let all = names.join(", ");
    if all.len() <= max {
        return all;
    }
    let mut out = String::new();
    for (i, n) in names.iter().enumerate() {
        let more = format!(" +{}", names.len() - i);
        let next = if out.is_empty() { n.clone() } else { format!("{out}, {n}") };
        if next.len() + format!(" +{}", names.len() - i - 1).len() > max {
            return if out.is_empty() { format!("{}{more}", &n[..max.saturating_sub(more.len()).min(n.len())]) } else { out + &more };
        }
        out = next;
    }
    out
}

/// The overlay at the frame's size, as straight-alpha RGBA8.
fn render(rt: &Runtime, w: u32, h: u32) -> Vec<u8> {
    let o = &rt.overlay;
    let empty = HashMap::new();
    let font = o.font.as_ref().unwrap_or(&empty);
    // About the size of the game's own menu text.
    let glyph = (h as f32 / 45.0).max(12.0);
    let mut c = Canvas { px: vec![0; (w * h * 4) as usize], w, h, glyph };
    c.rect(0, 0, w, h, [0, 0, 0], 0.85);
    let line = (glyph * 1.7).round() as u32;
    let unit = glyph / 16.0;
    let px = |n: f32| (n * unit).round() as u32;
    let items = o.items();
    let title = match o.page() {
        Page::Main => "SETTINGS",
        Page::Controls => "CONTROLS",
    };
    let top = px(48.0);
    c.centered(font, top, title, GREEN);
    let title_menu = title_menu(rt);
    let mut y = top + line * 2;
    // Two columns of COLUMN letters each: labels, then values.
    const COLUMN: u32 = 19;
    let left = (w / 2).saturating_sub(COLUMN * c.advance());
    for (i, item) in items.iter().enumerate() {
        let selected = i == o.sel;
        let (label, value, enabled) = match *item {
            Item::Resume => ("RESUME".to_string(), String::new(), true),
            Item::Difficulty => {
                let name = String::from_utf8_lossy(DIFFICULTIES[current_difficulty(rt)].1).trim().to_string();
                ("DIFFICULTY".to_string(), format!("< {name} >"), true)
            }
            Item::HighScores => ("HIGH SCORES".to_string(), String::new(), title_menu),
            Item::Controls => ("CONTROLS".to_string(), String::new(), true),
            Item::Quit => ("QUIT BUGATRON".to_string(), String::new(), true),
            Item::Action(a) => {
                let names: Vec<String> = rt.user.bindings.inputs[a].iter().map(|i| i.name().to_uppercase()).collect();
                let v = if selected && o.capture { "PRESS A KEY".to_string() } else if names.is_empty() { "-".to_string() } else { fit(&names, COLUMN as usize) };
                (ACTIONS[a].label.to_uppercase(), v, true)
            }
            Item::ResetControls => ("RESET TO DEFAULTS".to_string(), String::new(), true),
            Item::Back => ("BACK".to_string(), String::new(), true),
        };
        if selected {
            c.rect(left.saturating_sub(px(10.0)), y.saturating_sub(px(5.0)), 2 * COLUMN * c.advance() + px(20.0), px(26.0), [40, 110, 40], 0.75);
        }
        let rgb = if !enabled { GREY } else if selected { YELLOW } else { WHITE };
        c.text(font, left, y, &label, rgb);
        if !value.is_empty() {
            c.text(font, left + COLUMN * c.advance(), y, &value, rgb);
        }
        y += line;
    }
    let hint = match (o.page(), o.capture) {
        (_, true) => "ESC: CANCEL",
        (Page::Main, _) => if title_menu { "ENTER/A: SELECT   ESC/B: CLOSE" } else { "HIGH SCORES: FROM THE TITLE SCREEN" },
        (Page::Controls, _) => "ENTER/A: ADD   BACKSPACE/X: REMOVE   ESC/B: BACK",
    };
    c.centered(font, h.saturating_sub(px(48.0)), hint, GREY);
    c.px
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn values_fit_their_column() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(fit(&n(&["PAGEUP", "="]), 19), "PAGEUP, =");
        assert_eq!(fit(&n(&["LEFT CTRL", "RIGHT CTRL"]), 19), "LEFT CTRL +1");
        assert_eq!(fit(&n(&["A", "B", "C", "D", "E", "F", "G", "H", "I"]), 19), "A, B, C, D, E, F +3");
        assert!(fit(&n(&["A VERY LONG KEY NAME INDEED"]), 19).len() <= 19);
    }
}
