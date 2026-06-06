//! Patch application engine.

use std::{collections::BTreeMap, collections::BTreeSet, fs, io::ErrorKind};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use mimir_schemas::PatchStep;

use crate::{backup::backup_name_for_path, EditError, EditableSet, Result};

/// Engine for applying patches.
pub struct PatchEngine;

#[derive(Debug, Clone)]
struct SafePath {
    requested: String,
    normalized: String,
    full_path: Utf8PathBuf,
}

/// Transactional patch application with rollback on later step failure.
pub struct PatchTransaction<'a> {
    base_path: &'a Utf8Path,
    canonical_base: Utf8PathBuf,
    editable: &'a EditableSet,
    backup_dir: Utf8PathBuf,
    rollback_actions: Vec<RollbackAction>,
    created_dirs: Vec<Utf8PathBuf>,
    backed_up: BTreeMap<String, Utf8PathBuf>,
    created_files: BTreeSet<String>,
    prepared: bool,
}

#[derive(Debug, Clone)]
enum RollbackAction {
    RestoreFile {
        requested: String,
        normalized: String,
        path: Utf8PathBuf,
        backup: Utf8PathBuf,
    },
    RemoveCreatedFile {
        requested: String,
        normalized: String,
        path: Utf8PathBuf,
    },
    MoveBack {
        from_requested: String,
        from_normalized: String,
        from_path: Utf8PathBuf,
        to_requested: String,
        to_normalized: String,
        to_path: Utf8PathBuf,
        backup: Utf8PathBuf,
    },
}

