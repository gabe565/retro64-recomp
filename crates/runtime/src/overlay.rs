//! The settings overlay: a menu drawn by the runtime over the game, opened with
//! the "Open settings" binding (F10 or a controller's Back button by default).
//!
//! While it is open the game is paused with its own pause (the P key's flag,
//! 0x8429c8, restored on close): it stops updating and drawing, and the music plays
//! on, so volume changes can be heard as they are made. A paused game still calls
//! `PeekMessageA` in a loop, which runs the overlay (`tick`): input, then the overlay
//! drawn over the last frame, then a refresh's wait.
//!
//! The difficulty is set directly (0x4370e4, read by gameplay every frame), and the
//! title screen's "F3: DIFFICULTY - NORMAL" text is updated as the game's own F3
//! handler does (`set_difficulty`), so it works on any screen and mid-game. The
//! high scores are reached with the game's own screen transition, from any screen,
//! gameplay included (`show_high_scores`).
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
/// Music and sound-effect volumes (int, 0..=100; defaults 90 and 25), changed in
/// steps of 10 by Page Up/Down and Home/End (0x413377..), each change applied by
/// `APPLY_VOLUMES(music - 1, sound, 0)` (cdecl floats) while sound is on (`SOUND_ON`).
const MUSIC_VOLUME: u32 = 0x4370a8;
/// The overlay's volume step (finer than the game's 10).
const VOLUME_STEP: i32 = 5;
const SOUND_VOLUME: u32 = 0x4370ac;
const APPLY_VOLUMES: u32 = 0x417210;
const SOUND_ON: u32 = 0x438144;
/// Nonzero: the game shows its frames per second and render and logic times in
/// milliseconds (toggled by Delete, 0x412df8).
const FRAME_TIME: u32 = 0x8429b8;
/// The main loop's state (0x40ba70): 5 runs a screen (menus, high scores and
/// gameplay alike), 3 fades out and tears it down, 1 loads the next.
const STATE: u32 = 0x4370a0;
const STATE_SCREEN: u32 = 5;
const STATE_FADE_OUT: u32 = 3;
/// The config the next screen is loaded from.
const NEXT_SCREEN: u32 = 0x81d548;
/// The fade's start time (float, in the game's 1/1024 s ticks), and the camera's
/// last three heights during it (set to 5 when it starts; it ends at 16, 16, 16).
const FADE_TIMERS: [u32; 3] = [0x98d4c4, 0x98d4b8, 0x98d4bc];
const FADE_START: u32 = 0x437864;
const TICK_NOW: u32 = 0x8429dc;
/// The camera's height (its position is 0x81d670..; the camera object 0x81d64c):
/// 0 on a screen, 16 at the "LOADING" sign between screens (`skip_loading`).
const CAMERA_Y: u32 = 0x81d674;
/// Where the new screen's camera starts: the sign is out of view below about 12.2
/// (and a screen's top edge comes into view at about 10.8).
const ENTER_FROM: f32 = 12.0;
/// Fade time (ticks of 2000) by which every screen tested has slid out of view.
const SKIP_AT: f32 = 1450.0;
/// Nonzero while the game is paused (its P key, 0x4122f7): the main loop's frame
/// function returns at once (0x40ba8c), so nothing updates or is drawn; the music
/// plays on.
const PAUSED: u32 = 0x8429c8;
/// Labels as the F3 handler writes them (0x437e60..), six letters each.
const DIFFICULTIES: [(f32, &[u8; 6]); 3] = [(4.0, b"NORMAL"), (1.0, b"HARD  "), (8.0, b"EASY  ")];

// SDL scancodes.
const SC_RETURN: i32 = 40;
const SC_ESCAPE: i32 = 41;
const SC_BACKSPACE: i32 = 42;
const SC_SPACE: i32 = 44;
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
    Music,
    Sound,
    Resolution,
    Fullscreen,
    SkipLoading,
    Logo,
    FrameTime,
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
    /// The volumes changed: apply them through the game once it resumes.
    apply_volumes: bool,
    /// The game's pause flag before the overlay opened.
    was_paused: bool,
    /// The last frame shown, which the overlay is drawn over.
    last_front: Option<u32>,
    /// The overlay as drawn (RGBA8, the frame's size).
    image: Vec<u8>,
    font: Option<HashMap<char, [u8; 256]>>,
    dirty: bool,
}

impl Overlay {
    fn page(&self) -> Page {
        self.page.unwrap_or(Page::Main)
    }

