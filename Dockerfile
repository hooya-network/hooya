FROM rust:1.73.0-alpine as builder
RUN apk add openssl-dev musl-dev protoc
WORKDIR /wd
COPY . /wd

ENV OPENSSL_DIR=/usr
RUN cargo build --bin hooyad --bin hooya-web-proxy --release

FROM scratch as hooyad
COPY --from=builder /wd/target/release/hooyad /

EXPOSE 8531
CMD ["/hooyad"]

FROM scratch as hooya-web-proxy
COPY --from=builder /wd/target/release/hooya-web-proxy /

EXPOSE 8532
CMD ["/hooya-web-proxy"]
