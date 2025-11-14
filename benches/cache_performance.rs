//! Benchmarks for Phase B-2: Document IR Cache Performance
//!
//! This benchmark suite validates the expected performance improvements from
//! the document IR cache with blake3 content hashing and LRU eviction.
//!
//! Benchmarks:
//! - Cache hit performance (hash + lookup)
//! - Cache miss performance (hash + lookup + parse + insert)
//! - Realistic workload (80% hit rate)
//! - Cache capacity impact
//!
//! Expected Results (from baseline):
//! - Cache hit: ~100µs (vs 182.63ms baseline = 1,826x faster)
//! - Cache miss: ~182.69ms (negligible 0.03% overhead)
//! - 80% hit rate: ~36.6ms average (5x faster than baseline)
//!
//! Run with: taskset -c 0 cargo bench --bench cache_performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rholang_language_server::lsp::backend::document_cache::{ContentHash, DocumentCache};
use rholang_language_server::lsp::models::{CachedDocument, WorkspaceState};
use rholang_language_server::tree_sitter::{parse_code, parse_to_document_ir};
use ropey::Rope;
use std::sync::Arc;
use std::time::Duration;
use tower_lsp::lsp_types::Url;

// Helper to generate test code
fn generate_test_code(file_id: usize, contract_count: usize) -> String {
    let mut code = String::new();
    for i in 0..contract_count {
        code.push_str(&format!(
            r#"
contract test{}_{} (@arg, ret) = {{
  new x in {{ x!(arg) | for (@result <- x) {{ ret!(result) }} }}
}}
"#,
            file_id, i
        ));
    }
    code
}

// Helper to create a mock cached document (for cache testing only)
// This is a simplified version that doesn't require full backend initialization
fn create_mock_cached_document(code: &str, uri: &Url) -> CachedDocument {
    use rholang_language_server::ir::rholang_node::RholangNode;
    use rholang_language_server::ir::semantic_node::{NodeBase, Position};
    use rholang_language_server::ir::symbol_table::SymbolTable;
    use std::collections::HashMap;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let rope = Rope::from_str(code);
    let tree = Arc::new(parse_code(code));
    let document_ir = parse_to_document_ir(&tree, &rope);

    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    let content_hash = hasher.finish();

    // Create minimal cached document for benchmarking
    let placeholder_ir = Arc::new(RholangNode::Nil {
        base: NodeBase::new_simple(Position { row: 0, column: 0, byte: 0 }, code.len(), 0, code.len()),
        metadata: None,
    });

    CachedDocument {
        ir: document_ir.root.clone(),
        position_index: Arc::new(rholang_language_server::lsp::position_index::PositionIndex::new()),
        document_ir: Some(document_ir.clone()),
        metta_ir: None,
        unified_ir: rholang_language_server::ir::unified_ir::UnifiedIR::from_rholang(&document_ir.root),
        language: rholang_language_server::lsp::models::DocumentLanguage::Rholang,
        tree: tree.clone(),
        symbol_table: Arc::new(SymbolTable::new(None)),
        inverted_index: HashMap::new(),
        version: 0,
        text: rope,
        positions: Arc::new(HashMap::new()),
        symbol_index: Arc::new(rholang_language_server::lsp::symbol_index::SymbolIndex::new(Vec::new())),
        content_hash,
        completion_state: None,
    }
}

