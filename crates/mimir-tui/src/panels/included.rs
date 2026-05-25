//! Included ranges panel.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Included ranges panel.
pub struct IncludedPanel;

impl IncludedPanel {
    /// Build display lines for included context.
    #[must_use]
    pub fn lines(app: &App) -> Vec<Line<'static>> {
        if let Some(ref result) = app.pipeline_result {
            if result.manifest.included.is_empty() {
                return vec![Line::from("No included items")];
            }

            let mut lines = vec![Line::from(vec![Span::styled(
                format!(
                    "Items: {} | Tokens: {}",
                    result.manifest.included.len(),
                    result.manifest.total_tokens
                ),
                Style::default().add_modifier(Modifier::BOLD),
            )])];
            for item in &result.manifest.included {
                let range_str = item
                    .ranges
                    .iter()
                    .map(|r| format!("{}-{}", r.start, r.end))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Green)),
                    Span::raw(item.path.to_string()),
                    Span::raw(format!("  [{}]  ", range_str)),
                    Span::styled(
                        format!("{} tok", item.estimated_tokens),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
            }
            return lines;
        }

        if let Some(ref packet) = app.packet {
            if packet.included.is_empty() {
                return vec![Line::from("No included items")];
            }

            let total_tokens = packet.included.iter().map(|item| item.tokens).sum::<u32>();
            let mut lines = vec![Line::from(vec![Span::styled(
                format!(
                    "Items: {} | Tokens: {}",
                    packet.included.len(),
                    total_tokens
                ),
                Style::default().add_modifier(Modifier::BOLD),
            )])];
            for item in &packet.included {
                let range_str = item
                    .ranges
                    .iter()
                    .map(|r| format!("{}-{}", r.start, r.end))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Green)),
                    Span::raw(item.path.to_string()),
                    Span::raw(format!("  [{}]  ", range_str)),
                    Span::styled(
                        format!("{} tok", item.tokens),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
            }
            return lines;
        }

        vec![Line::from("No context packet or pipeline result loaded")]
    }

    /// Draw the included ranges panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Included Ranges")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title("Included Ranges")
                .borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let text = Self::lines(app);

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
