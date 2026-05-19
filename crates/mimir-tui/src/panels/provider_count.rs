//! Provider count panel.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Provider count panel.
pub struct ProviderCountPanel;

impl ProviderCountPanel {
    /// Draw the provider count panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Provider Counts")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title("Provider Counts")
                .borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let text = if app.provider_counts.is_empty() {
            vec![Line::from("No provider data loaded")]
        } else {
            let mut lines = vec![Line::from(vec![Span::styled(
                format!("Providers: {}", app.provider_counts.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )])];
            for count in &app.provider_counts {
                lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Blue)),
                    Span::raw(format!("{}/{}", count.name, count.model)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("   input: "),
                    Span::styled(
                        count.input_tokens.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  output: "),
                    Span::styled(
                        count.output_tokens.to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                if count.cache_creation_tokens > 0 || count.cache_read_tokens > 0 {
                    lines.push(Line::from(vec![
                        Span::raw("   cache write: "),
                        Span::styled(
                            count.cache_creation_tokens.to_string(),
                            Style::default().fg(Color::Green),
                        ),
                        Span::raw("  cache read: "),
                        Span::styled(
                            count.cache_read_tokens.to_string(),
                            Style::default().fg(Color::Green),
                        ),
                    ]));
                }
            }
            lines
        };

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
