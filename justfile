export RUST_LOG := "spin-trace"

# Set the default recipe
default:
    just test
    cargo build --release

# Run linting and unit tests
test:
    just lint
    just test-unit

# Lint the codebase
lint:
    cargo clippy --all-features -- -D warnings
    cargo fmt -- --check

# Run unit tests with dynamic target
test-unit:
    RUST_LOG=${RUST_LOG} cargo test --target=`rustc -vV | sed -n 's|host: ||p'`

release:
    #!/usr/bin/env fish
    set this_version (grep '^version =' spin.toml | sed -E 's/version = "(.*)"/\1/')
    git tag v$this_version
    git push origin v$this_version
    set -e this_version

up:
    #!/usr/bin/env fish
    #for line in (cat ../../.env | grep -v '^#' | grep -v '^[[:space:]]*$')
    #    set item (string split -m 1 '=' $line)
    #    set -gx $item[1] $item[2]
    #end
    spin up --build --runtime-config-file ../../runtime-config.dev.toml
