//! Exit-gate lock: "All P0 schemas validate".
//!
//! This durable test guards the normative P0 contract surface:
//!   1. Every `schemas/*.schema.json` is valid JSON and compiles as a
//!      JSON Schema 2020-12 validator.
//!   2. Every `examples/*.example.{json,yaml}` validates with ZERO errors
//!      against the schema named by its PascalCase `title`.
//!   3. Schemas and examples are 1:1 — each schema has exactly one example,
//!      modulo the explicit `SCHEMAS_WITHOUT_EXAMPLE` allow-list below.
//!
//! Counts are NOT hardcoded: the test discovers `schemas/` and `examples/`
//! at run time and only asserts `count > 0` plus the 1:1 coverage invariant,
//! so the gate stays correct as the schema set evolves.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::{Resource, Retrieve, Uri, ValidationOptions, Validator};

/// Schemas that legitimately ship without a matching example.
///
/// Each entry is a schema `title`. Empty today — every P0 schema has a
/// corresponding example. Add a title here (with a justifying comment) only
/// if a schema is intentionally example-less; otherwise the 1:1 coverage
/// assertion will fail and force the example to be written.
const SCHEMAS_WITHOUT_EXAMPLE: &[&str] = &[
    // (none) — all P0 schemas currently have a 1:1 example.
];

/// Workspace root, two levels up from `CARGO_MANIFEST_DIR`
/// (`crates/mimir-schemas` -> workspace root).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above CARGO_MANIFEST_DIR")
        .to_path_buf()
}

/// Returns the sorted list of files in `dir` whose names end with `suffix`.
fn files_with_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot read dir {}: {err}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect();
    out.sort();
    out
}

/// Parse a `*.example.{json,yaml}` file into a JSON value.
fn parse_example(path: &Path) -> serde_json::Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("cannot read example {}: {err}", path.display()));
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    match ext {
        "yaml" | "yml" => serde_yaml::from_str(&raw)
            .unwrap_or_else(|err| panic!("example {} is not valid YAML: {err}", path.display())),
        _ => serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("example {} is not valid JSON: {err}", path.display())),
    }
}

/// Retriever that refuses every external fetch.
///
/// All P0 schemas are registered in-memory as resources keyed by their `$id`,
/// so cross-`$ref`s resolve locally and this retriever is never legitimately
/// invoked. It exists as a hard guard: if compilation ever tries to fetch an
/// unregistered URI over the network, it fails loudly instead of phoning home.
struct OfflineRetriever;

impl Retrieve for OfflineRetriever {
    fn retrieve(
        &self,
        uri: &Uri<&str>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!(
            "refusing to fetch external schema resource {uri} \
             (all P0 schemas must be registered in-memory; no network in tests)"
        )
        .into())
    }
}

/// Build `ValidationOptions` with every schema registered as an in-memory
/// resource under its `$id`, plus the offline retriever guard. Cross-schema
/// `$ref`s (e.g. `CandidateManifest` -> `ContextCandidate`) resolve against
/// these resources with zero network access.
fn build_options(schemas: &BTreeMap<String, (PathBuf, serde_json::Value)>) -> ValidationOptions {
    let mut options = jsonschema::options();
    options.with_retriever(OfflineRetriever);
    for (path, value) in schemas.values() {
        let id = value
            .get("$id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("schema {} has no string `$id`", path.display()))
            .to_string();
        let resource = Resource::from_contents(value.clone()).unwrap_or_else(|err| {
            panic!(
                "schema {} could not be loaded as a resource: {err}",
                path.display()
            )
        });
        options.with_resource(id, resource);
    }
    options
}

/// Compile a single schema through the shared registry, panicking with the
/// offending path on failure.
fn compile(options: &ValidationOptions, path: &Path, value: &serde_json::Value) -> Validator {
    options.build(value).unwrap_or_else(|err| {
        panic!(
            "schema {} did not compile as a JSON Schema validator: {err}",
            path.display()
        )
    })
}

/// Discover all schemas, returning `title -> (path, parsed JSON)`.
///
/// Parsing only — compilation is deferred to [`compile`] so cross-schema
/// `$ref`s can resolve against the full registry built by [`build_options`].
fn load_schemas(schemas_dir: &Path) -> BTreeMap<String, (PathBuf, serde_json::Value)> {
    let schema_paths = files_with_suffix(schemas_dir, ".schema.json");
    assert!(
        !schema_paths.is_empty(),
        "no schemas discovered in {} (path drift?)",
        schemas_dir.display()
    );

    let mut by_title: BTreeMap<String, (PathBuf, serde_json::Value)> = BTreeMap::new();
    for path in schema_paths {
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read schema {}: {err}", path.display()));
        let value: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("schema {} is not valid JSON: {err}", path.display()));

        let title = value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("schema {} has no string `title`", path.display()))
            .to_string();

        if let Some((existing, _)) = by_title.insert(title.clone(), (path.clone(), value)) {
            panic!(
                "duplicate schema title {title:?}: {} and {}",
                existing.display(),
                path.display()
            );
        }
    }
    by_title
}