impl<'a> PatchTransaction<'a> {
    /// Create a transaction rooted at `base_path`.
    pub fn new(
        base_path: &'a Utf8Path,
        editable: &'a EditableSet,
        backup_dir: impl Into<Utf8PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            base_path,
            canonical_base: canonicalize_utf8(base_path, "base path")?,
            editable,
            backup_dir: backup_dir.into(),
            rollback_actions: Vec::new(),
            created_dirs: Vec::new(),
            backed_up: BTreeMap::new(),
            created_files: BTreeSet::new(),
            prepared: false,
        })
    }

    /// Preflight every step and create backups before target files are mutated.
    pub fn prepare(&mut self, steps: &[PatchStep]) -> Result<()> {
        for step in steps {
            self.preflight_step(step)?;
        }

        self.rollback_actions.clear();
        self.created_dirs.clear();
        self.backed_up.clear();
        self.created_files.clear();

        let mut created_dir_set = BTreeSet::new();
        for step in steps {
            self.record_rollback(step, &mut created_dir_set)?;
        }
        self.prepared = true;
        Ok(())
    }

    /// Apply all steps and roll back already-applied steps if a later step fails.
    pub fn apply_all(&mut self, steps: &[PatchStep]) -> Result<()> {
        if !self.prepared {
            self.prepare(steps)?;
        }

        for step in steps {
            if let Err(apply_error) = PatchEngine::apply(step, self.base_path, self.editable) {
                if let Err(rollback_error) = self.rollback() {
                    return Err(EditError::BackupFailed {
                        path: "transaction".to_string(),
                        reason: format!(
                            "apply failed: {apply_error}; rollback failed: {rollback_error}"
                        ),
                    });
                }
                return Err(apply_error);
            }
        }

        Ok(())
    }

    /// Roll back target files and remove empty directories created by this transaction.
    pub fn rollback(&mut self) -> Result<()> {
        let mut errors = Vec::new();

        for action in self.rollback_actions.iter().rev() {
            if let Err(error) = self.rollback_action(action) {
                errors.push(error.to_string());
            }
        }

        for dir in self.created_dirs.iter().rev() {
            match fs::remove_dir(dir) {
                Ok(()) => {}
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(err) => errors.push(format!("remove dir {dir}: {err}")),
            }
        }

        if errors.is_empty() {
            self.cleanup_backup_dir()
        } else {
            Err(EditError::BackupFailed {
                path: "transaction".to_string(),
                reason: errors.join("; "),
            })
        }
    }

    /// Mark the transaction complete and remove raw backup artifacts.
    pub fn commit(mut self) -> Result<()> {
        self.rollback_actions.clear();
        self.created_dirs.clear();
        self.prepared = false;
        self.cleanup_backup_dir()
    }

    fn preflight_step(&self, step: &PatchStep) -> Result<()> {
        match step {
            PatchStep::LineRange { path, .. } | PatchStep::UnifiedDiff { path, .. } => {
                let target = self.resolve_target(path)?;
                ensure_regular_existing_file(&target, "edit")
            }
            PatchStep::WholeFile { path, .. } => {
                let target = self.resolve_target(path)?;
                ensure_no_symlink_components(
                    self.base_path,
                    &target.normalized,
                    &target.requested,
                )?;
                if target.full_path.exists() {
                    ensure_regular_existing_file(&target, "edit")?;
                }
                Ok(())
            }
            PatchStep::Create { path, .. } => {
                let target = self.resolve_target(path)?;
                ensure_missing_target(&target.full_path, &target.requested, "create")?;
                ensure_no_symlink_components(self.base_path, &target.normalized, &target.requested)
            }
            PatchStep::Delete { path } => {
                let target = self.resolve_target(path)?;
                ensure_regular_existing_file(&target, "delete")
            }
            PatchStep::Move { from, to } => {
                let from_path = self.resolve_target(from)?;
                let to_path = self.resolve_target(to)?;
                ensure_regular_existing_file(&from_path, "move")?;
                ensure_missing_target(&to_path.full_path, &to_path.requested, "move")?;
                ensure_no_symlink_components(
                    self.base_path,
                    &to_path.normalized,
                    &to_path.requested,
                )
            }
        }
    }

    fn record_rollback(
        &mut self,
        step: &PatchStep,
        created_dir_set: &mut BTreeSet<String>,
    ) -> Result<()> {
        match step {
            PatchStep::LineRange { path, .. } | PatchStep::UnifiedDiff { path, .. } => {
                let target = self.resolve_target(path)?;
                self.record_restore(&target)
            }
            PatchStep::WholeFile { path, .. } => {
                let target = self.resolve_target(path)?;
                if target.full_path.exists() {
                    self.record_restore(&target)
                } else {
                    self.record_created_file(&target, created_dir_set)
                }
            }
            PatchStep::Create { path, .. } => {
                let target = self.resolve_target(path)?;
                self.record_created_file(&target, created_dir_set)
            }
            PatchStep::Delete { path } => {
                let target = self.resolve_target(path)?;
                self.record_restore(&target)
            }
            PatchStep::Move { from, to } => {
                let from_path = self.resolve_target(from)?;
                let to_path = self.resolve_target(to)?;
                self.record_move_back(&from_path, &to_path, created_dir_set)
            }
        }
    }

    fn resolve_target(&self, path: &str) -> Result<SafePath> {
        PatchEngine::resolve_target(self.base_path, &self.canonical_base, path, self.editable)
    }

    fn record_restore(&mut self, target: &SafePath) -> Result<()> {
        if self.created_files.contains(&target.normalized)
            || self.backed_up.contains_key(&target.normalized)
        {
            return Ok(());
        }

        let backup = self.backup_existing(target)?;
        self.rollback_actions.push(RollbackAction::RestoreFile {
            requested: target.requested.clone(),
            normalized: target.normalized.clone(),
            path: target.full_path.clone(),
            backup,
        });
        Ok(())
    }

    fn record_created_file(
        &mut self,
        target: &SafePath,
        created_dir_set: &mut BTreeSet<String>,
    ) -> Result<()> {
        if self.created_files.insert(target.normalized.clone()) {
            self.track_created_dirs(target, created_dir_set)?;
            self.rollback_actions
                .push(RollbackAction::RemoveCreatedFile {
                    requested: target.requested.clone(),
                    normalized: target.normalized.clone(),
                    path: target.full_path.clone(),
                });
        }
        Ok(())
    }

    fn record_move_back(
        &mut self,
        from: &SafePath,
        to: &SafePath,
        created_dir_set: &mut BTreeSet<String>,
    ) -> Result<()> {
        self.track_created_dirs(to, created_dir_set)?;
        let backup = self.backup_existing(from)?;
        self.rollback_actions.push(RollbackAction::MoveBack {
            from_requested: from.requested.clone(),
            from_normalized: from.normalized.clone(),
            from_path: from.full_path.clone(),
            to_requested: to.requested.clone(),
            to_normalized: to.normalized.clone(),
            to_path: to.full_path.clone(),
            backup,
        });
        Ok(())
    }

    fn backup_existing(&mut self, target: &SafePath) -> Result<Utf8PathBuf> {
        if let Some(path) = self.backed_up.get(&target.normalized) {
            return Ok(path.clone());
        }

        ensure_regular_existing_file(target, "backup")?;
        let backup_root = self.base_path.join(&self.backup_dir);
        fs::create_dir_all(&backup_root).map_err(|e| EditError::BackupFailed {
            path: target.requested.clone(),
            reason: format!("create backup dir: {e}"),
        })?;
        let backup_path = backup_root.join(backup_name_for_path(&target.normalized));
        fs::copy(&target.full_path, &backup_path).map_err(|e| EditError::BackupFailed {
            path: target.requested.clone(),
            reason: format!("copy: {e}"),
        })?;
        self.backed_up
            .insert(target.normalized.clone(), backup_path.clone());
        Ok(backup_path)
    }

    fn track_created_dirs(
        &mut self,
        target: &SafePath,
        created_dir_set: &mut BTreeSet<String>,
    ) -> Result<()> {
        let parts = target.normalized.split('/').collect::<Vec<_>>();
        if parts.len() <= 1 {
            return Ok(());
        }

        let mut candidate = self.base_path.to_path_buf();
        for part in parts.iter().take(parts.len() - 1) {
            candidate.push(part);
            match fs::symlink_metadata(&candidate) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(path_not_editable(&target.requested));
                }
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    if created_dir_set.insert(candidate.to_string()) {
                        self.created_dirs.push(candidate.clone());
                    }
                }
                Err(err) => {
                    return Err(EditError::Io(format!("stat {}: {}", target.requested, err)));
                }
            }
        }

        Ok(())
    }

    fn rollback_action(&self, action: &RollbackAction) -> Result<()> {
        match action {
            RollbackAction::RestoreFile {
                requested,
                normalized,
                path,
                backup,
            } => self.restore_file(requested, normalized, path, backup),
            RollbackAction::RemoveCreatedFile {
                requested,
                normalized,
                path,
            } => self.remove_created_file(requested, normalized, path),
            RollbackAction::MoveBack {
                from_requested,
                from_normalized,
                from_path,
                to_requested,
                to_normalized,
                to_path,
                backup,
            } => {
                self.remove_created_file(to_requested, to_normalized, to_path)?;
                self.restore_file(from_requested, from_normalized, from_path, backup)
            }
        }
    }

    fn restore_file(
        &self,
        requested: &str,
        normalized: &str,
        path: &Utf8Path,
        backup: &Utf8Path,
    ) -> Result<()> {
        ensure_no_symlink_components(self.base_path, normalized, requested)?;
        match fs::symlink_metadata(path) {
            Ok(_) => fs::remove_file(path).map_err(|e| EditError::BackupFailed {
                path: requested.to_string(),
                reason: format!("remove before restore: {e}"),
            })?,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(EditError::BackupFailed {
                    path: requested.to_string(),
                    reason: format!("stat before restore: {err}"),
                });
            }
        }
        fs::copy(backup, path).map_err(|e| EditError::BackupFailed {
            path: requested.to_string(),
            reason: format!("restore copy: {e}"),
        })?;
        Ok(())
    }

    fn remove_created_file(
        &self,
        requested: &str,
        normalized: &str,
        path: &Utf8Path,
    ) -> Result<()> {
        ensure_no_symlink_components(self.base_path, normalized, requested)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                fs::remove_file(path).map_err(|e| EditError::BackupFailed {
                    path: requested.to_string(),
                    reason: format!("remove created file: {e}"),
                })?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(EditError::BackupFailed {
                    path: requested.to_string(),
                    reason: format!("stat created file: {err}"),
                });
            }
        }
        Ok(())
    }

    fn cleanup_backup_dir(&self) -> Result<()> {
        let backup_root = self.base_path.join(&self.backup_dir);
        match fs::remove_dir_all(&backup_root) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EditError::BackupFailed {
                path: backup_root.to_string(),
                reason: format!("remove backup dir: {err}"),
            }),
        }
    }
}

