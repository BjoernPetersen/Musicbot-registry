FROM ekidd/rust-musl-builder:nightly-2020-10-08 as builder

USER root
RUN cargo new --bin musicbot-registry
WORKDIR ./musicbot-registry

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo update
RUN cargo build --release
RUN rm src/*.rs

COPY ./src/ ./src/
RUN cargo build --release

FROM alpine:latest
ARG APP=/app
ARG PORT=8000
EXPOSE ${PORT}

ENV TZ=Etc/UTC \
    APP_USER=appuser

RUN addgroup -S $APP_USER \
    && adduser -S -g $APP_USER $APP_USER

RUN apk update \
    && apk add --no-cache ca-certificates tzdata \
    && rm -rf /var/cache/apk/*

COPY --from=builder /home/rust/src/musicbot-registry/target/x86_64-unknown-linux-musl/release/musicbot-registry ${APP}/musicbot-registry
COPY ./Rocket.toml ./Rocket.toml
RUN chown -R $APP_USER:$APP_USER ${APP}

USER $APP_USER
WORKDIR ${APP}
CMD ["./musicbot-registry"]

HEALTHCHECK CMD curl -f http://localhost:$PORT/ || exit 1
