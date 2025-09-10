set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

web-proxy *ARGS:
    cargo run --bin hooya-web-proxy -- {{ARGS}}

hooyad *ARGS:
    cargo run --bin hooyad -- {{ARGS}}

build:
    cargo build --workspace

build-release:
    cargo build --workspace --release

unittests:
    cargo test --workspace --lib --exclude hooya-itest

build-test-images:
    docker build --target hooyad -t hooyad:itests .
    docker build --target hooya-web-proxy -t hooya-web-proxy:itests .

itests: build-test-images
    -k3d cluster delete hooya-itest
    -docker stop $(docker ps -q --filter "name=hooya-test-postgres") && docker rm $(docker ps -aq --filter "name=hooya-test-postgres")
    cargo test -p hooya-itest
    -k3d cluster delete hooya-itest
    -docker stop $(docker ps -q --filter "name=hooya-test-postgres") && docker rm $(docker ps -aq --filter "name=hooya-test-postgres")

check:
    cargo check --workspace

fmt:
    cargo fmt --all

clean:
    cargo clean

# postgres with a random password
pg:
	#!/usr/bin/env bash
	if ! docker volume inspect app-pgdata >/dev/null 2>&1; then
		PG_PW="$(openssl rand -hex 16)"
		POSTGRES_PASSWORD="$PG_PW" docker compose -f docker-compose.postgres.yml up -d pg
		echo "postgres://hooya:${PG_PW}@localhost:5433/hooya?sslmode=disable"
	else
		POSTGRES_PASSWORD=whatever docker compose -f docker-compose.postgres.yml up -d pg
	fi

pg-down:
	POSTGRES_PASSWORD=whatever docker compose -f docker-compose.postgres.yml down

pg-reset:
	POSTGRES_PASSWORD=whatever docker compose -f docker-compose.postgres.yml down -v || true
	docker volume rm -f app-pgdata || true