impl PatchEngine {
    /// Apply a single patch step.
    pub fn apply(step: &PatchStep, base_path: &Utf8Path, editable: &EditableSet) -> Result<()> {
        let canonical_base = canonicalize_utf8(base_path, "base path")?;
        match step {
            PatchStep::LineRange {
                path,
                start_line,
                end_line,
                content,
            } => {
                let target = Self::resolve_target(base_path, &canonical_base, path, editable)?;
                Self::apply_line_range(&target, *start_line, *end_line, content)
            }
            PatchStep::UnifiedDiff { path, diff } => {
                let target = Self::resolve_target(base_path, &canonical_base, path, editable)?;
                Self::apply_unified_diff(&target, diff)
            }
            PatchStep::WholeFile { path, content } => {
                let target = Self::resolve_target(base_path, &canonical_base, path, editable)?;
                Self::apply_whole_file(&target, content)
            }
            PatchStep::Create { path, content } => {
                let target = Self::resolve_target(base_path, &canonical_base, path, editable)?;
                Self::apply_create(base_path, &canonical_base, &target, content)
            }
            PatchStep::Delete { path } => {
                let target = Self::resolve_target(base_path, &canonical_base, path, editable)?;
                Self::apply_delete(&target)
            }
            PatchStep::Move { from, to } => {
                let from_path = Self::resolve_target(base_path, &canonical_base, from, editable)?;
                let to_path = Self::resolve_target(base_path, &canonical_base, to, editable)?;
                Self::apply_move(base_path, &canonical_base, &from_path, &to_path)
            }
        }
    }

    fn resolve_target(
        base: &Utf8Path,
        canonical_base: &Utf8Path,
        path: &str,
        editable: &EditableSet,
    ) -> Result<SafePath> {
        let normalized = normalize_patch_path(path)?;
        ensure_editable(path, &normalized, editable)?;
        ensure_path_within_base(base, canonical_base, &normalized, path)?;
        ensure_no_symlink_components(base, &normalized, path)?;

        Ok(SafePath {
            requested: path.to_string(),
            full_path: base.join(&normalized),
            normalized,
        })
    }

    fn apply_line_range(
        target: &SafePath,
        start_line: usize,
        end_line: usize,
        content: &str,
    ) -> Result<()> {
        let file_content = fs::read_to_string(&target.full_path)
            .map_err(|e| EditError::Io(format!("read {}: {}", target.requested, e)))?;
        let lines: Vec<&str> = file_content.lines().collect();

        if start_line == 0 || end_line < start_line || end_line > lines.len() {
            return Err(EditError::InvalidLineRange {
                path: target.requested.clone(),
                start: start_line,
                end: end_line,
                file_lines: lines.len(),
            });
        }

        let mut result = String::new();
        for line in lines.iter().take(start_line - 1) {
            result.push_str(line);
            result.push('\n');
        }
        result.push_str(content);
        if !content.ends_with('\n') {
            result.push('\n');
        }
        for line in lines.iter().skip(end_line) {
            result.push_str(line);
            result.push('\n');
        }

        fs::write(&target.full_path, result)
            .map_err(|e| EditError::Io(format!("write {}: {}", target.requested, e)))?;
        Ok(())
    }

