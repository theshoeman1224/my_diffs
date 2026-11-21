//! Interactive terminal viewer for `mydiff`.
//!
//! Architecture (Elm style, every piece pure except [`run_text`]):
//! terminal event → [`keys::KeyMap::action_for`] → [`app::App::update`] → [`view::render`].

pub mod app;
pub mod folder;
pub mod hex;
pub mod keys;
pub mod view;

use std::io;

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

pub use app::{App, TextSession};
pub use folder::{FolderApp, FolderSession};
pub use hex::{HexApp, HexSession};
use ratatui::Frame;

/// Something the event loop can drive.
trait Screen {
    fn update(&mut self, action: app::Action);
    fn draw(&self, frame: &mut Frame);
    fn should_quit(&self) -> bool;
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
}

/// Run the interactive text viewer until the user quits.
pub fn run_text(session: TextSession) -> io::Result<()> {
    let mut app = App::new(session).map_err(io::Error::other)?;
    run(&mut app)
}

/// Run the interactive folder viewer until the user quits.
pub fn run_folder(session: FolderSession) -> io::Result<()> {
    let mut app = FolderApp::new(session).map_err(io::Error::other)?;
    run(&mut app)
}

/// Run the interactive hex viewer until the user quits.
pub fn run_hex(session: HexSession) -> io::Result<()> {
    run(&mut HexApp::new(session))
}

fn run(screen: &mut impl Screen) -> io::Result<()> {
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
            if let Some(action) = keymap.action_for(&event::read()?) {
                screen.update(action);
            }
            if screen.should_quit() {
                return Ok(());
            }
        }
    })();
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
