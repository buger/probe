# Makefile for probe Rust project

# Configuration
VERSION ?= v0.1.0
CARGO := cargo
RUSTC := rustc

# Detect operating system
ifeq ($(OS),Windows_NT)
    # Windows - use cmd.exe to set environment variables
    SET_ENV := cmd.exe /c "set RUST_BACKTRACE=1 &&"
else
    # Unix-like systems (Linux, macOS)
    SET_ENV := RUST_BACKTRACE=1
endif
RUSTFMT := rustfmt
CLIPPY := cargo clippy
SCRIPTS_DIR := scripts
TESTS_DIR := tests
RELEASE_DIR := release/$(VERSION)
BINARY_NAME := probe

# Platform-specific settings
LINUX_TARGET := x86_64-unknown-linux-gnu
MACOS_X86_TARGET := x86_64-apple-darwin
MACOS_ARM_TARGET := aarch64-apple-darwin
WINDOWS_TARGET := x86_64-pc-windows-msvc

# Default target
.PHONY: all
all: build

# Version target
.PHONY: version
version:
	@echo "Probe version: $(VERSION)"

# Build targets
.PHONY: build
build:
	$(CARGO) build

.PHONY: install
install:
	$(CARGO) install --path .

.PHONY: release
release: clean-release version linux macos windows
	@echo "Release $(VERSION) created in $(RELEASE_DIR)"

.PHONY: clean-release
clean-release:
	rm -rf $(RELEASE_DIR)
	mkdir -p $(RELEASE_DIR)

.PHONY: linux
linux:
	@echo "Building for Linux ($(LINUX_TARGET))..."
	# Note: You may need to install the target with: rustup target add $(LINUX_TARGET)
	$(CARGO) build --release --target $(LINUX_TARGET)
	mkdir -p $(RELEASE_DIR)/linux
	cp target/$(LINUX_TARGET)/release/$(BINARY_NAME) $(RELEASE_DIR)/linux/$(BINARY_NAME)
	tar -czf $(RELEASE_DIR)/$(BINARY_NAME)-$(VERSION)-linux-x86_64.tar.gz -C $(RELEASE_DIR)/linux $(BINARY_NAME)

.PHONY: macos
macos: macos-x86 macos-arm

.PHONY: macos-x86
macos-x86:
	@echo "Building for macOS x86_64 ($(MACOS_X86_TARGET))..."
	# Note: You may need to install the target with: rustup target add $(MACOS_X86_TARGET)
	$(CARGO) build --release --target $(MACOS_X86_TARGET)
	mkdir -p $(RELEASE_DIR)/macos/x86_64
	cp target/$(MACOS_X86_TARGET)/release/$(BINARY_NAME) $(RELEASE_DIR)/macos/x86_64/$(BINARY_NAME)
	tar -czf $(RELEASE_DIR)/$(BINARY_NAME)-$(VERSION)-macos-x86_64.tar.gz -C $(RELEASE_DIR)/macos/x86_64 $(BINARY_NAME)

.PHONY: macos-arm
macos-arm:
	@echo "Building for macOS ARM ($(MACOS_ARM_TARGET))..."
	# Note: You may need to install the target with: rustup target add $(MACOS_ARM_TARGET)
	$(CARGO) build --release --target $(MACOS_ARM_TARGET)
	mkdir -p $(RELEASE_DIR)/macos/arm64
	cp target/$(MACOS_ARM_TARGET)/release/$(BINARY_NAME) $(RELEASE_DIR)/macos/arm64/$(BINARY_NAME)
	tar -czf $(RELEASE_DIR)/$(BINARY_NAME)-$(VERSION)-macos-arm64.tar.gz -C $(RELEASE_DIR)/macos/arm64 $(BINARY_NAME)

.PHONY: windows
windows:
	@echo "Building for Windows ($(WINDOWS_TARGET))..."
	# Note: You may need to install the target with: rustup target add $(WINDOWS_TARGET)
	# Note: You need to have the 'zip' utility installed on Windows (e.g., via chocolatey: choco install zip)
	$(CARGO) build --release --target $(WINDOWS_TARGET)
	mkdir -p $(RELEASE_DIR)/windows
	cp target/$(WINDOWS_TARGET)/release/$(BINARY_NAME).exe $(RELEASE_DIR)/windows/$(BINARY_NAME).exe
	zip -j $(RELEASE_DIR)/$(BINARY_NAME)-$(VERSION)-$(WINDOWS_TARGET).zip $(RELEASE_DIR)/windows/$(BINARY_NAME).exe