    fn apply_unified_diff(target: &SafePath, diff: &str) -> Result<()> {
        let original = fs::read_to_string(&target.full_path)
            .map_err(|e| EditError::Io(format!("read {}: {}", target.requested, e)))?;

        let patched = apply_unified_diff_text(&original, diff).map_err(|reason| {
            EditError::DiffApplyFailed {
                path: target.requested.clone(),
                reason,
            }
        })?;

        fs::write(&target.full_path, patched)
            .map_err(|e| EditError::Io(format!("write {}: {}", target.requested, e)))?;
        Ok(())
    }

    fn apply_whole_file(target: &SafePath, content: &str) -> Result<()> {
        fs::write(&target.full_path, content)
            .map_err(|e| EditError::Io(format!("write {}: {}", target.requested, e)))?;
        Ok(())
    }

    fn apply_create(
        base: &Utf8Path,
        canonical_base: &Utf8Path,
        target: &SafePath,
        content: &str,
    ) -> Result<()> {
        ensure_missing_target(&target.full_path, &target.requested, "create")?;
        if let Some(parent) = target.full_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| EditError::Io(format!("mkdir {}: {}", parent, e)))?;
            ensure_path_within_base(base, canonical_base, &target.normalized, &target.requested)?;
        }
        fs::write(&target.full_path, content)
            .map_err(|e| EditError::Io(format!("write {}: {}", target.requested, e)))?;
        Ok(())
    }

    fn apply_delete(target: &SafePath) -> Result<()> {
        fs::remove_file(&target.full_path)
            .map_err(|e| EditError::Io(format!("delete {}: {}", target.requested, e)))?;
        Ok(())
    }

    fn apply_move(
        base: &Utf8Path,
        canonical_base: &Utf8Path,
        from: &SafePath,
        to: &SafePath,
    ) -> Result<()> {
        ensure_missing_target(&to.full_path, &to.requested, "move")?;
        if let Some(parent) = to.full_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| EditError::Io(format!("mkdir {}: {}", parent, e)))?;
            ensure_path_within_base(base, canonical_base, &to.normalized, &to.requested)?;
        }
        fs::rename(&from.full_path, &to.full_path).map_err(|e| {
            EditError::Io(format!(
                "move {} -> {}: {}",
                from.requested, to.requested, e
            ))
        })?;
        Ok(())
    }
}

fn ensure_missing_target(path: &Utf8Path, requested: &str, action: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(EditError::Io(format!(
            "{action} {requested}: target already exists"
        ))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(EditError::Io(format!("stat {requested}: {err}"))),
    }
}

fn normalize_patch_path(path: &str) -> Result<String> {
    if path.is_empty() || has_windows_prefix(path) || path.contains('\\') {
        return Err(path_not_editable(path));
    }

    let patch_path = Utf8Path::new(path);
    if patch_path.is_absolute() {
        return Err(path_not_editable(path));
    }

    let mut normalized = Utf8PathBuf::new();
    for component in patch_path.components() {
        match component {
            Utf8Component::Normal(part) => normalized.push(part),
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir | Utf8Component::Prefix(_) | Utf8Component::RootDir => {
                return Err(path_not_editable(path));
            }
        }
    }

    if normalized.as_str().is_empty() {
        return Err(path_not_editable(path));
    }

    Ok(normalized.into_string())
}

fn has_windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn ensure_editable(requested: &str, normalized: &str, editable: &EditableSet) -> Result<()> {
    let allowed = editable.paths().iter().any(|path| {
        normalize_patch_path(path)
            .map(|editable_path| editable_path == normalized)
            .unwrap_or(false)
    });

    if allowed {
        Ok(())
    } else {
        Err(path_not_editable(requested))
    }
}

fn ensure_path_within_base(
    base: &Utf8Path,
    canonical_base: &Utf8Path,
    normalized: &str,
    requested: &str,
) -> Result<()> {
    let mut candidate = base.to_path_buf();
    for component in normalized.split('/') {
        candidate.push(component);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let canonical_target = fs::canonicalize(&candidate)
                    .ok()
                    .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
                    .ok_or_else(|| path_not_editable(requested))?;
                if !canonical_target.starts_with(canonical_base) {
                    return Err(path_not_editable(requested));
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => break,
            Err(err) => return Err(EditError::Io(format!("stat {}: {}", requested, err))),
        }
    }

    Ok(())
}

fn ensure_no_symlink_components(base: &Utf8Path, normalized: &str, requested: &str) -> Result<()> {
    let mut candidate = base.to_path_buf();
    for component in normalized.split('/') {
        candidate.push(component);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(path_not_editable(requested));
            }
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => break,
            Err(err) => return Err(EditError::Io(format!("stat {}: {}", requested, err))),
        }
    }

    Ok(())
}

