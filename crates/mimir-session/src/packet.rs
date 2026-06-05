//! Packet replay and sharing helpers shared by the CLI and local UI server.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path};

use anyhow::{anyhow, bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use mimir_providers::{ProviderMessage, ProviderRequest};
use mimir_runs::{RunDir, RunId};
use mimir_schemas::ContextPacket;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PACKET_SHARE_BUNDLE_KIND: &str = "mimir.packet_share";
const PACKET_SHARE_BUNDLE_SCHEMA_VERSION: u32 = 1;
const MAX_PROVIDER_REQUEST_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_PACKET_ARTIFACT_BYTES: usize = 1024 * 1024;
/// Upper bound for an on-disk shared packet bundle: a full context packet plus
/// the redacted provider request plus JSON envelope overhead.
const MAX_SHARED_BUNDLE_ARTIFACT_BYTES: usize =
    MAX_CONTEXT_PACKET_ARTIFACT_BYTES + MAX_PROVIDER_REQUEST_ARTIFACT_BYTES + 64 * 1024;

/// Portable redacted packet share bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedPacketBundle {
    /// Schema version for the share bundle.
    pub schema_version: u32,
    /// Bundle kind marker.
    pub kind: String,
    /// RFC3339 export timestamp.
    pub exported_at: String,
    /// Owning run id.
    pub run_id: String,
    /// Stable packet hash.
    pub packet_hash: String,
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Schema-valid context packet.
    pub packet: ContextPacket,
    /// Redacted replay payload.
    pub replay: SharedPacketReplay,
}

/// Redacted replay payload embedded in a share bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedPacketReplay {
    /// SHA-256 of the pretty-printed redacted provider request.
    pub provider_request_sha256: String,
    /// SHA-256 of the user prompt content, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_prompt_sha256: Option<String>,
    /// Provider request with secret-like material redacted.
    pub provider_request_redacted: serde_json::Value,
}

/// UI-safe replay request preview for one run.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayRequestPreview {
    /// Owning run id.
    pub run_id: String,
    /// Packet id.
    pub packet_id: String,
    /// Stable packet hash.
    pub packet_hash: String,
    /// Workspace-relative packet path.
    pub packet_path: String,
    /// Whether the request came from a saved artifact or deterministic reconstruction.
    pub source: ReplayRequestSource,
    /// SHA-256 of the pretty-printed redacted provider request.
    pub provider_request_sha256: String,
    /// SHA-256 of the user prompt content, when available.
    pub user_prompt_sha256: Option<String>,
    /// Always true for this API surface.
    pub redacted: bool,
    /// Redacted provider request JSON.
    pub request: serde_json::Value,
}

/// Source of a redacted replay request.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayRequestSource {
    /// Loaded from `.mimir/runs/<run_id>/provider_request.redacted.json`.
    SavedArtifact,
    /// Reconstructed from the saved context packet.
    Reconstructed,
}

/// UI-safe share bundle response for one run.
#[derive(Debug, Clone, Serialize)]
pub struct ShareBundlePreview {
    /// Owning run id.
    pub run_id: String,
    /// Packet id.
    pub packet_id: String,
    /// Stable packet hash.
    pub packet_hash: String,
    /// Workspace-relative packet path.
    pub packet_path: String,
    /// SHA-256 of the pretty-printed share bundle.
    pub bundle_sha256: String,
    /// Always true for this API surface.
    pub redacted: bool,
    /// Portable share bundle.
    pub bundle: SharedPacketBundle,
}

/// Return redacted packet-only JSON bytes for a run.
///
/// # Errors
/// Returns an error if the run id is invalid, the packet is tampered, sources
/// are stale, or secret-like material is present.
pub fn redacted_packet_only_bytes_for_run(
    workspace_root: &Utf8Path,
    run_id: &str,
) -> Result<Vec<u8>> {
    let loaded = load_replayable_packet(workspace_root, run_id)?;
    ensure_context_packet_safe_to_share(&loaded.packet)?;
    redacted_pretty_json_bytes(&loaded.packet)
}

