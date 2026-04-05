// Copyright 2024 wrkflw contributors
// SPDX-License-Identifier: MIT

//! Benchmarks for secret masking performance

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use wrkflw_secrets::SecretMasker;

fn bench_basic_masking(c: &mut Criterion) {
    let masker = SecretMasker::new();
    masker.add_secret("password123");
    masker.add_secret("api_key_abcdef123456");
    masker.add_secret("super_secret_value_that_should_be_masked");

    let text = "The password is password123 and the API key is api_key_abcdef123456. Also super_secret_value_that_should_be_masked is here.";

    c.bench_function("basic_masking", |b| b.iter(|| masker.mask(black_box(text))));
}

fn bench_pattern_masking(c: &mut Criterion) {
    let masker = SecretMasker::new();

    let text = "GitHub token: ghp_1234567890123456789012345678901234567890 and AWS key: AKIAIOSFODNN7EXAMPLE";

    c.bench_function("pattern_masking", |b| {
        b.iter(|| masker.mask(black_box(text)))
    });
}

fn bench_large_text_masking(c: &mut Criterion) {
    let masker = SecretMasker::new();
    masker.add_secret("secret123");
    masker.add_secret("password456");

    // Create a large text with secrets scattered throughout
    let mut large_text = String::new();
    for i in 0..1000 {
        large_text.push_str(&format!(
            "Line {}: Some normal text here with secret123 and password456 mixed in. ",
            i
        ));
    }

    c.bench_function("large_text_masking", |b| {
        b.iter(|| masker.mask(black_box(&large_text)))
    });
}

fn bench_many_secrets(c: &mut Criterion) {
    let masker = SecretMasker::new();

    // Add many secrets
    for i in 0..100 {
        masker.add_secret(format!("secret_{}", i));
    }

    let text = "This text contains secret_50 and secret_75 but not others.";

    c.bench_function("many_secrets", |b| b.iter(|| masker.mask(black_box(text))));
}

fn bench_contains_secrets(c: &mut Criterion) {
    let masker = SecretMasker::new();
    masker.add_secret("password123");
    masker.add_secret("api_key_abcdef123456");

    let text_with_secrets = "The password is password123";
    let text_without_secrets = "Just some normal text";
    let text_with_patterns = "GitHub token: ghp_1234567890123456789012345678901234567890";

    c.bench_function("contains_secrets_with", |b| {
        b.iter(|| masker.contains_secrets(black_box(text_with_secrets)))
    });

    c.bench_function("contains_secrets_without", |b| {
        b.iter(|| masker.contains_secrets(black_box(text_without_secrets)))
    });

    c.bench_function("contains_secrets_patterns", |b| {
        b.iter(|| masker.contains_secrets(black_box(text_with_patterns)))
    });
}

criterion_group!(
    benches,
    bench_basic_masking,
    bench_pattern_masking,
    bench_large_text_masking,
    bench_many_secrets,
    bench_contains_secrets
);
criterion_main!(benches);
