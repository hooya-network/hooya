FROM rust:1.69.0-alpine as builder
RUN apk add openssl-dev musl-dev protoc
WORKDIR /wd
COPY . /wd
RUN cargo build --bin hooyad --release

FROM scratch
COPY --from=builder /wd/target/release/hooyad /

EXPOSE 8531
CMD ["/hooyad"]
