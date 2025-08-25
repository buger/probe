#!/usr/bin/env rust-script

//! Test script to verify Windows CI stack overflow fix
//! 
//! This script simulates the Windows CI environment and tests that
//! the PARSER_WARMER static initialization doesn't trigger stack overflow.

use std::process::Command;
use std::env;

fn main() {
    println!("=== Testing Windows CI Stack Overflow Fix ===\n");

    // Test 1: Normal environment (parser warming should work)
    println("1. Testing normal environment (parser warming enabled):");
    env::remove_var("CI");
    env::remove_var("GITHUB_ACTIONS");
    env::remove_var("PROBE_NO_PARSER_WARMUP");
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to run probe in normal environment");
    
    if output.status.success() {
        println!("   ✓ Normal environment works correctly");
    } else {
        println!("   ✗ Normal environment failed");
        println!("   stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    // Test 2: Windows CI environment (parser warming should be disabled)
    println!("\n2. Testing Windows CI environment (parser warming disabled):");
    env::set_var("CI", "true");
    env::set_var("GITHUB_ACTIONS", "true");
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to run probe in CI environment");
    
    if output.status.success() {
        println!("   ✓ Windows CI environment works correctly");
        println!("   ✓ Static initialization completed without stack overflow");
    } else {
        println!("   ✗ Windows CI environment failed");
        println!("   stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    // Test 3: Explicit parser warmup disabled
    println!("\n3. Testing explicit parser warmup disabled:");
    env::set_var("PROBE_NO_PARSER_WARMUP", "1");
    
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to run probe with warmup disabled");
    
    if output.status.success() {
        println!("   ✓ Explicit warmup disable works correctly");
    } else {
        println!("   ✗ Explicit warmup disable failed");
        println!("   stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    println!("\n=== Test Summary ===");
    println!("The fix should prevent stack overflow during static initialization");
    println!("by disabling parser warming on Windows CI environments while");
    println!("preserving normal functionality on other platforms.");
}