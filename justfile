default:
    @just --list

all: check test lint format
    @echo "✅ All checks passed!"

check:
    @echo "🔍 Running cargo check..."
    cargo check --all-features

test:
    @echo "🧪 Running tests..."
    cargo test --all-features

lint:
    @echo "📋 Running clippy..."
    cargo clippy --all-targets --all-features -- -D warnings

format:
    @echo "🎨 Running formatter..."
    cargo fmt

format-check:
    @echo "🎨 Checking format..."
    cargo fmt -- --check

audit:
    @echo "🔒 Running security audit..."
    cargo audit

ci: check test lint format-check audit
    @echo "✅ CI checks complete!"

fix:
    cargo fmt
    cargo clippy --fix --allow-dirty --allow-staged
    @echo "🔧 Auto-fixes applied!"

clean:
    cargo clean
