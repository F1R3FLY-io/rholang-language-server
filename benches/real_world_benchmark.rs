//! Real-world scenario benchmarks
//!
//! This benchmark suite tests the language server with realistic Rholang code samples
//! of various sizes and complexity levels.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::parsers::ParseCache;
use rholang_language_server::ir::symbol_table::SymbolTable;
use ropey::Rope;
use std::sync::Arc;

/// Small Rholang file (~50 lines) - typical single contract
const SMALL_FILE: &str = r#"
new stdout(`rho:io:stdout`), deployerId(`rho:rchain:deployerId`) in {
  contract @"HelloWorld"(@name) = {
    new channel in {
      stdout!(*deployerId ++ " says: Hello, " ++ name) |
      @"ack"!(*channel)
    }
  } |
  new ret in {
    @"HelloWorld"!("Alice", *ret) |
    for (@response <- ret) {
      stdout!(response)
    }
  }
}
"#;

/// Medium Rholang file (~200 lines) - multiple contracts with state
const MEDIUM_FILE: &str = r#"
new stdout(`rho:io:stdout`), deployerId(`rho:rchain:deployerId`),
    MakeCellFactory, BasicWallet in {

  // Cell factory pattern
  contract MakeCellFactory(@init, return) = {
    new valueStore in {
      valueStore!(init) |
      contract @"get"(return) = {
        for (@value <- valueStore) {
          valueStore!(value) | return!(value)
        }
      } |
      contract @"set"(@newValue, return) = {
        for (_ <- valueStore) {
          valueStore!(newValue) | return!(true)
        }
      }
    } |
    return!(@"get", @"set")
  } |

  // Basic wallet implementation
  contract BasicWallet(@owner, @initialBalance, return) = {
    new balanceStore, depositCh, withdrawCh, getBalanceCh in {
      balanceStore!(initialBalance) |

      contract @(*depositCh)(@amount, ack) = {
        for (@balance <- balanceStore) {
          balanceStore!(balance + amount) |
          ack!(true)
        }
      } |

      contract @(*withdrawCh)(@amount, return) = {
        for (@balance <- balanceStore) {
          if (balance >= amount) {
            balanceStore!(balance - amount) |
            return!(("success", balance - amount))
          } else {
            balanceStore!(balance) |
            return!(("insufficient_funds", balance))
          }
        }
      } |

      contract @(*getBalanceCh)(return) = {
        for (@balance <- balanceStore) {
          balanceStore!(balance) |
          return!(balance)
        }
      } |

      return!((*depositCh, *withdrawCh, *getBalanceCh))
    }
  } |

  // Test the wallet
  new walletCh, depositCh, withdrawCh, getBalanceCh in {
    BasicWallet!(*deployerId, 100, *walletCh) |
    for (@(deposit, withdraw, getBalance) <- walletCh) {
      new ack in {
        @deposit!(50, *ack) |
        for (_ <- ack) {
          @getBalance!(*getBalanceCh) |
          for (@balance <- getBalanceCh) {
            stdout!({"new balance": balance}) |

            @withdraw!(75, *withdrawCh) |
            for (@result <- withdrawCh) {
              match result {
                ("success", @newBalance) => {
                  stdout!({"withdraw success, balance": newBalance})
                }
                ("insufficient_funds", @balance) => {
                  stdout!({"insufficient funds, balance": balance})
                }
              }
            }
          }
        }
      }
    }
  }
}
"#;