/// Return a redacted replay request preview for a run.
///
/// # Errors
/// Returns an error if the run id is invalid, the packet is tampered, sources
/// are stale, or a saved redacted request contains secret-like material.
pub fn replay_request_preview_for_run(
    workspace_root: &Utf8Path,
    run_id: &str,
) -> Result<ReplayRequestPreview> {
    let loaded = load_replayable_packet(workspace_root, run_id)?;
    let (replay, source) = replay_for_loaded_packet(&loaded)?;
    Ok(ReplayRequestPreview {
        run_id: run_id.to_string(),
        packet_id: loaded.packet.packet_id.clone(),
        packet_hash: loaded.packet.packet_hash.clone(),
        packet_path: path_to_workspace_string(&loaded.workspace_root, &loaded.packet_path)?,
        source,
        provider_request_sha256: replay.provider_request_sha256,
        user_prompt_sha256: replay.user_prompt_sha256,
        redacted: true,
        request: replay.provider_request_redacted,
    })
}

/// Return redacted provider request JSON bytes for a run.
///
/// Saved `provider_request.redacted.json` artifacts are returned byte-for-byte
/// after integrity and secret checks; otherwise the request is reconstructed
/// deterministically from the packet.
///
/// # Errors
/// Returns an error if the run id is invalid, the packet is tampered, sources
/// are stale, or a saved redacted request contains secret-like material.
pub fn replay_request_bytes_for_run(workspace_root: &Utf8Path, run_id: &str) -> Result<Vec<u8>> {
    let loaded = load_replayable_packet(workspace_root, run_id)?;
    let provider_request_path = loaded.run_dir.join("provider_request.redacted.json");
    if optional_regular_artifact_exists(&provider_request_path, "provider request artifact")? {
        return redacted_provider_request_artifact_bytes(&provider_request_path);
    }
    let request = provider_request_from_packet(&loaded.workspace_root, &loaded.packet, false)?;
    redacted_pretty_json_bytes(&request)
}

/// Load the provider request that should be dispatched for a saved run.
///
/// Saved redacted request artifacts are preferred so mode-specific ask/plan/code
/// prompts remain replayable. Build-only packets fall back to deterministic
/// reconstruction from the context packet.
pub fn provider_request_for_run(
    workspace_root: &Utf8Path,
    run_id: &str,
    stream: bool,
) -> Result<(ContextPacket, ProviderRequest, ReplayRequestSource)> {
    let loaded = load_replayable_packet(workspace_root, run_id)?;
    let provider_request_path = loaded.run_dir.join("provider_request.redacted.json");
    if optional_regular_artifact_exists(&provider_request_path, "provider request artifact")? {
        let data = redacted_provider_request_artifact_bytes(&provider_request_path)?;
        let mut request: ProviderRequest = serde_json::from_slice(&data).map_err(|err| {
            anyhow!(
                "provider request artifact '{}' is invalid JSON: {err}",
                provider_request_path
            )
        })?;
        if request.model != loaded.packet.model {
            bail!("provider request artifact model does not match context packet");
        }
        request.stream = Some(stream);
        return Ok((loaded.packet, request, ReplayRequestSource::SavedArtifact));
    }
    let request = provider_request_from_packet(&loaded.workspace_root, &loaded.packet, stream)?;
    Ok((loaded.packet, request, ReplayRequestSource::Reconstructed))
}

/// Build a redacted share bundle preview for a run.
///
/// # Errors
/// Returns an error if the run id is invalid, the packet is tampered, sources
/// are stale, or secret-like material is present in the packet/request.
pub fn share_bundle_preview_for_run(
    workspace_root: &Utf8Path,
    run_id: &str,
) -> Result<ShareBundlePreview> {
    let loaded = load_replayable_packet(workspace_root, run_id)?;
    let (replay, _) = replay_for_loaded_packet(&loaded)?;
    let bundle = build_shared_packet_bundle(&loaded.packet, run_id, replay)?;
    let bundle_bytes = redacted_pretty_json_bytes(&bundle)?;
    Ok(ShareBundlePreview {
        run_id: run_id.to_string(),
        packet_id: loaded.packet.packet_id.clone(),
        packet_hash: loaded.packet.packet_hash.clone(),
        packet_path: path_to_workspace_string(&loaded.workspace_root, &loaded.packet_path)?,
        bundle_sha256: sha256_hex(&bundle_bytes),
        redacted: true,
        bundle,
    })
}