#[test]
fn all_p0_schemas_validate() {
    let root = workspace_root();
    let schemas_dir = root.join("schemas");
    let examples_dir = root.join("examples");

    // (1) Load + compile every schema. Schemas cross-reference each other by
    // `$id` URI, so they are registered as in-memory resources and compiled
    // through a shared, network-free `ValidationOptions`.
    let schemas = load_schemas(&schemas_dir);
    let schema_count = schemas.len();
    assert!(schema_count > 0, "discovered schema count must be > 0");

    let options = build_options(&schemas);
    for (path, value) in schemas.values() {
        let _validator = compile(&options, path, value);
    }

    // (2) Validate every example against the schema named by its title.
    let example_paths = files_with_suffix(&examples_dir, ".example.json")
        .into_iter()
        .chain(files_with_suffix(&examples_dir, ".example.yaml"))
        .chain(files_with_suffix(&examples_dir, ".example.yml"))
        .collect::<Vec<_>>();
    let example_count = example_paths.len();
    assert!(
        example_count > 0,
        "no examples discovered in {} (path drift?)",
        examples_dir.display()
    );

    let mut covered_titles: BTreeSet<String> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

    for example_path in &example_paths {
        // `foo-bar.example.json` -> kebab `foo-bar` -> PascalCase `FooBar`.
        let file_name = example_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("example file name");
        let stem = file_name
            .split_once(".example.")
            .map(|(stem, _ext)| stem)
            .unwrap_or_else(|| panic!("unexpected example file name: {file_name}"));
        let title = kebab_to_pascal(stem);

        let Some((schema_path, schema_value)) = schemas.get(&title) else {
            failures.push(format!(
                "example {} maps to title {title:?} but no such schema exists",
                example_path.display()
            ));
            continue;
        };

        let example_value = parse_example(example_path);
        let validator = compile(&options, schema_path, schema_value);
        let errors: Vec<String> = validator
            .iter_errors(&example_value)
            .map(|error| format!("    at {}: {error}", error.instance_path))
            .collect();
        if errors.is_empty() {
            covered_titles.insert(title);
        } else {
            failures.push(format!(
                "example {} failed validation against {}:\n{}",
                example_path.display(),
                schema_path.display(),
                errors.join("\n")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} example(s) failed P0 schema validation:\n{}",
        failures.len(),
        failures.join("\n\n")
    );

    // (3) 1:1 coverage: every schema (minus the allow-list) has an example.
    let allowed: BTreeSet<&str> = SCHEMAS_WITHOUT_EXAMPLE.iter().copied().collect();
    let mut uncovered: Vec<String> = schemas
        .keys()
        .filter(|title| !covered_titles.contains(*title) && !allowed.contains(title.as_str()))
        .cloned()
        .collect();
    uncovered.sort();
    assert!(
        uncovered.is_empty(),
        "schema(s) without a validating example (add to SCHEMAS_WITHOUT_EXAMPLE only if intentional): {uncovered:?}"
    );

    // Guard the allow-list against rot: every allowed title must be a real schema.
    let stale_allow: Vec<&str> = SCHEMAS_WITHOUT_EXAMPLE
        .iter()
        .copied()
        .filter(|title| !schemas.contains_key(*title))
        .collect();
    assert!(
        stale_allow.is_empty(),
        "SCHEMAS_WITHOUT_EXAMPLE references non-existent schema title(s): {stale_allow:?}"
    );

    println!(
        "P0 schema gate: {schema_count} schema(s) compiled, {example_count} example(s) validated, \
         {} schema(s) allow-listed without example.",
        SCHEMAS_WITHOUT_EXAMPLE.len()
    );
}

/// Convert a kebab-case identifier (`provider-capabilities-list`) to
/// PascalCase (`ProviderCapabilitiesList`).
fn kebab_to_pascal(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
