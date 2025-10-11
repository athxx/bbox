#!/usr/bin/env just --justfile

set shell := ["bash", "-c"]

# Run Tests
test:
    cargo test --all

# Run Test DB
mvtbenchdb:
    docker run -p 127.0.0.1:5439:5432 -d --name mvtbenchdb --rm sourcepole/mvtbenchdb:v1.2

# Run DB Tests
test-db:
    cargo test --all -- --ignored

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Publish to crates.io
publish:
    cd bbox-core && cargo publish
    cd bbox-feature-server && cargo publish
    cd bbox-map-server && cargo publish
    cd bbox-asset-server && cargo publish
    cd bbox-tile-server && cargo publish
    cd bbox-processes-server && cargo publish
    # cd bbox-routing-server && cargo publish
    cd bbox-frontend && cargo publish
    cd bbox-server && cargo publish

docker-build svc="bbox-tile-server":
    nice docker build --build-arg BUILD_DIR={{svc}} -f docker/Dockerfile -t {{svc}} .

docker-build-processes:
    nice docker build -f docker/Dockerfile-processes -t sourcepole/bbox-processes-server .

# Test recipe for processes server
[group('processes')]
hello *args="world":
    @echo hello {{args}}

# Test recipe for processes server
[group('processes')]
sleep count="1":
    @for i in {1..{{count}}}; do echo Sleep $i; sleep 1; done

[group('processes')]
upload fname tmpname *args:
    ln -s {{tmpname}} $(dirname {{tmpname}})/{{fname}}; ls -l $(dirname {{tmpname}})/{{fname}}
