//! Comprehensive benchmarks for LSP operations
//!
//! Measures performance of critical LSP handlers:
//! - goto_definition (Rholang and MeTTa)
//! - references
//! - rename
//! - symbol_resolution
//! - virtual_document parsing

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use rholang_language_server::parsers::metta_parser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use rholang_language_server::ir::symbol_resolution::{
    ComposableSymbolResolver, LexicalScopeResolver, MettaPatternFilter,
    ResolutionContext, SymbolLocation, SymbolResolver,
};
use rholang_language_server::language_regions::{DetectorRegistry, VirtualDocument};
use rholang_language_server::tree_sitter;
use tower_lsp::lsp_types::{Position as LspPosition, Url};
use ropey::Rope;

// Sample MeTTa code for benchmarking
const METTA_SIMPLE: &str = r#"
(= (factorial $n)
   (if (== $n 0) 1 (* $n (factorial (- $n 1)))))

(factorial 5)
"#;

const METTA_COMPLEX: &str = r#"
;; Robot navigation system
(= (connected room1 hallway) True)
(= (connected hallway room2) True)
(= (connected hallway kitchen) True)
(= (connected kitchen garden) True)

(= (get_neighbors $location)
   (match &locations $location $neighbors))

(= (find_path $from $to $visited)
   (if (== $from $to)
       (cons $from Nil)
       (let $neighbors (get_neighbors $from)
         (let $unvisited (filter (lambda (n) (not (contains $visited n))) $neighbors)
           (if (empty $unvisited)
               Nil
               (fold (lambda (neighbor acc)
                       (if (not (== acc Nil))
                           acc
                           (let $path (find_path neighbor $to (cons $from $visited))
                             (if (not (== $path Nil))
                                 (cons $from $path)
                                 Nil))))
                     Nil
                     $unvisited))))))

(= (navigate $from $to)
   (let $path (find_path $from $to Nil)
     (if (== $path Nil)
         (error "No path found")
         (move_along $path))))

(navigate room1 garden)
"#;

// Sample Rholang code with embedded MeTTa
const RHOLANG_SIMPLE: &str = "new metta in {\n\
  @\"#!metta\n\
  (= (test $x) (* $x 2))\n\
  (test 5)\n\
  \"!(metta)\n\
}\n";

const RHOLANG_COMPLEX: &str = "new metta, robotApi, result in {\n\
  contract robot(@location, return) = {\n\
    @\"#!metta\n\
    ;; Navigation logic\n\
    (= (connected room1 hallway) True)\n\
    (= (connected hallway room2) True)\n\
\n\
    (= (get_neighbors $loc)\n\
       (if (connected $loc $next) $next Empty))\n\
\n\
    (= (navigate $from $to)\n\
       (let $neighbors (get_neighbors $from)\n\
         (if (contains $neighbors $to)\n\
             (move $to)\n\
             (find_path $from $to))))\n\
    \"!(metta)\n\
  } |\n\
\n\
  robot!(\"room1\", *result) |\n\
\n\
  for (@nav <- result) {\n\
    @\"Navigated: \"!(nav)\n\
  }\n\
}\n";

fn bench_metta_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("metta_parsing");

    group.bench_function("simple", |b| {
        b.iter(|| {
            let mut parser = metta_parser::MettaParser::new().unwrap();
            black_box(parser.parse_to_ir(METTA_SIMPLE))
        })
    });

    group.bench_function("complex", |b| {
        b.iter(|| {
            let mut parser = metta_parser::MettaParser::new().unwrap();
            black_box(parser.parse_to_ir(METTA_COMPLEX))
        })
    });

    group.finish();
}

fn bench_symbol_table_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbol_table_building");

    // Pre-parse for benchmarking
    let mut parser = metta_parser::MettaParser::new().unwrap();
    let simple_ir = parser.parse_to_ir(METTA_SIMPLE).unwrap();
    let complex_ir = parser.parse_to_ir(METTA_COMPLEX).unwrap();

    let uri = Url::parse("file:///test.metta").unwrap();

    group.bench_function("simple", |b| {
        b.iter(|| {
            let builder = MettaSymbolTableBuilder::new(uri.clone());
            black_box(builder.build(&simple_ir))
        })
    });

    group.bench_function("complex", |b| {
        b.iter(|| {
            let builder = MettaSymbolTableBuilder::new(uri.clone());
            black_box(builder.build(&complex_ir))
        })
    });

    group.finish();
}

