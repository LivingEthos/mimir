//! Budget ledger panel.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Paragraph},
    Frame,
};

use crate::App;

/// Budget ledger panel.
pub struct BudgetPanel;

impl BudgetPanel {
    /// Draw the budget panel.
    pub fn draw(f: &mut Frame, area: Rect, app: &App, is_focused: bool) {
        let block = if is_focused {
            Block::default()
                .title("[*] Budget Ledger")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title("Budget Ledger")
                .borders(Borders::ALL)
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let text = if let Some(ref budget) = app.budget {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Run: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(&budget.run_id),
                ]),
                Line::from(vec![
                    Span::styled("Total cap: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(budget.total_tokens.to_string()),
                ]),
            ];
            for cat in &budget.categories {
                let pct = if budget.total_tokens > 0 {
                    (f64::from(cat.tokens) / f64::from(budget.total_tokens)) * 100.0
                } else {
                    0.0
                };
                lines.push(Line::from(vec![
                    Span::raw(format!("  {:20} ", cat.name)),
                    Span::styled(
                        format!("{:>8} tok", cat.tokens),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(" ({:.1}%)", pct)),
                ]));
            }
            lines
        } else {
            vec![Line::from("No budget loaded")]
        };

        let paragraph = Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(paragraph, inner);
    }
}