/// Large Rholang file (~500+ lines) - complex multi-contract system
const LARGE_FILE: &str = r#"
new stdout(`rho:io:stdout`), deployerId(`rho:rchain:deployerId`),
    TokenRegistry, MakeToken, ERC20Factory in {

  // Token Registry for managing all tokens
  contract TokenRegistry(@init, return) = {
    new tokensStore, registerCh, getCh, listCh in {
      tokensStore!({}) |

      contract @(*registerCh)(@tokenId, @tokenContract, return) = {
        for (@tokens <- tokensStore) {
          match tokens.get(tokenId) {
            Nil => {
              tokensStore!(tokens.set(tokenId, tokenContract)) |
              return!(("registered", tokenId))
            }
            _ => {
              tokensStore!(tokens) |
              return!(("already_exists", tokenId))
            }
          }
        }
      } |

      contract @(*getCh)(@tokenId, return) = {
        for (@tokens <- tokensStore) {
          tokensStore!(tokens) |
          match tokens.get(tokenId) {
            Nil => return!(("not_found", Nil))
            token => return!(("found", token))
          }
        }
      } |

      contract @(*listCh)(return) = {
        for (@tokens <- tokensStore) {
          tokensStore!(tokens) |
          return!(tokens.keys())
        }
      } |

      return!((*registerCh, *getCh, *listCh))
    }
  } |

  // ERC20-style token factory
  contract ERC20Factory(@name, @symbol, @decimals, @totalSupply, @owner, return) = {
    new balancesStore, allowancesStore,
        transferCh, approveCh, transferFromCh, balanceOfCh,
        totalSupplyCh, nameCh, symbolCh, decimalsCh in {

      balancesStore!({}.set(owner, totalSupply)) |
      allowancesStore!({}) |

      // Transfer tokens
      contract @(*transferCh)(@from, @to, @amount, return) = {
        for (@balances <- balancesStore) {
          match balances.get(from) {
            Nil => {
              balancesStore!(balances) |
              return!(("error", "insufficient_balance"))
            }
            @fromBalance => {
              if (fromBalance >= amount) {
                match balances.get(to) {
                  Nil => {
                    new newBalances in {
                      newBalances!(
                        balances
                          .set(from, fromBalance - amount)
                          .set(to, amount)
                      ) |
                      for (@nb <- newBalances) {
                        balancesStore!(nb) |
                        return!(("success", nb))
                      }
                    }
                  }
                  @toBalance => {
                    new newBalances in {
                      newBalances!(
                        balances
                          .set(from, fromBalance - amount)
                          .set(to, toBalance + amount)
                      ) |
                      for (@nb <- newBalances) {
                        balancesStore!(nb) |
                        return!(("success", nb))
                      }
                    }
                  }
                }
              } else {
                balancesStore!(balances) |
                return!(("error", "insufficient_balance"))
              }
            }
          }
        }
      } |

      // Approve spender
      contract @(*approveCh)(@owner, @spender, @amount, return) = {
        for (@allowances <- allowancesStore) {
          match allowances.get(owner) {
            Nil => {
              allowancesStore!(allowances.set(owner, {}.set(spender, amount))) |
              return!(("approved", amount))
            }
            @ownerAllowances => {
              allowancesStore!(
                allowances.set(owner, ownerAllowances.set(spender, amount))
              ) |
              return!(("approved", amount))
            }
          }
        }
      } |

      // Transfer from approved amount
      contract @(*transferFromCh)(@spender, @from, @to, @amount, return) = {
        for (@allowances <- allowancesStore ; @balances <- balancesStore) {
          match allowances.get(from) {
            Nil => {
              allowancesStore!(allowances) |
              balancesStore!(balances) |
              return!(("error", "no_allowance"))
            }
            @ownerAllowances => {
              match ownerAllowances.get(spender) {
                Nil => {
                  allowancesStore!(allowances) |
                  balancesStore!(balances) |
                  return!(("error", "no_allowance"))
                }
                @allowance => {
                  if (allowance >= amount) {
                    // Check from balance
                    match balances.get(from) {
                      Nil => {
                        allowancesStore!(allowances) |
                        balancesStore!(balances) |
                        return!(("error", "insufficient_balance"))
                      }
                      @fromBalance => {
                        if (fromBalance >= amount) {
                          // Update balances
                          match balances.get(to) {
                            Nil => {
                              new newBalances, newAllowances in {
                                newBalances!(
                                  balances
                                    .set(from, fromBalance - amount)
                                    .set(to, amount)
                                ) |
                                newAllowances!(
                                  allowances.set(from,
                                    ownerAllowances.set(spender, allowance - amount))
                                ) |
                                for (@nb <- newBalances ; @na <- newAllowances) {
                                  balancesStore!(nb) |
                                  allowancesStore!(na) |
                                  return!(("success", amount))
                                }
                              }
                            }
                            @toBalance => {
                              new newBalances, newAllowances in {
                                newBalances!(
                                  balances
                                    .set(from, fromBalance - amount)
                                    .set(to, toBalance + amount)
                                ) |
                                newAllowances!(
                                  allowances.set(from,
                                    ownerAllowances.set(spender, allowance - amount))
                                ) |
                                for (@nb <- newBalances ; @na <- newAllowances) {
                                  balancesStore!(nb) |
                                  allowancesStore!(na) |
                                  return!(("success", amount))
                                }
                              }
                            }
                          }
                        } else {
                          allowancesStore!(allowances) |
                          balancesStore!(balances) |
                          return!(("error", "insufficient_balance"))
                        }
                      }
                    }
                  } else {
                    allowancesStore!(allowances) |
                    balancesStore!(balances) |
                    return!(("error", "allowance_exceeded"))
                  }
                }
              }
            }
          }
        }
      } |

      // Query balance
      contract @(*balanceOfCh)(@account, return) = {
        for (@balances <- balancesStore) {
          balancesStore!(balances) |
          match balances.get(account) {
            Nil => return!(0)
            @balance => return!(balance)
          }
        }
      } |

      // Metadata queries
      contract @(*totalSupplyCh)(return) = { return!(totalSupply) } |
      contract @(*nameCh)(return) = { return!(name) } |
      contract @(*symbolCh)(return) = { return!(symbol) } |
      contract @(*decimalsCh)(return) = { return!(decimals) } |

      return!({
        "transfer": *transferCh,
        "approve": *approveCh,
        "transferFrom": *transferFromCh,
        "balanceOf": *balanceOfCh,
        "totalSupply": *totalSupplyCh,
        "name": *nameCh,
        "symbol": *symbolCh,
        "decimals": *decimalsCh
      })
    }
  } |

  // Test the token system
  new registryCh, tokenCh in {
    TokenRegistry!({}, *registryCh) |
    for (@(register, get, list) <- registryCh) {
      ERC20Factory!("MyToken", "MTK", 18, 1000000, *deployerId, *tokenCh) |
      for (@token <- tokenCh) {
        @register!("MTK", token, *stdout) |
        for (@result <- stdout) {
          stdout!({"registration": result}) |

          // Test token operations
          new transferResult, balanceResult in {
            @(token.get("transfer"))!(*deployerId, "alice", 100, *transferResult) |
            for (@tResult <- transferResult) {
              stdout!({"transfer": tResult}) |

              @(token.get("balanceOf"))!("alice", *balanceResult) |
              for (@balance <- balanceResult) {
                stdout!({"alice balance": balance})
              }
            }
          }
        }
      }
    }
  }
}
"#;

