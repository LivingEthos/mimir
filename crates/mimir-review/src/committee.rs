//! Committee reviewer: deterministic deduplicated findings.
//!
//! Multiple reviewers (subagents or checks) produce findings;
//! the committee deduplicates and ranks them.

use std::collections::{HashMap, HashSet};

use crate::{Finding, ReviewReport};

/// A committee of reviewers producing a unified report.
pub struct Committee {
    /// Reviewer name -> their report.
    reports: HashMap<String, ReviewReport>,
    /// Deduplication key function.
    dedup_key: fn(&Finding) -> String,
}

impl Default for Committee {
    fn default() -> Self {
        Self {
            reports: HashMap::new(),
            dedup_key: default_dedup_key,
        }
    }
}

impl Committee {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a reviewer's report.
    pub fn add_report(&mut self, reviewer: impl Into<String>, report: ReviewReport) {
        self.reports.insert(reviewer.into(), report);
    }

    /// Produce a unified, deduplicated report.
    pub fn unify(&self) -> ReviewReport {
        let mut unified = ReviewReport::new("committee");
        let mut seen: HashSet<String> = HashSet::new();

        // Collect all findings, deduplicate by key
        for (reviewer, report) in &self.reports {
            for finding in &report.findings {
                let key = format!("{}:{}", reviewer, (self.dedup_key)(finding));
                if seen.insert(key.clone()) {
                    unified.add(finding.clone());
                }
            }
        }

        unified
    }

    /// Produce a unified report grouped by category.
    pub fn unify_by_category(&self) -> HashMap<String, Vec<Finding>> {
        let mut grouped: HashMap<String, Vec<Finding>> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (reviewer, report) in &self.reports {
            for finding in &report.findings {
                let key = format!("{}:{}", reviewer, (self.dedup_key)(finding));
                if seen.insert(key) {
                    grouped
                        .entry(finding.category.clone())
                        .or_default()
                        .push(finding.clone());
                }
            }
        }

        grouped
    }

    /// Print findings as a structured table.
    pub fn print_findings(&self) {
        let unified = self.unify();
        println!("┌─────────────────────────────────────────────────────────────────────┐");
        println!(
            "│ Committee Review Findings ({})                                     │",
            unified.findings.len()
        );
        println!("├──────────────────────┬──────────┬───────────────────────────────────┤");
        println!("│ Category             │ Severity │ Description                       │");
        println!("├──────────────────────┼──────────┼───────────────────────────────────┤");
        for f in &unified.findings {
            println!(
                "│ {:20} │ {:8} │ {:33} │",
                &f.category[..f.category.len().min(20)],
                &f.severity,
                &f.description[..f.description.len().min(33)]
            );
        }
        println!("└──────────────────────┴──────────┴───────────────────────────────────┘");
        println!("Passed: {}", unified.passed);
    }
}

fn default_dedup_key(finding: &Finding) -> String {
    format!(
        "{}:{}:{}",
        finding.category,
        finding.paths.join(","),
        finding.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_committee_unify() {
        let mut c = Committee::new();

        let mut r1 = ReviewReport::new("reviewer-a");
        r1.add(Finding {
            category: "style".into(),
            paths: vec!["src/main.rs".into()],
            description: "trailing whitespace".into(),
            severity: "info".into(),
            line_numbers: None,
        });

        let mut r2 = ReviewReport::new("reviewer-b");
        r2.add(Finding {
            category: "style".into(),
            paths: vec!["src/main.rs".into()],
            description: "trailing whitespace".into(),
            severity: "info".into(),
            line_numbers: None,
        });
        r2.add(Finding {
            category: "bug".into(),
            paths: vec!["src/lib.rs".into()],
            description: "null pointer".into(),
            severity: "error".into(),
            line_numbers: None,
        });

        c.add_report("a", r1);
        c.add_report("b", r2);

        let unified = c.unify();
        // trailing whitespace appears in both but deduped per-reviewer
        // so we get: a:style + b:style + b:bug = 3
        assert_eq!(unified.findings.len(), 3);
        assert!(!unified.passed);
    }

    #[test]
    fn test_committee_unify_by_category() {
        let mut c = Committee::new();
        let mut r = ReviewReport::new("r");
        r.add(Finding {
            category: "style".into(),
            paths: vec!["a.rs".into()],
            description: "ws".into(),
            severity: "info".into(),
            line_numbers: None,
        });
        r.add(Finding {
            category: "bug".into(),
            paths: vec!["b.rs".into()],
            description: "panic".into(),
            severity: "error".into(),
            line_numbers: None,
        });
        c.add_report("r", r);

        let grouped = c.unify_by_category();
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped.get("style").unwrap().len(), 1);
        assert_eq!(grouped.get("bug").unwrap().len(), 1);
    }
}
