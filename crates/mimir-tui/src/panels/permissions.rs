//! Permissions panel.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Permissions panel.
pub struct PermissionsPanel;

impl PermissionsPanel {
    /// Draw the permissions panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Permissions")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default().title("Permissions").borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let perms = &app.permissions;
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Edit: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    if perms.can_edit { "YES" } else { "NO" },
                    if perms.can_edit {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Red)
                    },
                ),
            ]),
            Line::from(vec![
                Span::styled("Run:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    if perms.can_run { "YES" } else { "NO" },
                    if perms.can_run {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Red)
                    },
                ),
            ]),
            Line::from(vec![
                Span::styled("Net:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    if perms.can_network { "YES" } else { "NO" },
                    if perms.can_network {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Red)
                    },
                ),
            ]),
        ];

        if !perms.allowed_paths.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "Allowed paths:",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            for p in &perms.allowed_paths {
                lines.push(Line::from(vec![
                    Span::styled("  + ", Style::default().fg(Color::Green)),
                    Span::raw(p),
                ]));
            }
        }

        if !perms.blocked_paths.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "Blocked paths:",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            for p in &perms.blocked_paths {
                lines.push(Line::from(vec![
                    Span::styled("  - ", Style::default().fg(Color::Red)),
                    Span::raw(p),
                ]));
            }
        }

        let paragraph = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
