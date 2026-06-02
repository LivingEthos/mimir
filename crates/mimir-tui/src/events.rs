//! Input event handling for the Mimir TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tracing::{debug, info};

use crate::App;

/// Handle a crossterm key event.
pub fn handle_key_event(key: KeyEvent, app: &mut App) {
    match key.code {
        // Ctrl-C exits cleanly
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            info!("Ctrl-C pressed — exiting cleanly");
            app.should_quit = true;
        }
        // ESC then q is explicit quit
        KeyCode::Char('q') if app.awaiting_quit_confirm => {
            info!("q after ESC — quitting");
            app.should_quit = true;
        }
        KeyCode::Esc => {
            if app.awaiting_quit_confirm {
                app.awaiting_quit_confirm = false;
                app.status = "Quit cancelled".to_string();
            } else {
                app.awaiting_quit_confirm = true;
                app.status = "Press q to quit, any other key to cancel".to_string();
            }
        }
        KeyCode::Char('q') if !app.awaiting_quit_confirm => {
            // swallow stray q
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.next_panel();
            app.status = format!("Focused panel: {}", panel_name(app.focused_panel));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.prev_panel();
            app.status = format!("Focused panel: {}", panel_name(app.focused_panel));
        }
        KeyCode::Char('1') => {
            app.focused_panel = 0;
            app.status = "Focused panel: Budget".to_string();
        }
        KeyCode::Char('2') => {
            app.focused_panel = 1;
            app.status = "Focused panel: Included".to_string();
        }
        KeyCode::Char('3') => {
            app.focused_panel = 2;
            app.status = "Focused panel: Omitted".to_string();
        }
        KeyCode::Char('4') => {
            app.focused_panel = 3;
            app.status = "Focused panel: Provider".to_string();
        }
        KeyCode::Char('5') => {
            app.focused_panel = 4;
            app.status = "Focused panel: Permissions".to_string();
        }
        KeyCode::Char('6') => {
            app.focused_panel = 5;
            app.status = "Focused panel: Diff".to_string();
        }
        KeyCode::Char('r') => {
            app.request_live_refresh();
        }
        _ => {
            debug!("Unhandled key: {:?}", key);
            if app.awaiting_quit_confirm {
                app.awaiting_quit_confirm = false;
                app.status = "Quit cancelled".to_string();
            }
        }
    }
}

fn panel_name(idx: usize) -> &'static str {
    match idx {
        0 => "Budget",
        1 => "Included",
        2 => "Omitted",
        3 => "Provider",
        4 => "Permissions",
        5 => "Diff",
        _ => "Unknown",
    }
}