/// Read and verify a portable shared packet bundle from disk.
///
/// # Errors
/// Returns an error when the bundle cannot be read or fails integrity checks.
pub fn read_shared_packet_bundle(path: &Path) -> Result<SharedPacketBundle> {
    // Reject symlinks and non-regular files, and cap the on-disk size before
    // reading — mirrors the hardening on `read_regular_artifact_bytes`.
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        anyhow!(
            "shared packet bundle '{}' could not be inspected: {err}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        bail!(
            "shared packet bundle '{}' is not a regular file",
            path.display()
        );
    }
    if metadata.len() > MAX_SHARED_BUNDLE_ARTIFACT_BYTES as u64 {
        bail!(
            "shared packet bundle '{}' exceeds size cap ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_SHARED_BUNDLE_ARTIFACT_BYTES
        );
    }
    let data = fs::read_to_string(path).map_err(|err| {
        anyhow!(
            "shared packet bundle '{}' could not be read: {err}",
            path.display()
        )
    })?;
    let bundle: SharedPacketBundle = serde_json::from_str(&data).map_err(|err| {
        anyhow!(
            "shared packet bundle '{}' is invalid JSON: {err}",
            path.display()
        )
    })?;
    verify_shared_packet_bundle(&bundle)?;
    Ok(bundle)
}

/// Return the byte-identical redacted provider request from a verified bundle.
///
/// # Errors
/// Returns an error when the bundle fails integrity checks.
pub fn shared_bundle_request_bytes(bundle: &SharedPacketBundle) -> Result<Vec<u8>> {
    verify_shared_packet_bundle(bundle)?;
    Ok(serde_json::to_vec_pretty(
        &bundle.replay.provider_request_redacted,
    )?)
}

/// Return redacted pretty JSON bytes.
///
/// # Errors
/// Returns an error when the value cannot be serialized.
pub fn redacted_pretty_json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&redacted_json_value(value)?)?)
}

/// Verify that a context packet's stored hash still matches its content.
pub fn verify_packet_integrity(packet: &ContextPacket) -> Result<()> {
    let recomputed = mimir_context::hash_packet(packet);
    if packet.packet_hash != recomputed {
        bail!(
            "context packet hash mismatch: declared {}, recomputed {}",
            packet.packet_hash,
            recomputed
        );
    }
    Ok(())
}

/// Verify that a packet belongs to an expected validated run ID.
pub fn verify_packet_run_id(packet: &ContextPacket, expected_run_id: &str) -> Result<()> {
    RunId::parse(expected_run_id.to_string()).map_err(|_| anyhow!("invalid run id"))?;
    if packet.run_id != expected_run_id {
        bail!(
            "packet run_id mismatch: packet declares {}, but run directory is {}",
            packet.run_id,
            expected_run_id
        );
    }
    Ok(())
}

/// Verify that the packet's provider capability snapshot still matches the registry.
pub fn verify_capability_snapshot_ref(packet: &ContextPacket) -> Result<()> {
    let current = mimir_providers::capabilities::resolve_provider_capabilities(
        &packet.provider,
        &packet.model,
    )
    .map(|resolved| resolved.snapshot_ref)
    .map_err(anyhow::Error::msg)?;
    if !mimir_providers::capabilities::snapshot_refs_match(
        &current,
        &packet.capability_snapshot_ref,
    ) {
        bail!(
            "capability snapshot mismatch: packet snapshot {} does not match current {} for {}/{}",
            packet.capability_snapshot_ref,
            current,
            packet.provider,
            packet.model
        );
    }
    Ok(())
}

/// Verify packet integrity and provider capability preconditions before replay.
pub fn verify_packet_replay_preconditions(packet: &ContextPacket) -> Result<()> {
    verify_packet_integrity(packet)?;
    verify_capability_snapshot_ref(packet)
}

/// Build the replayable user prompt for a packet using files under `workspace_root`.
pub fn context_prompt_for_packet(
    workspace_root: &Utf8Path,
    packet: &ContextPacket,
) -> Result<String> {
    verify_packet_replay_preconditions(packet)?;

    let mut prompt = String::new();
    prompt.push_str("Task:\n");
    prompt.push_str(&packet.task_card.goal);
    if !packet.task_card.acceptance_criteria.is_empty() {
        prompt.push_str("\nAcceptance criteria:\n");
        for criterion in &packet.task_card.acceptance_criteria {
            prompt.push_str(&format!("- {criterion}\n"));
        }
    }
    prompt.push_str("\n\nContext packet:\n");
    prompt.push_str(&format!(
        "packet_id={} packet_hash={} run_id={}\n",
        packet.packet_id, packet.packet_hash, packet.run_id
    ));

    if !packet.included.is_empty() {
        prompt.push_str("\nIncluded context:\n");
        for item in &packet.included {
            let content = included_item_content(workspace_root, item)?;
            prompt.push_str(&format!(
                "\n--- {} ({}; {}; {} tokens) ---\n{}\n",
                item.path, item.candidate_kind, item.reason_code, item.tokens, content
            ));
        }
    }

    if !packet.omitted_candidates.is_empty() {
        prompt.push_str("\nOmitted candidates:\n");
        for item in &packet.omitted_candidates {
            prompt.push_str(&format!(
                "- {} omitted because {} ({} tokens)\n",
                item.path, item.reason_for_omission, item.estimated_tokens
            ));
        }
    }

    Ok(prompt)
}