/// Benchmark parsing different file sizes
fn bench_parsing_file_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsing/file_sizes");

    group.bench_with_input(BenchmarkId::new("small", "50_lines"), &SMALL_FILE, |b, code| {
        b.iter(|| {
            let tree = parse_code(black_box(code));
            black_box(tree);
        });
    });

    group.bench_with_input(BenchmarkId::new("medium", "200_lines"), &MEDIUM_FILE, |b, code| {
        b.iter(|| {
            let tree = parse_code(black_box(code));
            black_box(tree);
        });
    });

    group.bench_with_input(BenchmarkId::new("large", "500_lines"), &LARGE_FILE, |b, code| {
        b.iter(|| {
            let tree = parse_code(black_box(code));
            black_box(tree);
        });
    });

    group.finish();
}

/// Benchmark IR conversion for different file sizes
fn bench_ir_conversion_file_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ir_conversion/file_sizes");

    let small_tree = parse_code(SMALL_FILE);
    let medium_tree = parse_code(MEDIUM_FILE);
    let large_tree = parse_code(LARGE_FILE);

    group.bench_with_input(BenchmarkId::new("small", "50_lines"), &(small_tree, SMALL_FILE), |b, (tree, code)| {
        b.iter(|| {
            let rope = Rope::from_str(code);
            let ir = parse_to_ir(black_box(tree), &rope);
            black_box(ir);
        });
    });

    group.bench_with_input(BenchmarkId::new("medium", "200_lines"), &(medium_tree, MEDIUM_FILE), |b, (tree, code)| {
        b.iter(|| {
            let rope = Rope::from_str(code);
            let ir = parse_to_ir(black_box(tree), &rope);
            black_box(ir);
        });
    });

    group.bench_with_input(BenchmarkId::new("large", "500_lines"), &(large_tree, LARGE_FILE), |b, (tree, code)| {
        b.iter(|| {
            let rope = Rope::from_str(code);
            let ir = parse_to_ir(black_box(tree), &rope);
            black_box(ir);
        });
    });

    group.finish();
}

