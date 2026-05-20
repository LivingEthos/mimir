//! `.mimir/skills/` — path-gated subagent skills.
//!
//! Skills are reusable subagent configurations gated by file path patterns.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::fs;

/// A skill definition loaded from `.mimir/skills/*.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Path patterns that activate this skill (glob).
    pub path_gates: Vec<String>,
    /// Subagent to use.
    pub subagent: String,
    /// Additional context to inject.
    pub context: String,
    /// Token cap override.
    pub token_cap: Option<u32>,
}

/// Load all skills from `.mimir/skills/`.
pub fn load_skills(base: &Utf8Path) -> Vec<Skill> {
    let skills_dir = base.join(".mimir/skills");
    let mut skills = Vec::new();

    let entries = match fs::read_dir(&skills_dir) {
        Ok(e) => e,
        Err(_) => return skills,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(skill) = serde_yaml::from_str::<Skill>(&content) {
                    skills.push(skill);
                }
            }
        }
    }

    skills
}

/// Check if a skill is active for a given file path.
pub fn skill_applies(skill: &Skill, file_path: &str) -> bool {
    for gate in &skill.path_gates {
        if glob_match(gate, file_path) {
            return true;
        }
    }
    false
}

/// Simple glob matching (* = any chars, ** = any path segments).
fn glob_match(pattern: &str, path: &str) -> bool {
    let regex_pattern = pattern
        .replace(".", r"\.")
        .replace("**/", "__DOUBLESTAR__")
        .replace("*", "[^/]*")
        .replace("__DOUBLESTAR__", ".*");
    let re = match regex::Regex::new(&format!("^{}$", regex_pattern)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    re.is_match(path)
}

/// Get all skills that apply to a file.
pub fn active_skills<'a>(skills: &'a [Skill], file_path: &str) -> Vec<&'a Skill> {
    skills
        .iter()
        .filter(|s| skill_applies(s, file_path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/lib/mod.rs"));
        assert!(glob_match("src/**/*.rs", "src/lib/mod.rs"));
        assert!(glob_match("*.md", "README.md"));
    }

    #[test]
    fn test_skill_applies() {
        let skill = Skill {
            name: "rust-analysis".into(),
            description: "Rust-specific analysis".into(),
            path_gates: vec!["src/**/*.rs".into(), "crates/**/*.rs".into()],
            subagent: "file-analyst".into(),
            context: "Look for unsafe blocks".into(),
            token_cap: Some(4000),
        };
        assert!(skill_applies(&skill, "src/main.rs"));
        assert!(skill_applies(&skill, "crates/foo/src/lib.rs"));
        assert!(!skill_applies(&skill, "README.md"));
    }

    #[test]
    fn test_active_skills() {
        let skills = vec![
            Skill {
                name: "rust".into(),
                description: "d".into(),
                path_gates: vec!["*.rs".into()],
                subagent: "file-analyst".into(),
                context: "".into(),
                token_cap: None,
            },
            Skill {
                name: "js".into(),
                description: "d".into(),
                path_gates: vec!["*.js".into()],
                subagent: "file-analyst".into(),
                context: "".into(),
                token_cap: None,
            },
        ];
        let active = active_skills(&skills, "main.rs");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "rust");
    }
}