/// Build the default replay provider request for a packet.
pub fn provider_request_from_packet(
    workspace_root: &Utf8Path,
    packet: &ContextPacket,
    stream: bool,
) -> Result<ProviderRequest> {
    Ok(provider_request_with_prompt(
        packet,
        "You are Mimir, a careful coding-agent assistant. Answer directly and use the supplied replayable context when it is relevant.",
        context_prompt_for_packet(workspace_root, packet)?,
        packet.output_reserve_tokens,
        stream,
    ))
}

/// Build a provider request from a packet and caller-supplied prompt.
pub fn provider_request_with_prompt(
    packet: &ContextPacket,
    system: &str,
    user_prompt: String,
    max_tokens: u32,
    stream: bool,
) -> ProviderRequest {
    ProviderRequest {
        model: packet.model.clone(),
        system: Some(system.to_string()),
        messages: vec![ProviderMessage {
            role: "user".to_string(),
            content: user_prompt,
        }],
        tools: None,
        max_tokens: Some(max_tokens.min(packet.output_reserve_tokens)),
        temperature: Some(0.0),
        stream: Some(stream),
        stop_sequences: None,
        extra: provider_extra(packet),
    }
}

#[derive(Debug)]
struct LoadedPacket {
    packet: ContextPacket,
    run_dir: Utf8PathBuf,
    packet_path: Utf8PathBuf,
    workspace_root: Utf8PathBuf,
}

fn load_replayable_packet(workspace_root: &Utf8Path, run_id: &str) -> Result<LoadedPacket> {
    let workspace_root = canonical_utf8(workspace_root)?;
    let run_dir = safe_run_dir(&workspace_root, run_id)?;
    let packet_path = run_dir.join("context_packet.json");
    let data = read_regular_artifact_bytes(
        &packet_path,
        "context packet",
        MAX_CONTEXT_PACKET_ARTIFACT_BYTES,
    )
    .map_err(|error| {
        if error.to_string().contains("does not exist") {
            anyhow!("No packet found for run {run_id}")
        } else {
            error
        }
    })?;
    let packet: ContextPacket = serde_json::from_slice(&data)
        .with_context(|| format!("context packet '{}' is invalid JSON", packet_path))?;
    verify_packet_run_id(&packet, run_id)?;
    verify_packet_replay_preconditions(&packet)?;
    verify_included_source_hashes(&workspace_root, &packet)?;
    Ok(LoadedPacket {
        packet,
        run_dir,
        packet_path,
        workspace_root,
    })
}

fn replay_for_loaded_packet(
    loaded: &LoadedPacket,
) -> Result<(SharedPacketReplay, ReplayRequestSource)> {
    let provider_request_path = loaded.run_dir.join("provider_request.redacted.json");
    if optional_regular_artifact_exists(&provider_request_path, "provider request artifact")? {
        return Ok((
            shared_replay_from_provider_request_artifact(&provider_request_path)?,
            ReplayRequestSource::SavedArtifact,
        ));
    }
    let request = provider_request_from_packet(&loaded.workspace_root, &loaded.packet, false)?;
    Ok((
        shared_replay_from_request(&request)?,
        ReplayRequestSource::Reconstructed,
    ))
}

fn redacted_json_value(value: &impl Serialize) -> Result<serde_json::Value> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    Ok(json)
}

fn ensure_context_packet_safe_to_share(packet: &ContextPacket) -> Result<()> {
    let packet_json = serde_json::to_value(packet)?;
    if redacted_value(packet_json.clone()) != packet_json {
        bail!("context packet contains secret-like text; refusing to create portable share bundle");
    }
    Ok(())
}

