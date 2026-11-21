//! Interactive terminal viewer for `mydiff`.
//!
//! Architecture (Elm style, every piece pure except the `run_*` functions):
//! terminal event → [`keys::KeyMap::action_for`] → [`app::App::update`] → [`view::render`].
//!
//! The `run_*` functions take the sidebar state to start with and return the
//! one the user left it in, so the caller can remember it between runs.

pub mod app;
pub mod cmdline;
pub mod folder;
pub mod hex;
pub mod keys;
pub mod search;
pub mod view;

use std::io;

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

pub use app::{App, Dock, Sidebar, TextSession};
pub use folder::{FolderApp, FolderSession};
pub use hex::{HexApp, HexSession};
use ratatui::Frame;

/// Something the event loop can drive.
trait Screen {
    fn update(&mut self, action: app::Action);
    fn draw(&self, frame: &mut Frame);
    fn should_quit(&self) -> bool;
    fn input_mode(&self) -> keys::InputMode;
    fn sidebar(&self) -> Sidebar;
    fn set_sidebar(&mut self, sidebar: Sidebar);
}

impl Screen for App {
    fn update(&mut self, action: app::Action) {
        App::update(self, action);
    }
    fn draw(&self, frame: &mut Frame) {
        view::render(self, frame);
    }
    fn should_quit(&self) -> bool {
        self.should_quit
    }
    fn input_mode(&self) -> keys::InputMode {
        App::input_mode(self)
    }
    fn sidebar(&self) -> Sidebar {
        self.sidebar
    }
    fn set_sidebar(&mut self, sidebar: Sidebar) {
        self.sidebar = sidebar;
    }
}

impl Screen for HexApp {
    fn update(&mut self, action: app::Action) {
        HexApp::update(self, action);
    }
    fn draw(&self, frame: &mut Frame) {
        hex::render(self, frame);
    }
    fn should_quit(&self) -> bool {
        self.should_quit
    }
    fn input_mode(&self) -> keys::InputMode {
        HexApp::input_mode(self)
    }
    fn sidebar(&self) -> Sidebar {
        self.sidebar
    }
    fn set_sidebar(&mut self, sidebar: Sidebar) {
        self.sidebar = sidebar;
    }
}

impl Screen for FolderApp {
    fn update(&mut self, action: app::Action) {
        FolderApp::update(self, action);
    }
    fn draw(&self, frame: &mut Frame) {
        folder::render(self, frame);
    }
    fn should_quit(&self) -> bool {
        FolderApp::should_quit(self)
    }
    fn input_mode(&self) -> keys::InputMode {
        FolderApp::input_mode(self)
    }
    fn sidebar(&self) -> Sidebar {
        FolderApp::current_sidebar(self)
    }
    fn set_sidebar(&mut self, sidebar: Sidebar) {
        self.sidebar = sidebar;
    }
}

/// Run the interactive text viewer until the user quits; returns the sidebar
/// state it ended with.
pub fn run_text(session: TextSession, sidebar: Sidebar) -> io::Result<Sidebar> {
    let mut app = App::new(session).map_err(io::Error::other)?;
    run(&mut app, sidebar)
}

/// Run the interactive folder viewer until the user quits; returns the sidebar
/// state it ended with.
pub fn run_folder(session: FolderSession, sidebar: Sidebar) -> io::Result<Sidebar> {
    let mut app = FolderApp::new(session).map_err(io::Error::other)?;
    run(&mut app, sidebar)
}

/// Run the interactive hex viewer until the user quits; returns the sidebar
/// state it ended with.
pub fn run_hex(session: HexSession, sidebar: Sidebar) -> io::Result<Sidebar> {
    run(&mut HexApp::new(session), sidebar)
}

fn run(screen: &mut impl Screen, sidebar: Sidebar) -> io::Result<Sidebar> {
    screen.set_sidebar(sidebar);
    let mut terminal = ratatui::init();
    // Mouse capture enables the scroll wheel; hold Shift to select text in most terminals.
    let _ = execute!(io::stdout(), EnableMouseCapture);
    let mut keymap = keys::KeyMap::default();
    let result = (|| -> io::Result<()> {
        loop {
            let size = terminal.size()?;
            screen.update(app::Action::Resize {
                width: size.width,
                height: size.height,
            });
            terminal.draw(|f| screen.draw(f))?;
            if let Some(action) = keymap.action_for(&event::read()?, screen.input_mode()) {
                screen.update(action);
            }
            if screen.should_quit() {
                return Ok(());
            }
        }
    })();
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result.map(|()| screen.sidebar())
}
