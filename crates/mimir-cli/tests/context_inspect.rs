use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Path to a checked-in, schema-valid context packet fixture that contains
/// included items with line ranges and an omitted candidate with a reason.
const EXAMPLE_PACKET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/context-packet.example.json"
);

fn write_fixture_packet(dir: &TempDir) -> std::path::PathBuf {
    let data = std::fs::read_to_string(EXAMPLE_PACKET)
        .expect("example context packet fixture should exist");
    // Sanity-check the fixture really carries what the test asserts on, so the
    // test fails loudly if the fixture is ever stripped down.
    let packet: Value = serde_json::from_str(&data).unwrap();
    assert!(
        !packet["included"].as_array().unwrap().is_empty(),
        "fixture must include at least one item with ranges"
    );
    assert!(
        !packet["omitted_candidates"].as_array().unwrap().is_empty(),
        "fixture must include at least one omitted candidate"
    );
    let path = dir.path().join("packet.json");
    std::fs::write(&path, data).unwrap();
    path
}

#[test]
fn inspect_shows_included_ranges_and_omitted_reasons() {
    let dir = TempDir::new().unwrap();
    let packet_path = write_fixture_packet(&dir);

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .args(["context", "inspect", packet_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    // Existing summary line must remain.
    assert!(
        stdout.contains("tokens, hash"),
        "summary line missing:\n{stdout}"
    );

    // Section headers prove ranges + omitted-with-reasons are surfaced.
    assert!(
        stdout.contains("Included ("),
        "missing Included section:\n{stdout}"
    );
    assert!(
        stdout.contains("Omitted ("),
        "missing Omitted section:\n{stdout}"
    );

    // (a) An included path WITH a range. The fixture's src/auth/session.ts
    // carries the range 40-140.
    assert!(
        stdout.contains("src/auth/session.ts"),
        "included path missing:\n{stdout}"
    );
    assert!(
        stdout.contains("40-140"),
        "included path range missing:\n{stdout}"
    );

    // (b) An omitted path WITH its reason_for_omission text.
    assert!(
        stdout.contains("src/auth/legacy/old-session.ts"),
        "omitted path missing:\n{stdout}"
    );
    assert!(
        stdout.contains("lower_relevance_score"),
        "omitted reason_for_omission text missing:\n{stdout}"
    );

    // Guard: must NOT have reverted to summary-only output.
    assert!(
        stdout.lines().count() > 1,
        "inspect collapsed to summary-only output:\n{stdout}"
    );
}

#[test]
fn inspect_json_emits_included_ranges_and_omitted_reasons() {
    let dir = TempDir::new().unwrap();
    let packet_path = write_fixture_packet(&dir);

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .args([
            "context",
            "inspect",
            packet_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("--json output must be parseable JSON");

    let included = value["included"].as_array().expect("included array");
    let session = included
        .iter()
        .find(|item| item["path"] == "src/auth/session.ts")
        .expect("included src/auth/session.ts present");
    let ranges = session["ranges"].as_array().expect("included ranges array");
    assert!(!ranges.is_empty(), "included item must carry ranges");
    assert_eq!(ranges[0]["start"], 40);
    assert_eq!(ranges[0]["end"], 140);

    let omitted = value["omitted"].as_array().expect("omitted array");
    let legacy = omitted
        .iter()
        .find(|item| item["path"] == "src/auth/legacy/old-session.ts")
        .expect("omitted candidate present");
    assert_eq!(legacy["reason_for_omission"], "lower_relevance_score");
}
