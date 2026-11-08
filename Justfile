set shell := ["zsh", "-cu"]

root := justfile_directory()

default:
    @just --list

dev:
    ./scripts/verify-production-boundary
    ./tools/builtin-agent/build debug
    ./scripts/with-dev-cache cargo build --locked --bin meerkatd --bin meerkat-machine --bin meerkat
    ./scripts/with-dev-cache cargo run --locked -p meerkat-desktop

web-dev:
    pnpm -C apps/web dev

web-types:
    ./scripts/with-dev-cache cargo run --locked -p meerkat-web-types -- apps/web/src/lib/api/generated

check:
    just web-check
    ./scripts/with-dev-cache ./scripts/verify-web-types
    ./scripts/verify-open-source
    ./scripts/verify-production-boundary
    cargo fmt --all --check
    ./scripts/with-dev-cache cargo check --workspace --all-targets --locked
    ./scripts/verify-e2e-contracts

test:
    ./scripts/with-dev-cache ./scripts/verify-all --gate hermetic

build-release:
    ./scripts/build-release

app: build-release
    ./scripts/package-macos --app-only

package: build-release
    ./scripts/package-macos

verify-package: package
    ./scripts/verify-package

release-local: check test verify-package
    ./scripts/verify-all --gate release-regressions
    @echo "Local checks and macOS distribution verification passed."

run: app
    open "{{root}}/target/release/Meerkat.app"

full-release:
    MEERKAT_E2E_LIVE=1 ./scripts/verify-all --gate full-release

release: full-release
    @echo "Release verification and distributable package are ready."

web-check:
    pnpm -C apps/web install --frozen-lockfile
    pnpm -C apps/web format:check
    pnpm -C apps/web lint
    pnpm -C apps/web stylelint
    pnpm -C apps/web typecheck
    pnpm -C apps/web build
    ./scripts/verify-web-lint

web-e2e:
    pnpm -C apps/web build
    ./scripts/with-dev-cache cargo build --locked -p meerkat-system-e2e --bins
    pnpm -C apps/web e2e
