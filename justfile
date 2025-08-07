default:
    @just --list

# run the web proxy server
web-proxy *ARGS:
    cargo run --bin hooya-web-proxy -- {{ARGS}}

# run the hooya daemon
hooyad *ARGS:
    cargo run --bin hooyad -- {{ARGS}}

# run the web ui in development mode
web-ui:
    cd ../hooya-web-ui && npm run dev

# build the web ui for production
web-ui-build:
    cd ../hooya-web-ui && npm run build

# run the web ui in production mode
web-ui-start:
    cd ../hooya-web-ui && npm run start

# build all binaries
build:
    cargo build --workspace

# build all binaries in release mode
build-release:
    cargo build --workspace --release

unittests:
    cargo test --workspace --lib --exclude hooya-itest

# build local docker images for testing
build-test-images:
    docker build --target hooyad -t hooyad:itests .
    docker build --target hooya-web-proxy -t hooya-web-proxy:itests .

itests: build-test-images
    cargo test -p hooya-itest

# check all code compiles
check:
    cargo check --workspace

# format code
fmt:
    cargo fmt --all

# clean build artifacts
clean:
    cargo clean