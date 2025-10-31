//! Benchmark comparing spawn_blocking vs rayon for virtual document detection

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use rholang_language_server::language_regions::{DetectorRegistry, spawn_detection_worker};
use url::Url;

// Sample Rholang code with varying complexity
const SIMPLE_CODE: &str = r#"
@"rho:metta:compile"!("(= test 123)")
"#;

const MEDIUM_CODE: &str = r#"
// @metta
@"rho:metta:compile"!("(= factorial (lambda (n) (if (< n 2) 1 (* n (factorial (- n 1))))))")

new metta in {
  for (metta <- @"rho:metta:compile") {
    metta!("(= fibonacci (lambda (n) 42))")
  }
}

@"rho:metta:eval"!("(+ 1 2 3)")
"#;

const COMPLEX_CODE: &str = r#"
// @metta
@"rho:metta:compile"!("(= factorial (lambda (n) (if (< n 2) 1 (* n (factorial (- n 1))))))")

new mettaCompile(`rho:metta:compile`), result in {
  for (metta <- mettaCompile) {
    metta!("(= test1 123)") |
    metta!("(= test2 456)") |
    metta!("(= test3 789)")
  } |

  @"rho:metta:eval"!("!(get_neighbors " ++ room ++ ")") |

  contract processQuery(@query) = {
    // @metta
    @"rho:metta:compile"!("!(process " ++ query ++ ")")
  }
}

// Multiple directive-based regions
// @metta
@"rho:metta:compile"!("(= helper1 (lambda (x) (+ x 1)))")

// @metta
@"rho:metta:compile"!("(= helper2 (lambda (x) (+ x 2)))")

// @metta
@"rho:metta:compile"!("(= helper3 (lambda (x) (+ x 3)))")
"#;

/// Benchmark using spawn_blocking (current implementation)
async fn benchmark_spawn_blocking(
    source: &str,
    iterations: usize,
) -> Duration {
    let registry = Arc::new(DetectorRegistry::with_defaults());
    let worker = spawn_detection_worker(registry);

    let start = Instant::now();

    let mut receivers = Vec::new();
    for i in 0..iterations {
        let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
        let rx = worker.detect(uri, source.to_string());
        receivers.push(rx);
    }

    // Wait for all results
    for rx in receivers {
        rx.await.expect("Should receive result");
    }

    start.elapsed()
}

/// Benchmark using rayon for parallel detection
async fn benchmark_rayon(
    source: &str,
    iterations: usize,
) -> Duration {
    use rayon::prelude::*;

    let registry = Arc::new(DetectorRegistry::with_defaults());
    let start = Instant::now();

    // Process all iterations in parallel using rayon
    let _results: Vec<_> = (0..iterations)
        .into_par_iter()
        .map(|i| {
            let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
            let tree = rholang_language_server::tree_sitter::parse_code(source);
            let rope = ropey::Rope::from_str(source);
            let regions = registry.detect_all(source, &tree, &rope);
            (uri, regions)
        })
        .collect();

    start.elapsed()
}

/// Hybrid approach: spawn_blocking + rayon inside
async fn benchmark_hybrid(
    source: &str,
    iterations: usize,
) -> Duration {
    let registry = Arc::new(DetectorRegistry::with_defaults());
    let start = Instant::now();

    let registry_clone = registry.clone();
    let source = source.to_string();

    let _result = tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;

        (0..iterations)
            .into_par_iter()
            .map(|i| {
                let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
                let tree = rholang_language_server::tree_sitter::parse_code(&source);
                let rope = ropey::Rope::from_str(&source);
                let regions = registry_clone.detect_all(&source, &tree, &rope);
                (uri, regions)
            })
            .collect::<Vec<_>>()
    })
    .await
    .expect("Task should complete");

    start.elapsed()
}

fn run_benchmarks() {
    println!("\n═══════════════════════════════════════════════════════");
    println!("  Virtual Document Detection Worker Benchmark");
    println!("═══════════════════════════════════════════════════════\n");

    let rt = Runtime::new().unwrap();

    // Test scenarios
    let scenarios = vec![
        ("Simple code (1 region)", SIMPLE_CODE, 100),
        ("Medium code (3-4 regions)", MEDIUM_CODE, 50),
        ("Complex code (7+ regions)", COMPLEX_CODE, 20),
        ("Burst load (simple)", SIMPLE_CODE, 500),
    ];

    for (name, code, iterations) in scenarios {
        println!("Scenario: {} ({} iterations)", name, iterations);
        println!("───────────────────────────────────────────────────────");

        // Warm up
        rt.block_on(benchmark_spawn_blocking(code, 5));
        rt.block_on(benchmark_rayon(code, 5));
        rt.block_on(benchmark_hybrid(code, 5));

        // Run benchmarks
        let spawn_blocking_time = rt.block_on(benchmark_spawn_blocking(code, iterations));
        let rayon_time = rt.block_on(benchmark_rayon(code, iterations));
        let hybrid_time = rt.block_on(benchmark_hybrid(code, iterations));

        // Calculate throughput
        let spawn_blocking_throughput = iterations as f64 / spawn_blocking_time.as_secs_f64();
        let rayon_throughput = iterations as f64 / rayon_time.as_secs_f64();
        let hybrid_throughput = iterations as f64 / hybrid_time.as_secs_f64();

        println!("  spawn_blocking: {:>8.2}ms ({:>7.1} req/s)",
                 spawn_blocking_time.as_millis(), spawn_blocking_throughput);
        println!("  rayon:          {:>8.2}ms ({:>7.1} req/s)",
                 rayon_time.as_millis(), rayon_throughput);
        println!("  hybrid:         {:>8.2}ms ({:>7.1} req/s)",
                 hybrid_time.as_millis(), hybrid_throughput);

        // Determine winner
        let fastest = spawn_blocking_time.min(rayon_time).min(hybrid_time);
        let winner = if fastest == spawn_blocking_time {
            "spawn_blocking"
        } else if fastest == rayon_time {
            "rayon"
        } else {
            "hybrid"
        };

        let speedup = if fastest == spawn_blocking_time {
            1.0
        } else if fastest == rayon_time {
            spawn_blocking_time.as_secs_f64() / rayon_time.as_secs_f64()
        } else {
            spawn_blocking_time.as_secs_f64() / hybrid_time.as_secs_f64()
        };

        println!("  ✓ Winner: {} ({:.2}x faster)\n", winner, speedup);
    }

    println!("═══════════════════════════════════════════════════════");
    println!("\nRecommendation based on benchmarks:");
    println!("  • spawn_blocking: Best for async/await integration");
    println!("  • rayon: Best for pure CPU-bound parallel workloads");
    println!("  • hybrid: Best balance for LSP server use case");
    println!("═══════════════════════════════════════════════════════\n");
}

fn main() {
    run_benchmarks();
}
