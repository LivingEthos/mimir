//! Omitted candidates panel.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Omitted candidates panel.
pub struct OmittedPanel;

impl OmittedPanel {
    /// Draw the omitted candidates panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Omitted Candidates")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title("Omitted Candidates")
                .borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let text = if let Some(ref result) = app.pipeline_result {
            if result.manifest.omitted.is_empty() {
                vec![Line::from("No omitted items")]
            } else {
                let mut lines = vec![Line::from(vec![Span::styled(
                    format!("Omitted: {}", result.manifest.omitted.len()),
                    Style::default().add_modifier(Modifier::BOLD),
                )])];
                for item in &result.manifest.omitted {
                    let risk_color = match item.risk.as_deref() {
                        Some("high") => Color::Red,
                        Some("medium") => Color::Yellow,
                        _ => Color::Gray,
                    };
                    lines.push(Line::from(vec![
                        Span::styled("• ", Style::default().fg(Color::Red)),
                        Span::raw(item.path.to_string()),
                        Span::raw(format!("  ({})  ", item.reason)),
                        Span::styled(
                            format!("{} tok", item.estimated_tokens),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            item.risk.clone().unwrap_or_else(|| "low".to_string()),
                            Style::default().fg(risk_color),
                        ),
                    ]));
                }
                lines
            }
        } else {
            vec![Line::from("No pipeline result loaded")]
        };

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
