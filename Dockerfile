FROM rust:latest AS builder

RUN apt-get update && apt-get install -y musl-tools musl-dev

WORKDIR /app

ENV TARGET=x86_64-unknown-linux-musl
RUN rustup target add "$TARGET"

# copy all your source files ...
COPY . .

RUN cargo build --release --locked --target "$TARGET"

# and then copy it to an empty docker image
FROM scratch
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/route53_dd /bin/route53_dd
ENTRYPOINT ["/bin/route53_dd -d"]