fn ensure_json_safe_to_share(value: &serde_json::Value, label: &str) -> Result<()> {
    if redacted_value(value.clone()) != *value {
        bail!("{label} contains secret-like text; refusing to share it");
    }
    Ok(())
}

fn user_prompt_sha256(request: &serde_json::Value) -> Option<String> {
    let messages = request.get("messages")?.as_array()?;
    let user_message = messages
        .iter()
        .find(|message| message.get("role").and_then(|role| role.as_str()) == Some("user"))?;
    let content = user_message.get("content")?.as_str()?;
    Some(sha256_hex(content.as_bytes()))
}

fn shared_replay_from_request(request: &ProviderRequest) -> Result<SharedPacketReplay> {
    let provider_request_redacted = redacted_json_value(request)?;
    shared_replay_from_redacted_request(provider_request_redacted)
}

fn shared_replay_from_redacted_request(
    provider_request_redacted: serde_json::Value,
) -> Result<SharedPacketReplay> {
    ensure_json_safe_to_share(&provider_request_redacted, "provider request artifact")?;
    let request_bytes = serde_json::to_vec_pretty(&provider_request_redacted)?;
    if request_bytes.len() > MAX_PROVIDER_REQUEST_ARTIFACT_BYTES {
        bail!(
            "provider_request artifact shared packet bundle exceeds size cap ({} bytes > {} bytes)",
            request_bytes.len(),
            MAX_PROVIDER_REQUEST_ARTIFACT_BYTES
        );
    }
    Ok(SharedPacketReplay {
        provider_request_sha256: sha256_hex(&request_bytes),
        user_prompt_sha256: user_prompt_sha256(&provider_request_redacted),
        provider_request_redacted,
    })
}

fn shared_replay_from_provider_request_artifact(path: &Utf8Path) -> Result<SharedPacketReplay> {
    let data = redacted_provider_request_artifact_bytes(path)?;
    let request: serde_json::Value = serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "provider request artifact '{}' is invalid JSON: {err}",
            path
        )
    })?;
    shared_replay_from_redacted_request(request)
}

fn redacted_provider_request_artifact_bytes(path: &Utf8Path) -> Result<Vec<u8>> {
    let data = read_regular_artifact_bytes(
        path,
        "provider request artifact",
        MAX_PROVIDER_REQUEST_ARTIFACT_BYTES,
    )?;
    let request: serde_json::Value = serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "provider request artifact '{}' is invalid JSON: {err}",
            path
        )
    })?;
    ensure_json_safe_to_share(&request, "provider request artifact")?;
    Ok(data)
}

fn build_shared_packet_bundle(
    packet: &ContextPacket,
    run_id: &str,
    replay: SharedPacketReplay,
) -> Result<SharedPacketBundle> {
    verify_packet_run_id(packet, run_id)?;
    verify_packet_replay_preconditions(packet)?;
    ensure_context_packet_safe_to_share(packet)?;
    Ok(SharedPacketBundle {
        schema_version: PACKET_SHARE_BUNDLE_SCHEMA_VERSION,
        kind: PACKET_SHARE_BUNDLE_KIND.to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        run_id: run_id.to_string(),
        packet_hash: packet.packet_hash.clone(),
        provider: packet.provider.clone(),
        model: packet.model.clone(),
        packet: packet.clone(),
        replay,
    })
}

