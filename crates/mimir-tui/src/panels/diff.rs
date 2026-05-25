//! Diff/review panel.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Diff/review panel.
pub struct DiffPanel;

impl DiffPanel {
    /// Draw the diff panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Diff / Review")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title("Diff / Review")
                .borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let text: Vec<Line> = if app.diff_lines.is_empty() {
            vec![Line::from("No diff loaded")]
        } else {
            app.diff_lines
                .iter()
                .map(|line| {
                    if line.starts_with('+') {
                        Line::from(vec![Span::styled(line, Style::default().fg(Color::Green))])
                    } else if line.starts_with('-') {
                        Line::from(vec![Span::styled(line, Style::default().fg(Color::Red))])
                    } else if line.starts_with("@@") {
                        Line::from(vec![Span::styled(line, Style::default().fg(Color::Cyan))])
                    } else {
                        Line::from(line.as_str())
                    }
                })
                .collect()
        };

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