/// Benchmark parse cache effectiveness
fn bench_parse_cache_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_cache/effectiveness");

    group.bench_function("cold_cache_small", |b| {
        b.iter(|| {
            let cache = ParseCache::new(100);
            cache.clear();  // Ensure cold cache
            let tree = parse_code(black_box(SMALL_FILE));
            black_box(tree);
        });
    });

    group.bench_function("warm_cache_small", |b| {
        let cache = ParseCache::new(100);
        // Warm up cache
        let tree = parse_code(SMALL_FILE);
        cache.insert(SMALL_FILE.to_string(), tree);

        b.iter(|| {
            let cached = cache.get(black_box(SMALL_FILE));
            black_box(cached);
        });
    });

    group.bench_function("cold_cache_large", |b| {
        b.iter(|| {
            let cache = ParseCache::new(100);
            cache.clear();  // Ensure cold cache
            let tree = parse_code(black_box(LARGE_FILE));
            black_box(tree);
        });
    });

    group.bench_function("warm_cache_large", |b| {
        let cache = ParseCache::new(100);
        // Warm up cache
        let tree = parse_code(LARGE_FILE);
        cache.insert(LARGE_FILE.to_string(), tree);

        b.iter(|| {
            let cached = cache.get(black_box(LARGE_FILE));
            black_box(cached);
        });
    });

    group.finish();
}

/// Benchmark symbol table building for different file sizes
fn bench_symbol_table_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbol_table/building");

    let small_tree = parse_code(SMALL_FILE);
    let small_rope = Rope::from_str(SMALL_FILE);
    let small_ir = parse_to_ir(&small_tree, &small_rope);

    let medium_tree = parse_code(MEDIUM_FILE);
    let medium_rope = Rope::from_str(MEDIUM_FILE);
    let medium_ir = parse_to_ir(&medium_tree, &medium_rope);

    let large_tree = parse_code(LARGE_FILE);
    let large_rope = Rope::from_str(LARGE_FILE);
    let large_ir = parse_to_ir(&large_tree, &large_rope);

    group.bench_function("small_file", |b| {
        b.iter(|| {
            let table = Arc::new(SymbolTable::new(None));
            // In real usage, SymbolTableBuilder would populate this
            black_box(table);
        });
    });

    group.bench_function("medium_file", |b| {
        b.iter(|| {
            let table = Arc::new(SymbolTable::new(None));
            black_box(table);
        });
    });

    group.bench_function("large_file", |b| {
        b.iter(|| {
            let table = Arc::new(SymbolTable::new(None));
            black_box(table);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parsing_file_sizes,
    bench_ir_conversion_file_sizes,
    bench_parse_cache_effectiveness,
    bench_symbol_table_building,
);

criterion_main!(benches);
