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
    /// Build display lines for omitted context.
    #[must_use]
    pub fn lines(app: &App) -> Vec<Line<'static>> {
        if let Some(ref result) = app.pipeline_result {
            if result.manifest.omitted.is_empty() {
                return vec![Line::from("No omitted items")];
            }

            let mut lines = vec![Line::from(vec![Span::styled(
                format!("Omitted: {}", result.manifest.omitted.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )])];
            for item in &result.manifest.omitted {
                let risk_color = risk_color(item.risk.as_deref());
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
            return lines;
        }

        if let Some(ref packet) = app.packet {
            if packet.omitted_candidates.is_empty() {
                return vec![Line::from("No omitted items")];
            }

            let mut lines = vec![Line::from(vec![Span::styled(
                format!("Omitted: {}", packet.omitted_candidates.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )])];
            for item in &packet.omitted_candidates {
                let risk = item.risk.clone().unwrap_or_else(|| "low".to_string());
                let risk_color = risk_color(Some(&risk));
                lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Red)),
                    Span::raw(item.path.to_string()),
                    Span::raw(format!("  ({})  ", item.reason_for_omission)),
                    Span::styled(
                        format!("{} tok", item.estimated_tokens),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  "),
                    Span::styled(risk, Style::default().fg(risk_color)),
                ]));
            }
            return lines;
        }

        vec![Line::from("No context packet or pipeline result loaded")]
    }

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

        let text = Self::lines(app);

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}

fn risk_color(risk: Option<&str>) -> Color {
    match risk {
        Some("high") => Color::Red,
        Some("medium") => Color::Yellow,
        _ => Color::Gray,
    }
}
