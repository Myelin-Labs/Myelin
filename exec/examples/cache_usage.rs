// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Cache Usage Example
//
// This example demonstrates the serialization cache for optimizing
// repeated serialization operations.

use spora_exec::serialization::utils;
use spora_exec::{CellOutput, Script, SerializationCache, ThreadSafeSerializationCache};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Serialization Cache Usage Example ===\n");

    // ============================================================================
    // Basic Cache Usage
    // ============================================================================
    println!("--- Basic Cache Usage ---");

    let mut cache = SerializationCache::new(100); // Cache up to 100 items

    let output = CellOutput {
        lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]),
        type_: Some(Script::new([0xCC; 32], 1, vec![0xDD; 10])),
        capacity: 1000,
    };

    // First access - serializes and caches
    let start = std::time::Instant::now();
    let bytes1 = cache.get_or_serialize(&output)?;
    let first_time = start.elapsed();
    println!("First access (with serialization): {:?}", first_time);
    println!("Cache size: {}", cache.len());

    // Second access - returns cached result
    let start = std::time::Instant::now();
    let bytes2 = cache.get_or_serialize(&output)?;
    let second_time = start.elapsed();
    println!("Second access (cached): {:?}", second_time);
    println!("Cache size: {} (no change)", cache.len());

    // Verify same memory
    assert_eq!(bytes1.as_ptr(), bytes2.as_ptr());
    println!("✓ Same memory location (cached)");

    // ============================================================================
    // Cache Statistics
    // ============================================================================
    println!("\n--- Cache Statistics ---");

    let stats = cache.stats();
    println!("Cache size: {}/{}", stats.size, stats.max_size);
    println!("Utilization: {:.1}%", stats.utilization_percent());

    // ============================================================================
    // Multiple Items
    // ============================================================================
    println!("\n--- Multiple Items ---");

    let outputs: Vec<CellOutput> = (0..10)
        .map(|i| CellOutput { lock: Script::new([i as u8; 32], 0, vec![i as u8; 20]), type_: None, capacity: i as u64 })
        .collect();

    // First pass - populate cache
    let start = std::time::Instant::now();
    for output in &outputs {
        cache.get_or_serialize(output)?;
    }
    let populate_time = start.elapsed();
    println!("Populated cache with {} items in {:?}", outputs.len(), populate_time);

    // Second pass - all cached
    let start = std::time::Instant::now();
    for output in &outputs {
        cache.get_or_serialize(output)?;
    }
    let cached_time = start.elapsed();
    println!("Retrieved {} items from cache in {:?}", outputs.len(), cached_time);

    let speedup = populate_time.as_nanos() as f64 / cached_time.as_nanos() as f64;
    println!("Speedup: {:.1}x", speedup);

    // ============================================================================
    // Cache Eviction (LRU)
    // ============================================================================
    println!("\n--- Cache Eviction (LRU) ---");

    let small_cache = SerializationCache::new(3); // Very small cache
    let mut small_cache = small_cache; // Make it mutable

    let items: Vec<CellOutput> = (0..5)
        .map(|i| CellOutput { lock: Script::new([i as u8; 32], 0, vec![i as u8; 20]), type_: None, capacity: i as u64 })
        .collect();

    // Add 3 items
    for i in 0..3 {
        small_cache.get_or_serialize(&items[i])?;
        println!("Added item {}, cache size: {}", i, small_cache.len());
    }

    // Access item 0 to make it recently used
    small_cache.get_or_serialize(&items[0])?;
    println!("Accessed item 0 (now most recent)");

    // Add 2 more items - should evict item 1
    for i in 3..5 {
        small_cache.get_or_serialize(&items[i])?;
        println!("Added item {}, cache size: {}", i, small_cache.len());
    }

    println!("Item 0 in cache: {}", small_cache.contains(&items[0])); // Should be true
    println!("Item 1 in cache: {}", small_cache.contains(&items[1])); // Should be false (evicted)
    println!("Item 2 in cache: {}", small_cache.contains(&items[2])); // Should be true

    // ============================================================================
    // Thread-Safe Cache
    // ============================================================================
    println!("\n--- Thread-Safe Cache ---");

    let thread_cache = ThreadSafeSerializationCache::new(100);
    let output = CellOutput { lock: Script::new([0xFF; 32], 0, vec![0xEE; 20]), type_: None, capacity: 9999 };

    // Simulate concurrent access
    let bytes1 = thread_cache.get_or_serialize(&output)?;
    let bytes2 = thread_cache.get_or_serialize(&output)?;

    assert_eq!(bytes1.as_ptr(), bytes2.as_ptr());
    println!("✓ Thread-safe cache returns same memory");

    let stats = thread_cache.stats();
    println!("Thread-safe cache size: {}/{}", stats.size, stats.max_size);

    // ============================================================================
    // Performance Comparison
    // ============================================================================
    println!("\n--- Performance Comparison ---");

    let large_outputs: Vec<CellOutput> = (0..1000)
        .map(|i| CellOutput {
            lock: Script::new([i as u8; 32], 0, vec![0xBB; 20]),
            type_: if i % 2 == 0 { Some(Script::new([0x11; 32], 1, vec![0x22; 10])) } else { None },
            capacity: i as u64,
        })
        .collect();

    // Without cache
    let start = std::time::Instant::now();
    for _ in 0..10 {
        for output in &large_outputs {
            let _ = utils::serialize_to_bytes(output).unwrap();
        }
    }
    let without_cache = start.elapsed();
    println!("Without cache (10 iterations): {:?}", without_cache);

    // With cache
    let mut cache = SerializationCache::new(1000);
    let start = std::time::Instant::now();
    for _ in 0..10 {
        for output in &large_outputs {
            let _ = cache.get_or_serialize(output).unwrap();
        }
    }
    let with_cache = start.elapsed();
    println!("With cache (10 iterations): {:?}", with_cache);

    let speedup = without_cache.as_nanos() as f64 / with_cache.as_nanos() as f64;
    println!("Overall speedup: {:.1}x", speedup);

    // ============================================================================
    // Summary
    // ============================================================================
    println!("\n=== Summary ===");
    println!("✓ SerializationCache - Single-threaded cache with LRU eviction");
    println!("✓ ThreadSafeSerializationCache - Thread-safe version with RwLock");
    println!("✓ CacheStats - Monitor cache utilization");
    println!("✓ get_or_serialize() - One-liner for cached serialization");
    println!("✓ contains() - Check if value is cached");
    println!("✓ clear() - Clear cache when needed");
    println!("\nBest practices:");
    println!("  - Use cache for frequently accessed data");
    println!("  - Set appropriate max_size based on memory constraints");
    println!("  - Monitor utilization to tune cache size");
    println!("  - Clear cache after large operations to free memory");

    Ok(())
}
