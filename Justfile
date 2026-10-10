set shell := ["zsh", "-cu"]

root := justfile_directory()

default:
    @just --list

dev:
    ./scripts/verify-production-boundary
    cargo build --locked --bin lankide-runtime-daemon --bin lankide-computer-daemon --bin lankide
    cargo run --locked -p lankide-desktop

check:
    ./scripts/verify-open-source
    ./scripts/verify-production-boundary
    cargo fmt --all --check
    cargo check --workspace --all-targets --locked
    ./scripts/verify-e2e-contracts
    ./scripts/verify-desktop-ui-boundary

test:
    ./scripts/verify-all --gate hermetic

build-release:
    ./scripts/build-release

app: build-release
    ./scripts/package-macos --app-only

package: build-release
    ./scripts/package-macos

verify-package: package
    ./scripts/verify-package

release-local: check test verify-package
    @echo "Local checks and macOS distribution verification passed."

run: app
    open "{{root}}/target/release/Lankide.app"

full-release:
    LANKIDE_E2E_LIVE=1 ./scripts/verify-all --gate full-release

release: full-release
    @echo "Release verification and distributable package are ready."
