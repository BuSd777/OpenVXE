mod app;
mod ui;

use anyhow::Context;
use app::App;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hidapi::HidApi;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use std::{io, time::Duration};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
}

fn main() -> anyhow::Result<()> {
    // Initialize HID before touching the terminal, so errors are printed normally.
    let api = HidApi::new().context("cannot initialize HID")?;
    let mut app = App::new(api);

    // Restore the terminal on panic, otherwise it stays in raw mode / alternate screen.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode().context("cannot enable raw mode")?;
    let res = execute!(io::stdout(), EnterAlternateScreen)
        .and_then(|()| Terminal::new(CrosstermBackend::new(io::stdout())))
        .and_then(|mut terminal| run_app(&mut terminal, &mut app));
    restore_terminal();
    res.context("terminal error")
}

fn run_app<B: Backend<Error = io::Error>>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_device();

        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                // Ignore release/repeat events (sent on Windows and with the kitty protocol).
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        if app.request_quit() {
                            return Ok(());
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                    KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                    KeyCode::Left | KeyCode::Char('h') => app.modify_setting(-1),
                    KeyCode::Right | KeyCode::Char('l') => app.modify_setting(1),
                    _ => {}
                }
            }
        }
    }
}