fn verify_shared_packet_bundle(bundle: &SharedPacketBundle) -> Result<()> {
    if bundle.schema_version != PACKET_SHARE_BUNDLE_SCHEMA_VERSION {
        bail!(
            "unsupported shared packet bundle schema_version {}",
            bundle.schema_version
        );
    }
    if bundle.kind != PACKET_SHARE_BUNDLE_KIND {
        bail!(
            "shared packet bundle kind mismatch: expected {}, got {}",
            PACKET_SHARE_BUNDLE_KIND,
            bundle.kind
        );
    }
    verify_packet_integrity(&bundle.packet)?;
    verify_packet_run_id(&bundle.packet, &bundle.run_id)?;
    ensure_context_packet_safe_to_share(&bundle.packet)?;
    ensure_json_safe_to_share(
        &bundle.replay.provider_request_redacted,
        "shared packet bundle provider request",
    )?;
    if bundle.packet_hash != bundle.packet.packet_hash {
        bail!(
            "shared packet bundle packet_hash mismatch: declared {}, packet {}",
            bundle.packet_hash,
            bundle.packet.packet_hash
        );
    }
    if bundle.provider != bundle.packet.provider || bundle.model != bundle.packet.model {
        bail!("shared packet bundle provider/model metadata mismatch");
    }
    let request_bytes = serde_json::to_vec_pretty(&bundle.replay.provider_request_redacted)?;
    let actual_request_sha256 = sha256_hex(&request_bytes);
    if bundle.replay.provider_request_sha256 != actual_request_sha256 {
        bail!(
            "shared packet bundle provider_request_sha256 mismatch: declared {}, actual {}",
            bundle.replay.provider_request_sha256,
            actual_request_sha256
        );
    }
    if let Some(expected_prompt_sha256) = &bundle.replay.user_prompt_sha256 {
        let actual_prompt_sha256 = user_prompt_sha256(&bundle.replay.provider_request_redacted)
            .ok_or_else(|| {
                anyhow!("shared packet bundle request is missing user prompt content")
            })?;
        if expected_prompt_sha256 != &actual_prompt_sha256 {
            bail!(
                "shared packet bundle user_prompt_sha256 mismatch: declared {}, actual {}",
                expected_prompt_sha256,
                actual_prompt_sha256
            );
        }
    }
    Ok(())
}

fn included_item_content(
    workspace_root: &Utf8Path,
    item: &mimir_schemas::IncludedItem,
) -> Result<String> {
    let path = safe_workspace_file(workspace_root, &item.path)?;
    let bytes = fs::read(&path)
        .map_err(|err| anyhow!("included context '{}' could not be read: {err}", item.path))?;
    let actual_hash = sha256_hex(&bytes);
    if actual_hash != item.source_hash {
        bail!(
            "included context '{}' source_hash mismatch: declared {}, actual {}",
            item.path,
            item.source_hash,
            actual_hash
        );
    }
    let content = String::from_utf8(bytes)
        .map_err(|err| anyhow!("included context '{}' is not valid UTF-8: {err}", item.path))?;
    if contains_secret_like_text(&content) {
        bail!(
            "included context '{}' secret_risk: file contains secret-like content",
            item.path
        );
    }

    // If the packet carries compression metadata, deterministically re-compress
    // the verified original so the provider request matches the token count
    // recorded in the packet.
    if let Some(ref compression) = item.compression {
        let language =
            mimir_index::detect_language(std::path::Path::new(&item.path), Some(&content));
        let compressed = mimir_compress::compress_body(
            &content,
            &language,
            0,
            mimir_providers::count::count_local,
        );
        // Verify the deterministic hash matches what the builder recorded.
        if compressed.original_hash != compression.original_hash {
            bail!(
                "included context '{}' compression hash mismatch: declared {}, actual {}",
                item.path,
                compression.original_hash,
                compressed.original_hash
            );
        }
        return Ok(compressed.text);
    }

    Ok(content)
}

fn verify_included_source_hashes(workspace_root: &Utf8Path, packet: &ContextPacket) -> Result<()> {
    for item in &packet.included {
        included_item_content(workspace_root, item)?;
    }
    Ok(())
}

fn provider_extra(packet: &ContextPacket) -> Option<HashMap<String, serde_json::Value>> {
    let mut extra = HashMap::new();
    if matches!(packet.provider.as_str(), "glm" | "zai") {
        extra.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "disabled" }),
        );
    }
    if extra.is_empty() {
        None
    } else {
        Some(extra)
    }
}

fn safe_run_dir(workspace_root: &Utf8Path, run_id: &str) -> Result<Utf8PathBuf> {
    let run_id = RunId::parse(run_id.to_string()).map_err(|_| anyhow!("invalid run id"))?;
    let mimir_root = workspace_root.join(".mimir");
    let run_dir = RunDir::open(&mimir_root, &run_id).map_err(|error| match error.kind() {
        ErrorKind::NotFound => anyhow!("No packet found for run {run_id}"),
        ErrorKind::InvalidInput => anyhow!("invalid run id"),
        ErrorKind::PermissionDenied => anyhow!("run path escapes workspace"),
        _ => anyhow!("run directory could not be opened: {error}"),
    })?;
    Ok(run_dir.root().clone())
}

fn optional_regular_artifact_exists(path: &Utf8Path, label: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => bail!("{label} '{path}' is not a regular file"),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(anyhow!("{label} '{path}' could not be inspected: {error}")),
    }
}