fn ensure_regular_existing_file(target: &SafePath, action: &str) -> Result<()> {
    match fs::symlink_metadata(&target.full_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(path_not_editable(&target.requested))
        }
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(EditError::Io(format!(
            "{action} {}: target is not a regular file",
            target.requested
        ))),
        Err(err) if err.kind() == ErrorKind::NotFound => Err(EditError::FileNotFound {
            path: target.requested.clone(),
        }),
        Err(err) => Err(EditError::Io(format!(
            "{action} {}: {}",
            target.requested, err
        ))),
    }
}

fn canonicalize_utf8(path: &Utf8Path, label: &str) -> Result<Utf8PathBuf> {
    let canonical = fs::canonicalize(path)
        .map_err(|e| EditError::Io(format!("canonicalize {}: {}", label, e)))?;
    Utf8PathBuf::from_path_buf(canonical).map_err(|path| {
        EditError::Io(format!(
            "canonicalize {}: non-UTF-8 path {}",
            label,
            path.display()
        ))
    })
}

fn path_not_editable(path: &str) -> EditError {
    EditError::FileNotEditable {
        path: path.to_string(),
    }
}

/// Apply a unified diff to original text by locating each hunk from its
/// pre-image context instead of trusting the hunk header line number.
fn apply_unified_diff_text(original: &str, diff: &str) -> std::result::Result<String, String> {
    let mut result_lines: Vec<String> = original.lines().map(String::from).collect();
    let hunks = parse_unified_hunks(diff)?;
    let mut search_start = 0usize;
    let mut line_offset = 0isize;

    for hunk in hunks {
        let location = locate_hunk(&result_lines, &hunk, search_start, line_offset)?;
        result_lines.splice(
            location..location + hunk.old_lines.len(),
            hunk.new_lines.iter().cloned(),
        );
        search_start = location + hunk.new_lines.len();
        line_offset += hunk.new_lines.len() as isize - hunk.old_lines.len() as isize;
    }

    Ok(result_lines.join("\n"))
}

#[derive(Debug)]
struct UnifiedHunk {
    header: String,
    old_start: usize,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn parse_unified_hunks(diff: &str) -> std::result::Result<Vec<UnifiedHunk>, String> {
    let mut hunks = Vec::new();
    let mut current: Option<UnifiedHunk> = None;

    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            current = Some(UnifiedHunk {
                header: line.to_string(),
                old_start: parse_hunk_old_start(line)?,
                old_lines: Vec::new(),
                new_lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        if line.starts_with('\\') {
            continue;
        }

        if let Some(content) = line.strip_prefix(' ') {
            hunk.old_lines.push(content.to_string());
            hunk.new_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix('-') {
            hunk.old_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix('+') {
            hunk.new_lines.push(content.to_string());
        } else {
            return Err(format!("invalid unified diff hunk line: {line}"));
        }
    }

    if let Some(hunk) = current {
        hunks.push(hunk);
    }

    if hunks.is_empty() {
        return Err("unified diff contains no hunks".to_string());
    }

    Ok(hunks)
}

fn parse_hunk_old_start(header: &str) -> std::result::Result<usize, String> {
    let old_range = header
        .split_whitespace()
        .find_map(|part| part.strip_prefix('-'))
        .ok_or_else(|| format!("invalid unified diff hunk header: {header}"))?;
    let start = old_range
        .split_once(',')
        .map_or(old_range, |(start, _)| start);
    start
        .parse::<usize>()
        .map_err(|_| format!("invalid unified diff hunk header: {header}"))
}

fn locate_hunk(
    lines: &[String],
    hunk: &UnifiedHunk,
    search_start: usize,
    line_offset: isize,
) -> std::result::Result<usize, String> {
    if hunk.old_lines.is_empty() {
        let location = adjusted_hunk_start(hunk.old_start, true, line_offset);
        return if location <= lines.len() {
            Ok(location)
        } else {
            Err(format!(
                "hunk insertion point out of bounds for {}: line {} in {} line file",
                hunk.header,
                location + 1,
                lines.len()
            ))
        };
    }

    let old_len = hunk.old_lines.len();
    if old_len > lines.len() {
        return Err(format!(
            "hunk context not found for {}: expected {} pre-image lines, file has {} lines",
            hunk.header,
            old_len,
            lines.len()
        ));
    }

    let candidates: Vec<usize> = (0..=lines.len() - old_len)
        .filter(|start| lines[*start..*start + old_len] == hunk.old_lines[..])
        .collect();

    if candidates.is_empty() {
        return Err(format!("hunk context not found for {}", hunk.header));
    }

    let pool: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|candidate| *candidate >= search_start)
        .collect();
    if pool.is_empty() {
        return Err(format!(
            "hunk context not found after prior hunk for {}",
            hunk.header
        ));
    }

    let expected = adjusted_hunk_start(hunk.old_start, false, line_offset);
    if pool.contains(&expected) {
        return Ok(expected);
    }
    if pool.len() == 1 {
        return Ok(pool[0]);
    }

    Err(format!(
        "ambiguous hunk context for {}: matched {} locations",
        hunk.header,
        pool.len()
    ))
}

fn adjusted_hunk_start(old_start: usize, empty_preimage: bool, line_offset: isize) -> usize {
    let base = if empty_preimage {
        old_start as isize
    } else {
        old_start.saturating_sub(1) as isize
    };
    (base + line_offset).max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, path: &str, content: &str) {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(full).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn read_file(dir: &TempDir, path: &str) -> String {
        fs::read_to_string(dir.path().join(path)).unwrap()
    }

    fn editable<I, S>(paths: I) -> EditableSet
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        EditableSet::from_paths(paths.into_iter().map(Into::into).collect())
    }

    fn apply_step<I, S>(dir: &TempDir, step: &PatchStep, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let editable = editable(paths);
        PatchEngine::apply(step, Utf8Path::from_path(dir.path()).unwrap(), &editable)
    }

    fn apply_transaction<I, S>(dir: &TempDir, steps: &[PatchStep], paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let editable = editable(paths);
        let base = Utf8Path::from_path(dir.path()).unwrap();
        let mut transaction = PatchTransaction::new(base, &editable, ".mimir/test-backups")?;
        transaction.apply_all(steps)
    }

    #[test]
    fn test_apply_line_range() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\nline4\nline5\n");

        let step = PatchStep::LineRange {
            path: "test.txt".into(),
            start_line: 2,
            end_line: 3,
            content: "replaced\n".into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nreplaced\nline4\nline5\n");
    }

    #[test]
    fn test_apply_whole_file() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "old content");

        let step = PatchStep::WholeFile {
            path: "test.txt".into(),
            content: "new content".into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        assert_eq!(read_file(&dir, "test.txt"), "new content");
    }

    #[test]
    fn test_apply_create() {
        let dir = TempDir::new().unwrap();
        let step = PatchStep::Create {
            path: "nested/file.txt".into(),
            content: "hello".into(),
        };

        apply_step(&dir, &step, ["nested/file.txt"]).unwrap();
        assert_eq!(read_file(&dir, "nested/file.txt"), "hello");
    }

    #[test]
    fn test_apply_create_rejects_existing_target() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "nested/file.txt", "original");
        let step = PatchStep::Create {
            path: "nested/file.txt".into(),
            content: "replacement".into(),
        };

