//! R-02: token-count drift calibration vs `count_drift_reserve_tokens`.
//!
//! The packer estimates a packet's size by summing `count_local` over each
//! chunk it appends. The provider, however, tokenizes the *whole* concatenated
//! payload as one string. Tokenization is not additive: token boundaries merge
//! (or split) at the seams between chunks, so the independent re-count of the
//! joined text drifts from the per-chunk sum used for packing.
//!
//! `TokenPolicy::count_drift_reserve_tokens` (default 512) is the headroom that
//! absorbs this drift. These tests calibrate the drift over a corpus of
//! synthetic-but-representative samples and prove:
//!   1. the absolute drift per packing stays within the 512-token reserve, and
//!   2. `available()` / `fits()` subtract the reserve so a packet packed to the
//!      boundary cannot exceed `cap_tokens` once worst-case drift is added.
//!
//! No provider calls: drift is produced purely by `count_local` (the same
//! tokenizer the gateway uses locally) over synthetic text.

use mimir_context::TokenPolicy;
use mimir_providers::count::count_local;

/// A representative packet: the ordered chunks the packer would append.
///
/// Mixes prose, source code, mixed-script Unicode, punctuation runs, and
/// whitespace/markup — the seam classes most likely to retokenize differently
/// when concatenated.
fn corpus() -> Vec<Vec<String>> {
    let prose = "The quick brown fox jumps over the lazy dog near the riverbank.";
    let code = "fn main() { let total = compute(items, 42); println!(\"{total}\"); }";
    let unicode =
        "\u{8776} butterfly; \u{0437}\u{0434}\u{0440}\u{0430}\u{0432} world; \u{1F600} ok.";
    let punct = "ABCDEFG-1234567890_=+[]{}|;:',.<>/?`~ (parenthetical) [bracketed].";
    let markup = "## Heading\n\n- item one\n- item two\n\n```rust\nlet x = 1;\n```\n";
    let mixed = "Mixed\tline\twith\ttabs and    runs   of    spaces, then text.";

    // Build several packets of differing seam counts/composition so a single
    // bad seam class cannot hide behind the others.
    let base: Vec<&str> = vec![prose, code, unicode, punct, markup, mixed];

    let mut small = Vec::new();
    for c in &base {
        small.push((*c).to_string());
    }

    // A medium packet: ~120 chunks, the size of a realistic context packet.
    let mut medium = Vec::new();
    for i in 0..120 {
        medium.push(base[i % base.len()].to_string());
    }

    // A code-heavy packet (many short seams that frequently merge).
    let mut code_heavy = Vec::new();
    for _ in 0..80 {
        code_heavy.push(code.to_string());
        code_heavy.push(punct.to_string());
    }

    // A unicode-heavy packet (multi-byte boundaries).
    let mut unicode_heavy = Vec::new();
    for _ in 0..80 {
        unicode_heavy.push(unicode.to_string());
        unicode_heavy.push(prose.to_string());
    }

    vec![small, medium, code_heavy, unicode_heavy]
}

/// The packer's estimate: sum of per-chunk local counts.
fn packing_estimate(chunks: &[String]) -> u32 {
    chunks.iter().map(|c| count_local(c)).sum()
}

/// The independent re-count: tokenize the whole concatenated payload.
fn independent_recount(chunks: &[String]) -> u32 {
    let joined: String = chunks.concat();
    count_local(&joined)
}

/// Absolute drift between the packing estimate and the independent re-count.
fn abs_drift(chunks: &[String]) -> u32 {
    packing_estimate(chunks).abs_diff(independent_recount(chunks))
}

#[test]
fn corpus_drift_is_nonzero_so_the_reserve_is_load_bearing() {
    // Sanity: if no packet in the corpus actually drifts, the calibration would
    // be vacuous and would pass even with a zero reserve. Guard against that.
    let total: u32 = corpus().iter().map(|p| abs_drift(p)).sum();
    assert!(
        total > 0,
        "corpus produced zero drift; calibration would be vacuous"
    );
}

#[test]
fn drift_per_packing_stays_within_reserve() {
    let policy = TokenPolicy::default();
    let reserve = policy.count_drift_reserve_tokens; // 512

    for (i, packet) in corpus().iter().enumerate() {
        let drift = abs_drift(packet);
        assert!(
            drift <= reserve,
            "packet {i}: drift {drift} exceeds count_drift_reserve_tokens {reserve} \
             (estimate={}, recount={})",
            packing_estimate(packet),
            independent_recount(packet),
        );
    }
}

#[test]
fn available_and_fits_subtract_the_drift_reserve() {
    let policy = TokenPolicy::default();

    // available() must leave room for BOTH output and drift reserves.
    assert_eq!(
        policy.available(),
        policy.cap_tokens - policy.output_reserve_tokens - policy.count_drift_reserve_tokens,
    );

    // A packet packed exactly to available() fits; one token more does not.
    assert!(policy.fits(policy.available()));
    assert!(!policy.fits(policy.available() + 1));

    // Boundary safety: a packet packed to available() has estimate ==
    // available(). The true provider count is estimate +/- drift with
    // |drift| <= count_drift_reserve_tokens. Even in the worst (additive)
    // direction the true total stays under cap with the output reserve intact.
    let estimate_at_boundary = policy.available();
    let worst_case_true_count = estimate_at_boundary + policy.count_drift_reserve_tokens;
    assert!(
        worst_case_true_count + policy.output_reserve_tokens <= policy.cap_tokens,
        "boundary packet + worst-case drift + output reserve must not exceed cap",
    );
}

#[test]
fn zero_reserve_would_let_a_drifting_packet_exceed_cap() {
    // This is the meaningful negative: prove the reserve is doing real work by
    // showing that setting it to 0 breaks the boundary guarantee for an
    // actually-drifting corpus packet.
    //
    // Pick the corpus packet with the largest real drift and treat its true
    // (re-counted) size as what the provider would charge for it.
    let packet = corpus()
        .into_iter()
        .max_by_key(|p| abs_drift(p))
        .expect("corpus is non-empty");
    let drift = abs_drift(&packet);
    assert!(drift > 0, "selected packet must actually drift");

    let cap = TokenPolicy::default().cap_tokens;
    let output_reserve = TokenPolicy::default().output_reserve_tokens;

    // With a proper drift reserve >= the observed drift, a packet packed to
    // available() stays under cap even after the worst-case drift is added.
    let good = TokenPolicy {
        cap_tokens: cap,
        output_reserve_tokens: output_reserve,
        count_drift_reserve_tokens: drift, // exactly enough headroom
    };
    let estimate = good.available();
    assert!(
        estimate + drift + output_reserve <= cap,
        "with a sufficient reserve, estimate + drift + output reserve must fit under cap",
    );

    // With a ZERO drift reserve, the very same boundary packet's true size
    // (estimate + drift) eats into the output reserve and pushes the request
    // over cap. This is the failure the 512-token reserve exists to prevent.
    let zero = TokenPolicy {
        cap_tokens: cap,
        output_reserve_tokens: output_reserve,
        count_drift_reserve_tokens: 0,
    };
    let estimate_zero = zero.available();
    assert!(
        estimate_zero + drift + output_reserve > cap,
        "with a zero drift reserve, a real drifting packet at the boundary must overflow cap \
         (this assertion is what makes the calibration meaningful)",
    );
}