    fn items(&self) -> Vec<Item> {
        match self.page() {
            Page::Main => vec![Item::Resume, Item::Difficulty, Item::Music, Item::Sound, Item::Resolution, Item::Fullscreen, Item::SkipLoading, Item::Logo, Item::FrameTime, Item::HighScores, Item::Controls, Item::Quit],
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
    if rt.overlay.open || rt.user.headless || rt.user.ignore_host_input || rt.gpu.is_none() || rt.overlay.last_front.is_none() {
        return;
    }
    // Release whatever the player was holding, so nothing sticks while paused.
    for (sc, down, repeat) in rt.user.bindings.release_all() {
        crate::user::key_input_rt(rt, sc, down, repeat, false);
    }
    if rt.overlay.font.is_none() {
        rt.overlay.font = Some(load_font(rt));
    }
    // The game's own pause (its P key): it stops updating and drawing; the music plays on.
    rt.overlay.was_paused = rt.mem.r32(PAUSED) != 0;
    rt.mem.w32(PAUSED, 1);
    let o = &mut rt.overlay;
    o.open = true;
    o.page = Some(Page::Main);
    o.sel = 0;
    o.capture = false;
    o.dirty = true;
    o.image.clear();
}

/// After every flip: run the overlay if it was requested.
pub(crate) fn after_flip(rt: &mut Runtime, front: u32) {
    rt.overlay.last_front = Some(front);
    skip_loading(rt);
    fix_difficulty_label(rt);
}

/// From `PeekMessageA`, where game code may run: apply volumes changed in the
/// overlay with the game's own function, as its volume keys do.
pub(crate) fn deliver(cpu: &mut crate::Cpu) {
    let apply = crate::with_rt(|rt| std::mem::take(&mut rt.overlay.apply_volumes) && rt.mem.r32(SOUND_ON) != 0);
    if apply {
        let (music, sound) = crate::with_rt(|rt| (rt.mem.r32(MUSIC_VOLUME) as i32, rt.mem.r32(SOUND_VOLUME) as i32));
        crate::call_guest(cpu, APPLY_VOLUMES, &[((music - 1) as f32).to_bits(), (sound as f32).to_bits(), 0]);
    }
}

/// From `PeekMessageA` while the overlay is open (the game is paused and calls it in
/// a loop): handle input, draw the overlay over the last frame, and wait a refresh.
pub(crate) fn tick(rt: &mut Runtime) {
    poll_events(rt);
    let Some(front) = rt.overlay.last_front else { return close(rt) };
    if rt.overlay.open {
        let (w, h) = { let s = rt.com.surface(front); (s.w, s.h) };
        let changed = std::mem::take(&mut rt.overlay.dirty) || rt.overlay.image.len() != (w * h * 4) as usize;
        if changed {
            rt.overlay.image = render(rt, w, h);
        }
        let image = std::mem::take(&mut rt.overlay.image);
        crate::render::gpu::present_overlay(rt, front, &image, changed);
        rt.overlay.image = image;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    if !rt.overlay.open {
        close(rt);
    }
}

/// The overlay closed: restore the game's pause as it was, and show the game again.
fn close(rt: &mut Runtime) {
    rt.overlay.open = false;
    rt.mem.w32(PAUSED, rt.overlay.was_paused as u32);
    // The time spent in the menu is not owed to the simulation (no catch-up burst).
    if let Some(c) = rt.clock.cadence.as_mut() {
        c.forgive_lag(std::time::Duration::ZERO);
    }
    if let Some(front) = rt.overlay.last_front {
        crate::render::gpu::present(rt, front);
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
    let (in_screen, current) = (in_screen(rt), current_difficulty(rt));
    let o = &mut rt.overlay;
    match n {
        Nav::Up => o.sel = (o.sel + items.len() - 1) % items.len(),
        Nav::Down => o.sel = (o.sel + 1) % items.len(),
        Nav::Close => o.open = false,
        Nav::Back if o.page() == Page::Controls => {
            o.page = Some(Page::Main);
            o.sel = o.items().iter().position(|&i| i == Item::Controls).unwrap_or(0);
        }
        Nav::Back => o.open = false,
        Nav::Left | Nav::Right if matches!(item, Item::Music | Item::Sound) => {
            let at = if item == Item::Music { MUSIC_VOLUME } else { SOUND_VOLUME };
            let v = rt.mem.r32(at) as i32;
            // To the next multiple of VOLUME_STEP up or down (the game's keys step by 10).
            let v = if matches!(n, Nav::Right) { (v / VOLUME_STEP + 1) * VOLUME_STEP } else { (v + VOLUME_STEP - 1) / VOLUME_STEP * VOLUME_STEP - VOLUME_STEP };
            rt.mem.w32(at, v.clamp(0, 100) as u32);
            rt.overlay.apply_volumes = true;
        }
        Nav::Left | Nav::Right if item == Item::Resolution => {
            let choices = resolutions();
            let at = choices.iter().position(|&c| c == rt.display.resolution).unwrap_or(0);
            let step = if matches!(n, Nav::Right) { 1 } else { choices.len() - 1 };
            rt.display.resolution = choices[(at + step) % choices.len()];
            crate::config::save_from(rt);
        }
        Nav::Left | Nav::Right | Nav::Select if item == Item::Fullscreen => set_fullscreen(rt, !rt.display.fullscreen),
        Nav::Left | Nav::Right | Nav::Select if item == Item::Logo => {
            rt.display.gold_logo = !rt.display.gold_logo;
            rt.files.plain_logo = !rt.display.gold_logo;
            crate::config::save_from(rt);
        }
        Nav::Left | Nav::Right | Nav::Select if item == Item::SkipLoading => {
            rt.display.skip_loading = !rt.display.skip_loading;
            crate::config::save_from(rt);
        }
        // As the game's Delete key does: flip the flag.
        Nav::Left | Nav::Right | Nav::Select if item == Item::FrameTime => rt.mem.w32(FRAME_TIME, (rt.mem.r32(FRAME_TIME) == 0) as u32),
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
            Item::HighScores if in_screen => {
                show_high_scores(rt);
                rt.overlay.open = false;
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
                o.sel = o.items().iter().position(|&i| i == Item::Controls).unwrap_or(0);
            }
            _ => {}
        },
        _ => {}
    }
}

/// Leave the current screen for the high scores, as the game leaves a screen
/// (main loop 0x40bc4d): the next screen's config name in NEXT_SCREEN, the fade-out
/// state with its three fade timers at 5, and its start time. Gameplay is a screen
/// too, so a game in progress is torn down by the game's own transition (and, as
/// when quitting it, its score is not recorded).
fn show_high_scores(rt: &mut Runtime) {
    for (i, &b) in b"highscores.xml\0".iter().enumerate() {
        rt.mem.w8(NEXT_SCREEN + i as u32, b);
    }
    rt.mem.w32(STATE, STATE_FADE_OUT);
    for a in FADE_TIMERS {
        rt.mem.w32(a, 5.0f32.to_bits());
    }
    let now = rt.mem.r32(TICK_NOW) as f32;
    rt.mem.w32(FADE_START, now.to_bits());
}

/// Skip the loading screen (config `[display] skip_loading`), after every flip.
///
/// The game moves between screens with its camera: leaving a screen it raises the
/// camera from height 0 to 16 over a 2000-tick fade (about 2 s, while the music
/// fades), so the old screen slides out of view by about height 11 and a
/// "LOADING" sign placed above it comes into view from about 13. At 16 it loads
/// the next screen with the sign in view, then lowers the camera back to 0 by 0.2
/// a frame, so the new screen slides in from above, past the sign. The runtime
/// instead ends the fade at height 11.6 (1450 ticks: the old screen is gone), does
/// not present the frames at 16 (`hide_loading_frame`), and starts the new screen's
/// camera at 12.0 (past the sign, before any of the screen is in view), so it
/// slides in without passing the sign. Game time is not skipped: the new screen
/// then matches the original at every camera height, and the stars stay smooth.
/// Not in scripted runs.
fn skip_loading(rt: &mut Runtime) {
    if !rt.display.skip_loading || rt.user.ignore_host_input {
        return;
    }
    let f = |a: u32| f32::from_bits(rt.mem.r32(a));
    let now = rt.mem.r32(TICK_NOW) as f32;
    match rt.mem.r32(STATE) {
        STATE_FADE_OUT if (SKIP_AT..2001.0).contains(&(now - f(FADE_START))) => {
            rt.mem.w32(FADE_START, (now - 2001.0).to_bits());
            // The game loads once the camera has been at 16 for three frames
            // (FADE_TIMERS hold its last three heights); fill them so it loads on
            // the next frame and fewer frames go unshown.
            for a in FADE_TIMERS {
                rt.mem.w32(a, 16.0f32.to_bits());
            }
        }
        STATE_SCREEN if f(CAMERA_Y) >= 16.0 => {
            rt.mem.w32(CAMERA_Y, ENTER_FROM.to_bits());
            // Loading the screen took real time (about 0.3 s) that the original spends
            // at the sign, followed by a burst of catch-up frames; here they would be
            // on screen, so the backlog is forgiven instead.
            if let Some(c) = rt.clock.cadence.as_mut() {
                c.forgive_lag(std::time::Duration::ZERO);
            }
        }
        _ => {}
    }
}

/// Whether the frame about to be presented has the camera at the "LOADING" sign
/// (the frames the next screen loads in, and the new screen's first): with
/// `skip_loading`, the window keeps the previous frame instead.
pub(crate) fn hide_loading_frame(rt: &Runtime) -> bool {
    rt.display.skip_loading && !rt.user.ignore_host_input && f32::from_bits(rt.mem.r32(CAMERA_Y)) >= 16.0
}

/// Whether a screen is running (a menu, the high scores or gameplay), not a
/// transition between screens.
fn in_screen(rt: &Runtime) -> bool {
    rt.mem.r32(STATE) == STATE_SCREEN
}

/// Set the difficulty and the title screen's label.
fn set_difficulty(rt: &mut Runtime, d: usize) {
    rt.mem.w32(DIFFICULTY, DIFFICULTIES[d].0.to_bits());
    write_difficulty_label(rt, DIFFICULTIES[d].1);
}

/// Fix for a bug in the original: the title's "F3: DIFFICULTY - NORMAL" is only
/// updated when F3 changes the difficulty, so whenever the title is rebuilt (after
/// a game or the high scores) it says NORMAL again while the difficulty stays as
/// chosen. On the title, keep the label in step with the difficulty. Config
/// `[fixes] difficulty_label`; not in scripted runs, which reproduce the original.
pub(crate) fn fix_difficulty_label(rt: &mut Runtime) {
    if rt.fixes.difficulty_label && !rt.user.ignore_host_input && title_menu(rt) {
        write_difficulty_label(rt, DIFFICULTIES[current_difficulty(rt)].1);
    }
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

/// Resolution choices: automatic, then the 32-bit modes of the period list that fit
/// the host display.
fn resolutions() -> Vec<Option<(u32, u32)>> {
    let (pw, ph) = crate::display::query().map_or((u32::MAX, u32::MAX), |d| d.pixels());
    let mut v: Vec<(u32, u32)> = crate::ddraw::MODES.iter().filter(|m| m.2 == 32 && m.0 <= pw && m.1 <= ph).map(|m| (m.0, m.1)).collect();
    v.sort();
    std::iter::once(None).chain(v.into_iter().map(Some)).collect()
}

/// What "auto" resolves to on this display.
fn auto_resolution() -> Option<(u32, u32)> {
    crate::display::query().map(|d| {
        let m = crate::display::auto_mode(&d);
        (m.0, m.1)
    })
}

/// Fullscreen on or off, now and at the next start.
fn set_fullscreen(rt: &mut Runtime, on: bool) {
    rt.display.fullscreen = on;
    rt.user.host_fullscreen = on;
    if let Some(w) = rt.user.window.as_ref().filter(|w| !w.sdl.is_null()) {
        // SAFETY: valid window.
        unsafe { sdl3_sys::video::SDL_SetWindowFullscreen(w.sdl, on) };
    }
    crate::config::save_from(rt);
}

/// A volume as the overlay shows it: the game's keys go down to 1, not 0, which
/// is silent (the music is applied as volume − 1), so 1 shows as 0.
fn shown_volume(v: u32) -> u32 {
    if v <= 1 { 0 } else { v }
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
            Item::Music => ("MUSIC VOLUME".to_string(), format!("< {} >", shown_volume(rt.mem.r32(MUSIC_VOLUME))), true),
            Item::Sound => ("SOUND VOLUME".to_string(), format!("< {} >", shown_volume(rt.mem.r32(SOUND_VOLUME))), true),
            Item::Resolution => {
                let r = rt.display.resolution.map_or("AUTO".to_string(), |(rw, rh)| format!("{rw}X{rh}"));
                // Takes effect at the next start: mark a choice that differs from now.
                let now = rt.display.resolution.unwrap_or_else(|| auto_resolution().unwrap_or((w, h)));
                let mark = if now != (w, h) { " *" } else { "" };
                ("RESOLUTION".to_string(), format!("< {r} >{mark}"), true)
            }
            Item::Fullscreen => ("FULLSCREEN".to_string(), format!("< {} >", if rt.display.fullscreen { "ON" } else { "OFF" }), true),
            Item::Logo => ("LOGO".to_string(), format!("< {} >", if rt.display.gold_logo { "GOLD" } else { "BUGATRON" }), true),
            Item::SkipLoading => ("SKIP LOADING SCREEN".to_string(), format!("< {} >", if rt.display.skip_loading { "ON" } else { "OFF" }), true),
            Item::FrameTime => ("SHOW FPS".to_string(), format!("< {} >", if rt.mem.r32(FRAME_TIME) != 0 { "ON" } else { "OFF" }), true),
            Item::HighScores => ("HIGH SCORES".to_string(), String::new(), in_screen(rt)),
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
        (Page::Main, _) if o.items().get(o.sel) == Some(&Item::Resolution) => "* AFTER A RESTART",
        (Page::Main, _) if o.items().get(o.sel) == Some(&Item::Logo) => "SHOWN THE NEXT TIME THE TITLE SCREEN LOADS",
        (Page::Main, _) => "ENTER/A: SELECT   ESC/B: CLOSE",
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