fn read_regular_artifact_bytes(path: &Utf8Path, label: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        anyhow!("{label} '{path}' does not exist or could not be inspected: {error}")
    })?;
    if !metadata.file_type().is_file() {
        bail!("{label} '{path}' is not a regular file");
    }
    if metadata.len() > max_bytes as u64 {
        bail!(
            "{label} '{}' exceeds size cap ({} bytes > {} bytes)",
            path,
            metadata.len(),
            max_bytes
        );
    }
    fs::read(path).map_err(|error| anyhow!("{label} '{path}' could not be read: {error}"))
}

fn safe_workspace_file(workspace_root: &Utf8Path, relative: &str) -> Result<Utf8PathBuf> {
    if relative.is_empty() || relative.contains('\0') {
        bail!("invalid included context path");
    }
    let mut path = workspace_root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment
                    .to_str()
                    .ok_or_else(|| anyhow!("included context path is not UTF-8"))?;
                path.push(segment);
            }
            _ => bail!("included context path escapes workspace"),
        }
    }
    if !path.is_file() {
        bail!("included context '{}' could not be read", relative);
    }
    let canonical_root = canonical_utf8(workspace_root)?;
    let canonical_path = canonical_utf8(&path)?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("included context path escapes workspace");
    }
    Ok(canonical_path)
}

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(
        fs::canonicalize(path).with_context(|| format!("failed to canonicalize {}", path))?,
    )
    .map_err(|path| anyhow!("path is not UTF-8: {}", path.display()))
}