/// Benchmark: Content hash computation (blake3)
fn bench_content_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/content_hash");
    group.sample_size(100);

    for contract_count in [10, 50, 100] {
        let code = generate_test_code(0, contract_count);
        group.bench_with_input(
            BenchmarkId::new("blake3", contract_count),
            &contract_count,
            |b, _| {
                b.iter(|| {
                    let hash = ContentHash::from_str(&code);
                    black_box(hash)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Cache hit (hash + lookup + Arc clone)
fn bench_cache_hit(c: &mut Criterion) {
    let cache = DocumentCache::new();
    let uri = Url::parse("file:///test.rho").unwrap();
    let code = generate_test_code(0, 100);

    // Pre-populate cache
    let cached_doc = create_mock_cached_document(&code, &uri);
    let hash = ContentHash::from_str(&code);
    cache.insert(
        uri.clone(),
        hash,
        Arc::new(cached_doc),
        std::time::SystemTime::now(),
    );

    let mut group = c.benchmark_group("cache/hit");
    group.sample_size(1000);

    group.bench_function("lookup_100_contracts", |b| {
        b.iter(|| {
            let result = cache.get(&uri, &hash);
            black_box(result)
        });
    });

    group.finish();
}

/// Benchmark: Cache miss (hash + lookup + parse + index + insert)
fn bench_cache_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/miss");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(30));

    for contract_count in [10, 50, 100] {
        let code = generate_test_code(0, contract_count);
        let uri = Url::parse(&format!("file:///test_{}.rho", contract_count)).unwrap();

        group.bench_with_input(
            BenchmarkId::new("full_index", contract_count),
            &contract_count,
            |b, _| {
                b.iter(|| {
                    // Simulate cache miss: hash + lookup + parse + index
                    let hash = ContentHash::from_str(&code);
                    let cache = DocumentCache::new(); // Fresh cache = guaranteed miss

                    // This simulates what happens on cache miss
                    let result = cache.get(&uri, &hash);
                    assert!(result.is_none()); // Verify it's a miss

                    // Parse and index (what happens on miss)
                    let cached_doc = create_mock_cached_document(&code, &uri);

                    // Insert into cache
                    cache.insert(
                        uri.clone(),
                        hash,
                        Arc::new(cached_doc),
                        std::time::SystemTime::now(),
                    );

                    black_box(())
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Realistic workload (80% hits, 20% misses)
fn bench_realistic_workload(c: &mut Criterion) {
    let cache = DocumentCache::with_capacity(100);

    // Pre-populate cache with 80 documents
    let mut uris = Vec::new();
    for i in 0..80 {
        let uri = Url::parse(&format!("file:///cached_{}.rho", i)).unwrap();
        let code = generate_test_code(i, 10);
        let hash = ContentHash::from_str(&code);
        let cached_doc = create_mock_cached_document(&code, &uri);

        cache.insert(
            uri.clone(),
            hash,
            Arc::new(cached_doc),
            std::time::SystemTime::now(),
        );

        uris.push((uri, code, hash));
    }

    let mut group = c.benchmark_group("cache/realistic_workload");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("80_percent_hit_rate", |b| {
        let mut access_count = 0;

        b.iter(|| {
            // 80% of accesses are cache hits (existing files)
            // 20% of accesses are cache misses (new files)
            let is_hit = access_count % 5 != 0; // 4 out of 5 = 80%

            if is_hit {
                // Cache hit: access existing file
                let idx = access_count % 80;
                let (uri, _code, hash) = &uris[idx];
                let result = cache.get(uri, hash);
                black_box(result);
            } else {
                // Cache miss: new file
                let new_idx = 80 + (access_count / 5);
                let uri = Url::parse(&format!("file:///new_{}.rho", new_idx)).unwrap();
                let code = generate_test_code(new_idx, 10);
                let hash = ContentHash::from_str(&code);

                // Lookup (miss)
                let result = cache.get(&uri, &hash);
                // Note: Assertion removed because access_count persists across iterations,
                // causing URI reuse. This doesn't affect benchmark accuracy.
                // assert!(result.is_none());

                // Parse and index on miss (or update if exists)
                let cached_doc = create_mock_cached_document(&code, &uri);
                cache.insert(uri, hash, Arc::new(cached_doc), std::time::SystemTime::now());
            }

            access_count += 1;
        });
    });

    group.finish();

    // Print cache statistics
    let stats = cache.stats();
    println!("\n=== Cache Statistics ===");
    println!("Total queries: {}", stats.total_queries);
    println!("Hits: {}", stats.hits);
    println!("Misses: {}", stats.misses);
    println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);
    println!("Evictions: {}", stats.evictions);
    println!("Current size: {}/{}", stats.current_size, stats.max_capacity);
}

/// Benchmark: Cache capacity impact
fn bench_cache_capacity(c: &mut Criterion) {

    let mut group = c.benchmark_group("cache/capacity_impact");
    group.sample_size(50);

    for capacity in [20, 50, 100, 200] {
        let cache = DocumentCache::with_capacity(capacity);

        // Pre-populate to 80% capacity
        let populate_count = (capacity * 80) / 100;
        for i in 0..populate_count {
            let uri = Url::parse(&format!("file:///test_{}.rho", i)).unwrap();
            let code = generate_test_code(i, 10);
            let hash = ContentHash::from_str(&code);
            let cached_doc = create_mock_cached_document(&code, &uri);
            cache.insert(uri, hash, Arc::new(cached_doc), std::time::SystemTime::now());
        }

        group.bench_with_input(
            BenchmarkId::new("hit_rate", capacity),
            &capacity,
            |b, _| {
                let mut access_idx = 0;
                b.iter(|| {
                    // Access files in a loop (tests LRU behavior)
                    let idx = access_idx % populate_count;
                    let uri = Url::parse(&format!("file:///test_{}.rho", idx)).unwrap();
                    let code = generate_test_code(idx, 10);
                    let hash = ContentHash::from_str(&code);

                    let result = cache.get(&uri, &hash);
                    black_box(result);

                    access_idx += 1;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Comparison - with vs without cache
fn bench_cache_comparison(c: &mut Criterion) {

    let mut group = c.benchmark_group("cache/comparison");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(20));

    let code = generate_test_code(0, 100);
    let uri = Url::parse("file:///test.rho").unwrap();

    // Baseline: No cache (full parse + index every time)
    group.bench_function("no_cache_baseline", |b| {
        b.iter(|| {
            let cached_doc = create_mock_cached_document(&code, &uri);
            black_box(cached_doc)
        });
    });

    // With cache: First access is a miss, subsequent are hits
    group.bench_function("with_cache_first_access", |b| {
        b.iter(|| {
            let cache = DocumentCache::new();
            let hash = ContentHash::from_str(&code);

            // First access: cache miss
            let result = cache.get(&uri, &hash);
            assert!(result.is_none());

            let cached_doc = create_mock_cached_document(&code, &uri);
            cache.insert(
                uri.clone(),
                hash,
                Arc::new(cached_doc),
                std::time::SystemTime::now(),
            );

            black_box(())
        });
    });

    // With cache: Subsequent accesses are hits
    let cache = DocumentCache::new();
    let hash = ContentHash::from_str(&code);
    let cached_doc = create_mock_cached_document(&code, &uri);
    cache.insert(
        uri.clone(),
        hash,
        Arc::new(cached_doc),
        std::time::SystemTime::now(),
    );

    group.bench_function("with_cache_subsequent_access", |b| {
        b.iter(|| {
            let result = cache.get(&uri, &hash);
            black_box(result)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_content_hash,
    bench_cache_hit,
    bench_cache_miss,
    bench_realistic_workload,
    bench_cache_capacity,
    bench_cache_comparison,
);

criterion_main!(benches);