fn bench_symbol_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbol_resolution");

    // Setup
    let mut parser = metta_parser::MettaParser::new().unwrap();
    let ir = parser.parse_to_ir(METTA_COMPLEX).unwrap();
    let uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new(uri.clone());
    let symbol_table = Arc::new(builder.build(&ir));

    // Create resolver
    let base_resolver = Box::new(LexicalScopeResolver::new(
        symbol_table.clone(),
        "metta".to_string(),
    ));

    let filters = vec![];
    let resolver = ComposableSymbolResolver::new(base_resolver, filters, None);

    // Context for resolution
    let context = ResolutionContext {
        uri: uri.clone(),
        scope_id: Some(0),
        ir_node: None,
        language: "metta".to_string(),
        parent_uri: None,
    };

    let position = rholang_language_server::ir::semantic_node::Position {
        row: 10,
        column: 15,
        byte: 100,
    };

    group.bench_function("resolve_navigate", |b| {
        b.iter(|| {
            black_box(resolver.resolve_symbol("navigate", &position, &context))
        })
    });

    group.bench_function("resolve_get_neighbors", |b| {
        b.iter(|| {
            black_box(resolver.resolve_symbol("get_neighbors", &position, &context))
        })
    });

    group.finish();
}

fn bench_virtual_document_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtual_document_detection");

    let registry = Arc::new(DetectorRegistry::with_defaults());

    group.bench_function("simple_rholang", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_SIMPLE);
            let rope = Rope::from_str(RHOLANG_SIMPLE);
            black_box(registry.detect_all(RHOLANG_SIMPLE, &tree, &rope))
        })
    });

    group.bench_function("complex_rholang", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_COMPLEX);
            let rope = Rope::from_str(RHOLANG_COMPLEX);
            black_box(registry.detect_all(RHOLANG_COMPLEX, &tree, &rope))
        })
    });

    group.finish();
}

fn bench_end_to_end_virtual_doc(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end_virtual_doc");

    let registry = Arc::new(DetectorRegistry::with_defaults());
    let tree = tree_sitter::parse_code(RHOLANG_COMPLEX);
    let rope = Rope::from_str(RHOLANG_COMPLEX);
    let regions = registry.detect_all(RHOLANG_COMPLEX, &tree, &rope);

    group.bench_function("parse_and_build_symbols", |b| {
        b.iter(|| {
            for region in &regions {
                if region.language == "metta" {
                    // Parse
                    let mut parser = metta_parser::MettaParser::new().unwrap();
                    let ir = parser.parse_to_ir(&region.content).unwrap();

                    // Build symbol table
                    let uri = Url::parse("file:///test.rho#vdoc:0").unwrap();
                    let builder = MettaSymbolTableBuilder::new(uri);
                    let table = builder.build(&ir);

                    black_box(table);
                }
            }
        })
    });

    group.finish();
}

fn bench_parallel_processing(c: &mut Criterion) {
    use rayon::prelude::*;

    let mut group = c.benchmark_group("parallel_processing");

    // Generate multiple documents
    let documents: Vec<String> = (0..10)
        .map(|i| {
            format!(
                "new metta{} in {{\n  @\"#!metta\n  (= (func{} $x) (* $x {}))\n  (func{} 10)\n  \"!(metta{})\n}}\n",
                i, i, i + 1, i, i
            )
        })
        .collect();

    group.bench_function("sequential", |b| {
        b.iter(|| {
            let registry = Arc::new(DetectorRegistry::with_defaults());
            for doc in &documents {
                let tree = tree_sitter::parse_code(doc);
                let rope = Rope::from_str(doc);
                black_box(registry.detect_all(doc, &tree, &rope));
            }
        })
    });

    group.bench_function("rayon_parallel", |b| {
        b.iter(|| {
            let registry = Arc::new(DetectorRegistry::with_defaults());
            documents.par_iter().for_each(|doc| {
                let tree = tree_sitter::parse_code(doc);
                let rope = Rope::from_str(doc);
                black_box(registry.detect_all(doc, &tree, &rope));
            });
        })
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(10));
    targets =
        bench_metta_parsing,
        bench_symbol_table_building,
        bench_symbol_resolution,
        bench_virtual_document_detection,
        bench_end_to_end_virtual_doc,
        bench_parallel_processing
}

criterion_main!(benches);
