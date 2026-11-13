bin := "cosmicmost"

default:
    @just --list

build:
    cargo build --release

check:
    cargo check --all-targets

test:
    cargo test

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --all-targets -- -D warnings

release-check: fmt-check clippy test
    sh -n install.sh

install: build
    install -Dm755 target/release/{{bin}} "$HOME/.local/bin/{{bin}}"

uninstall:
    -systemctl --user disable --now cosmicmost.service
    rm -f "$HOME/.local/bin/{{bin}}"
    rm -f "$HOME/.config/systemd/user/cosmicmost.service"

service: install
    {{bin}} install-service

clean:
    cargo clean