fn path_to_workspace_string(workspace_root: &Utf8Path, path: &Utf8Path) -> Result<String> {
    let rel = path
        .as_std_path()
        .strip_prefix(workspace_root.as_std_path())
        .map_err(|_| anyhow!("path escapes workspace"))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn redacted_value(mut value: serde_json::Value) -> serde_json::Value {
    mimir_security::redact_json_value(&mut value);
    value
}

fn contains_secret_like_text(text: &str) -> bool {
    mimir_security::redact_secrets(text) != text
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_bundle_verification_rejects_tampered_request_digest() {
        let run_id = "20260101-120000-abcdef10";
        let mut bundle = SharedPacketBundle {
            schema_version: PACKET_SHARE_BUNDLE_SCHEMA_VERSION,
            kind: PACKET_SHARE_BUNDLE_KIND.to_string(),
            exported_at: "2026-05-28T00:00:00Z".to_string(),
            run_id: run_id.to_string(),
            packet_hash: "bad".to_string(),
            provider: "glm".to_string(),
            model: "glm-5.1".to_string(),
            packet: empty_packet(run_id),
            replay: SharedPacketReplay {
                provider_request_sha256: "wrong".to_string(),
                user_prompt_sha256: None,
                provider_request_redacted: serde_json::json!({"messages": []}),
            },
        };
        bundle.packet_hash = mimir_context::hash_packet(&bundle.packet);
        bundle.packet.packet_hash = bundle.packet_hash.clone();

        let error = verify_shared_packet_bundle(&bundle)
            .expect_err("tampered provider request digest must fail")
            .to_string();
        assert!(error.contains("provider_request_sha256 mismatch"));
    }

    #[test]
    fn shared_replay_rejects_secret_like_redacted_request() {
        let error = shared_replay_from_redacted_request(serde_json::json!({
            "api_key": "sk-123456789012345678901234"
        }))
        .expect_err("secret-like request must fail")
        .to_string();
        assert!(!error.contains("sk-123456789012345678901234"));
    }

    #[test]
    fn saved_provider_request_artifact_respects_size_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 tempdir");
        let run_id = "20260101-120000-abcdef11";
        let run_dir = write_empty_run(&root, run_id);
        fs::write(
            run_dir.join("provider_request.redacted.json"),
            vec![b' '; MAX_PROVIDER_REQUEST_ARTIFACT_BYTES + 1],
        )
        .expect("oversized provider request");

        let error = replay_request_bytes_for_run(&root, run_id)
            .expect_err("oversized provider request must fail")
            .to_string();
        assert!(error.contains("exceeds size cap"));
    }

    #[test]
    fn replay_rejects_invalid_run_ids_without_creating_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 tempdir");
        for run_id in [
            "../20260101-120000-abcdef01",
            "/tmp/20260101-120000-abcdef01",
            "%2e%2e",
            "run-1",
        ] {
            let error = replay_request_preview_for_run(&root, run_id)
                .expect_err("invalid run id must fail")
                .to_string();
            assert!(error.contains("invalid run id"), "{error}");
        }
        assert!(!root.join(".mimir/runs").exists());
    }

    #[test]
    fn packet_only_share_rejects_secret_like_packet_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 tempdir");
        let run_id = "20260101-120000-abcdef14";
        let run_dir = write_empty_run(&root, run_id);
        let packet_path = run_dir.join("context_packet.json");
        let mut packet: ContextPacket =
            serde_json::from_slice(&fs::read(&packet_path).expect("packet bytes"))
                .expect("packet json");
        packet.task_card.goal = "api_key=sk-123456789012345678901234".to_string();
        packet.packet_hash = mimir_context::hash_packet(&packet);
        fs::write(
            &packet_path,
            serde_json::to_vec_pretty(&packet).expect("packet json"),
        )
        .expect("write tampered packet");

        let error = redacted_packet_only_bytes_for_run(&root, run_id)
            .expect_err("secret-like packet metadata must fail")
            .to_string();
        assert!(error.contains("context packet contains secret-like text"));
        assert!(!error.contains("sk-123456789012345678901234"));
    }

    #[cfg(unix)]
    #[test]
    fn replay_rejects_symlink_run_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 tempdir");
        let run_id = "20260101-120000-abcdef13";
        let runs_root = root.join(".mimir/runs");
        let outside = root.join("outside-run");
        fs::create_dir_all(&runs_root).expect("runs root");
        fs::create_dir_all(&outside).expect("outside run");
        fs::write(
            outside.join("context_packet.json"),
            serde_json::to_vec_pretty(&empty_packet(run_id)).expect("packet json"),
        )
        .expect("outside packet");
        std::os::unix::fs::symlink(outside.as_std_path(), runs_root.join(run_id))
            .expect("run symlink");

        let error = replay_request_preview_for_run(&root, run_id)
            .expect_err("symlink run must fail")
            .to_string();
        assert!(error.contains("run path escapes workspace"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn saved_provider_request_artifact_rejects_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 tempdir");
        let run_id = "20260101-120000-abcdef12";
        let run_dir = write_empty_run(&root, run_id);
        let outside = root.join("outside-request.json");
        fs::write(&outside, r#"{"messages":[]}"#).expect("outside request");
        std::os::unix::fs::symlink(outside, run_dir.join("provider_request.redacted.json"))
            .expect("provider request symlink");

        let error = replay_request_preview_for_run(&root, run_id)
            .expect_err("symlink provider request must fail")
            .to_string();
        assert!(error.contains("not a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn read_shared_packet_bundle_rejects_symlinked_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A valid regular bundle file the symlink points at — proves the
        // rejection is driven by the symlink, not by bad target content.
        let real = dir.path().join("real-bundle.json");
        fs::write(&real, br#"{"kind":"mimir.packet_share"}"#).expect("write real bundle");
        let link = dir.path().join("bundle-link.json");
        std::os::unix::fs::symlink(&real, &link).expect("symlink bundle");

        let error = read_shared_packet_bundle(&link)
            .expect_err("symlinked bundle must be rejected")
            .to_string();
        assert!(error.contains("is not a regular file"), "{error}");
    }

    #[test]
    fn read_shared_packet_bundle_rejects_oversized_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("huge-bundle.json");
        fs::write(&path, vec![b' '; MAX_SHARED_BUNDLE_ARTIFACT_BYTES + 1])
            .expect("write oversized bundle");

        let error = read_shared_packet_bundle(&path)
            .expect_err("oversized bundle must be rejected")
            .to_string();
        assert!(error.contains("exceeds size cap"), "{error}");
    }

    fn write_empty_run(root: &Utf8Path, run_id: &str) -> Utf8PathBuf {
        let run_dir = root.join(".mimir/runs").join(run_id);
        fs::create_dir_all(&run_dir).expect("run dir");
        fs::write(
            run_dir.join("context_packet.json"),
            serde_json::to_vec_pretty(&empty_packet(run_id)).expect("packet json"),
        )
        .expect("context packet");
        run_dir
    }

    fn empty_packet(run_id: &str) -> ContextPacket {
        let mut packet = mimir_context::ContextBuilder::new()
            .run_id(RunId(run_id.to_string()))
            .task_card("packet test")
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();
        packet.packet_hash = mimir_context::hash_packet(&packet);
        packet
    }
}