        let err = apply_step(&dir, &step, ["nested/file.txt"]).unwrap_err();
        assert!(matches!(err, EditError::Io(message) if message.contains("target already exists")));
        assert_eq!(read_file(&dir, "nested/file.txt"), "original");
    }

    #[test]
    fn test_apply_delete() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "content");

        let step = PatchStep::Delete {
            path: "test.txt".into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        assert!(!dir.path().join("test.txt").exists());
    }

    #[test]
    fn test_apply_move() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "old.txt", "content");

        let step = PatchStep::Move {
            from: "old.txt".into(),
            to: "new.txt".into(),
        };

        apply_step(&dir, &step, ["old.txt", "new.txt"]).unwrap();
        assert!(!dir.path().join("old.txt").exists());
        assert_eq!(read_file(&dir, "new.txt"), "content");
    }

    #[test]
    fn test_apply_move_rejects_existing_destination() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "old.txt", "source");
        write_file(&dir, "new.txt", "destination");

        let step = PatchStep::Move {
            from: "old.txt".into(),
            to: "new.txt".into(),
        };

        let err = apply_step(&dir, &step, ["old.txt", "new.txt"]).unwrap_err();
        assert!(matches!(err, EditError::Io(message) if message.contains("target already exists")));
        assert_eq!(read_file(&dir, "old.txt"), "source");
        assert_eq!(read_file(&dir, "new.txt"), "destination");
    }

    #[test]
    fn transaction_rolls_back_create_and_created_parent_dirs() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "existing.txt", "one\n");
        let steps = vec![
            PatchStep::Create {
                path: "new/parent/file.txt".into(),
                content: "created".into(),
            },
            PatchStep::LineRange {
                path: "existing.txt".into(),
                start_line: 5,
                end_line: 6,
                content: "bad".into(),
            },
        ];

        let err =
            apply_transaction(&dir, &steps, ["new/parent/file.txt", "existing.txt"]).unwrap_err();

        assert!(matches!(err, EditError::InvalidLineRange { .. }));
        assert!(!dir.path().join("new/parent/file.txt").exists());
        assert!(!dir.path().join("new/parent").exists());
        assert!(!dir.path().join("new").exists());
        assert_eq!(read_file(&dir, "existing.txt"), "one\n");
    }

    #[test]
    fn transaction_rolls_back_delete() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "delete-me.txt", "keep me");
        write_file(&dir, "existing.txt", "one\n");
        let steps = vec![
            PatchStep::Delete {
                path: "delete-me.txt".into(),
            },
            PatchStep::LineRange {
                path: "existing.txt".into(),
                start_line: 5,
                end_line: 6,
                content: "bad".into(),
            },
        ];

        let err = apply_transaction(&dir, &steps, ["delete-me.txt", "existing.txt"]).unwrap_err();

        assert!(matches!(err, EditError::InvalidLineRange { .. }));
        assert_eq!(read_file(&dir, "delete-me.txt"), "keep me");
    }

    #[test]
    fn transaction_rolls_back_move() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "from.txt", "move me");
        write_file(&dir, "existing.txt", "one\n");
        let steps = vec![
            PatchStep::Move {
                from: "from.txt".into(),
                to: "nested/to.txt".into(),
            },
            PatchStep::LineRange {
                path: "existing.txt".into(),
                start_line: 5,
                end_line: 6,
                content: "bad".into(),
            },
        ];

        let err = apply_transaction(&dir, &steps, ["from.txt", "nested/to.txt", "existing.txt"])
            .unwrap_err();

        assert!(matches!(err, EditError::InvalidLineRange { .. }));
        assert_eq!(read_file(&dir, "from.txt"), "move me");
        assert!(!dir.path().join("nested/to.txt").exists());
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn transaction_rolls_back_file_modified_multiple_times() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "target.txt", "one\ntwo\n");
        write_file(&dir, "existing.txt", "ok\n");
        let steps = vec![
            PatchStep::LineRange {
                path: "target.txt".into(),
                start_line: 1,
                end_line: 1,
                content: "changed\n".into(),
            },
            PatchStep::WholeFile {
                path: "target.txt".into(),
                content: "changed again\n".into(),
            },
            PatchStep::LineRange {
                path: "existing.txt".into(),
                start_line: 10,
                end_line: 11,
                content: "bad".into(),
            },
        ];

        let err = apply_transaction(&dir, &steps, ["target.txt", "existing.txt"]).unwrap_err();

        assert!(matches!(err, EditError::InvalidLineRange { .. }));
        assert_eq!(read_file(&dir, "target.txt"), "one\ntwo\n");
    }

    #[test]
    fn transaction_rolls_back_move_source_modified_earlier() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "from.txt", "original\n");
        write_file(&dir, "existing.txt", "ok\n");
        let steps = vec![
            PatchStep::LineRange {
                path: "from.txt".into(),
                start_line: 1,
                end_line: 1,
                content: "modified\n".into(),
            },
            PatchStep::Move {
                from: "from.txt".into(),
                to: "nested/to.txt".into(),
            },
            PatchStep::LineRange {
                path: "existing.txt".into(),
                start_line: 10,
                end_line: 11,
                content: "bad".into(),
            },
        ];

        let err = apply_transaction(&dir, &steps, ["from.txt", "nested/to.txt", "existing.txt"])
            .unwrap_err();

        assert!(matches!(err, EditError::InvalidLineRange { .. }));
        assert_eq!(read_file(&dir, "from.txt"), "original\n");
        assert!(!dir.path().join("nested/to.txt").exists());
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn transaction_rejects_create_then_move_before_mutation() {
        let dir = TempDir::new().unwrap();
        let steps = vec![
            PatchStep::Create {
                path: "temp.txt".into(),
                content: "new\n".into(),
            },
            PatchStep::Move {
                from: "temp.txt".into(),
                to: "final.txt".into(),
            },
        ];

        let _err = apply_transaction(&dir, &steps, ["temp.txt", "final.txt"]).unwrap_err();

        assert!(!dir.path().join("temp.txt").exists());
        assert!(!dir.path().join("final.txt").exists());
    }

    #[test]
    fn test_apply_unified_diff() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2_modified\n line3\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nline2_modified\nline3");
    }

    #[test]
    fn test_apply_unified_diff_finds_hunk_by_context_when_header_line_is_wrong() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\nline4\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,3 +1,3 @@\n line2\n-line3\n+line3_modified\n line4\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nline2\nline3_modified\nline4");
    }

    #[test]
    fn test_apply_unified_diff_rejects_missing_context_without_write() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,3 +1,3 @@\n missing\n-line2\n+line2_modified\n line3\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        let err = apply_step(&dir, &step, ["test.txt"]).unwrap_err();
        assert!(matches!(err, EditError::DiffApplyFailed { .. }));
        assert_eq!(read_file(&dir, "test.txt"), "line1\nline2\nline3\n");
    }

    #[test]
    fn test_apply_unified_diff_searches_sparse_hunk_by_removed_line() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1 +1 @@\n-line3\n+line3_modified\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        apply_step(&dir, &step, ["test.txt"]).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nline2\nline3_modified");
    }

    #[test]
    fn test_apply_unified_diff_rejects_ambiguous_context() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "target\nkeep\nmiddle\ntarget\nkeep\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -9,2 +9,2 @@\n-target\n+changed\n keep\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        let err = apply_step(&dir, &step, ["test.txt"]).unwrap_err();
        assert!(matches!(err, EditError::DiffApplyFailed { .. }));
        assert_eq!(
            read_file(&dir, "test.txt"),
            "target\nkeep\nmiddle\ntarget\nkeep\n"
        );
    }

    #[test]
    fn test_apply_unified_diff_rejects_out_of_order_hunks() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "target\nkeep\nmiddle\ntarget\nkeep\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -4,2 +4,2 @@\n-target\n+changed_late\n keep\n@@ -1,2 +1,2 @@\n-target\n+changed_early\n keep\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        let err = apply_step(&dir, &step, ["test.txt"]).unwrap_err();
        assert!(matches!(err, EditError::DiffApplyFailed { .. }));
        assert_eq!(
            read_file(&dir, "test.txt"),
            "target\nkeep\nmiddle\ntarget\nkeep\n"
        );
    }

    #[test]
    fn test_invalid_line_range() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\n");

        let step = PatchStep::LineRange {
            path: "test.txt".into(),
            start_line: 5,
            end_line: 10,
            content: "x".into(),
        };

        let err = apply_step(&dir, &step, ["test.txt"]).unwrap_err();
        assert!(matches!(err, EditError::InvalidLineRange { .. }));
    }

    #[test]
    fn test_apply_enforces_editable_set() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "allowed.txt", "old");

        let step = PatchStep::WholeFile {
            path: "allowed.txt".into(),
            content: "new".into(),
        };

        let err = apply_step(&dir, &step, ["other.txt"]).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert_eq!(read_file(&dir, "allowed.txt"), "old");
    }

    #[test]
    fn test_apply_rejects_parent_path_escape() {
        let root = TempDir::new().unwrap();
        let base = root.path().join("base");
        fs::create_dir(&base).unwrap();

        let step = PatchStep::Create {
            path: "../outside.txt".into(),
            content: "escaped".into(),
        };
        let editable = editable(["../outside.txt"]);

        let err =
            PatchEngine::apply(&step, Utf8Path::from_path(&base).unwrap(), &editable).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert!(!root.path().join("outside.txt").exists());
    }

    #[test]
    fn test_apply_rejects_absolute_path_escape() {
        let root = TempDir::new().unwrap();
        let base = root.path().join("base");
        fs::create_dir(&base).unwrap();
        let outside = root.path().join("outside.txt");
        let outside_path = outside.to_str().unwrap().to_string();

        let step = PatchStep::Create {
            path: outside_path.clone(),
            content: "escaped".into(),
        };
        let editable = editable([outside_path]);

        let err =
            PatchEngine::apply(&step, Utf8Path::from_path(&base).unwrap(), &editable).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert!(!outside.exists());
    }

    #[test]
    fn test_apply_rejects_windows_prefix_path() {
        let dir = TempDir::new().unwrap();
        let step = PatchStep::Create {
            path: "C:\\outside.txt".into(),
            content: "escaped".into(),
        };

        let err = apply_step(&dir, &step, ["C:\\outside.txt"]).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert!(!dir.path().join("C:\\outside.txt").exists());
    }

    #[test]
    fn test_apply_rejects_move_destination_escape() {
        let root = TempDir::new().unwrap();
        let base = root.path().join("base");
        fs::create_dir(&base).unwrap();
        fs::write(base.join("old.txt"), "content").unwrap();

        let step = PatchStep::Move {
            from: "old.txt".into(),
            to: "../outside.txt".into(),
        };
        let editable = editable(["old.txt", "../outside.txt"]);

        let err =
            PatchEngine::apply(&step, Utf8Path::from_path(&base).unwrap(), &editable).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert!(base.join("old.txt").exists());
        assert!(!root.path().join("outside.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let base = root.path().join("base");
        let outside = root.path().join("outside");
        fs::create_dir(&base).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("target.txt"), "secret").unwrap();
        symlink(&outside, base.join("link")).unwrap();

        let step = PatchStep::WholeFile {
            path: "link/target.txt".into(),
            content: "escaped".into(),
        };
        let editable = editable(["link/target.txt"]);

        let err =
            PatchEngine::apply(&step, Utf8Path::from_path(&base).unwrap(), &editable).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert_eq!(
            fs::read_to_string(outside.join("target.txt")).unwrap(),
            "secret"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_rejects_in_repo_symlink_target() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        write_file(&dir, "real.txt", "real");
        symlink("real.txt", dir.path().join("link.txt")).unwrap();

        let step = PatchStep::WholeFile {
            path: "link.txt".into(),
            content: "changed".into(),
        };
        let err = apply_step(&dir, &step, ["link.txt"]).unwrap_err();

        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert_eq!(read_file(&dir, "real.txt"), "real");
    }

    #[cfg(unix)]
    #[test]
    fn transaction_rejects_symlink_backed_target_before_mutation() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        write_file(&dir, "real.txt", "real");
        write_file(&dir, "later.txt", "one\n");
        symlink("real.txt", dir.path().join("link.txt")).unwrap();
        let steps = vec![
            PatchStep::WholeFile {
                path: "link.txt".into(),
                content: "changed".into(),
            },
            PatchStep::LineRange {
                path: "later.txt".into(),
                start_line: 1,
                end_line: 1,
                content: "changed".into(),
            },
        ];

        let err = apply_transaction(&dir, &steps, ["link.txt", "later.txt"]).unwrap_err();

        assert!(matches!(err, EditError::FileNotEditable { .. }));
        assert_eq!(read_file(&dir, "real.txt"), "real");
        assert_eq!(read_file(&dir, "later.txt"), "one\n");
    }
}