# Test targets
.PHONY: test
test: test-unit test-integration test-property test-cli

.PHONY: test-unit
test-unit:
	$(SET_ENV) $(CARGO) test --lib

.PHONY: test-integration
test-integration:
	$(SET_ENV) $(CARGO) test --test integration_tests

.PHONY: test-property
test-property:
	$(SET_ENV) $(CARGO) test --test property_tests

.PHONY: test-cli
test-cli:
	$(SET_ENV) $(CARGO) test --test cli_tests

.PHONY: test-all
test-all:
	$(SET_ENV) $(CARGO) test

# Code quality targets
.PHONY: lint
lint:
	$(CLIPPY) --all-targets --all-features -- -D warnings

.PHONY: format
format:
	$(CARGO) fmt --all

.PHONY: check-format
check-format:
	$(CARGO) fmt --all -- --check

.PHONY: fix-all
fix-all:
	@echo "Fixing all Rust issues..."
	$(CLIPPY) --all-targets --all-features --fix --allow-staged --allow-dirty -- -D warnings
	$(CARGO) fmt --all
	$(CARGO) check --tests
	@echo "All fixes applied. Run 'make test' to ensure everything still works."

# Documentation
.PHONY: doc
doc:
	$(CARGO) doc --no-deps

.PHONY: doc-open
doc-open:
	$(CARGO) doc --no-deps --open

# Clean targets
.PHONY: clean
clean:
	$(CARGO) clean

.PHONY: clean-all
clean-all: clean
	rm -rf Cargo.lock

# Run targets
.PHONY: run
run:
	$(CARGO) run

.PHONY: run-release
run-release:
	$(CARGO) run --release

# Git hooks targets
.PHONY: install-hooks
install-hooks:
	git config core.hooksPath .githooks
	@echo "Git hooks installed successfully! Pre-commit checks are now enabled."

# Help target
.PHONY: help
help:
	@echo "Available targets:"
	@echo "  all               - Build the project (default)"
	@echo "  build             - Build the project in debug mode"
	@echo "  install           - Install the binary on your system using cargo install"
	@echo "  version           - Print the current version"
	@echo "  release           - Build release packages for all platforms (VERSION=v0.1.0)"
	@echo "  linux             - Build release package for Linux"
	@echo "  macos             - Build release packages for macOS (x86_64 and arm64)"
	@echo "  macos-x86         - Build release package for macOS (x86_64)"
	@echo "  macos-arm         - Build release package for macOS (arm64)"
	@echo "  windows           - Build release package for Windows"
	@echo "  clean-release     - Clean release directory"
	@echo "  test              - Run all tests (unit, integration, property, CLI)"
	@echo "  test-unit         - Run unit tests"
	@echo "  test-integration  - Run integration tests"
	@echo "  test-property     - Run property tests"
	@echo "  test-cli          - Run CLI tests"
	@echo "  test-all          - Run all tests (including doc tests and examples)"
	@echo "  lint              - Run clippy linter"
	@echo "  format            - Format code using rustfmt"
	@echo "  check-format      - Check if code is properly formatted"
	@echo "  fix-all           - Fix all Rust issues (clippy, formatting, etc.)"
	@echo "  doc               - Generate documentation"
	@echo "  doc-open          - Generate documentation and open in browser"
	@echo "  clean             - Clean build artifacts"
	@echo "  clean-all         - Clean build artifacts and Cargo.lock"
	@echo "  run               - Run the application in debug mode"
	@echo "  run-release       - Run the application in release mode"
	@echo "  help              - Show this help message"
	@echo ""
	@echo "Examples:"
	@echo "  make release                  - Build release packages with default version ($(VERSION))"
	@echo "  VERSION=v1.0.0 make release   - Build release packages with specified version"
	@echo "  make version                  - Print the current version"
