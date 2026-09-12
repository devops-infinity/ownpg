FROM rust:1.97.1-trixie@sha256:b1b3c9c0d921d7fa0a6d1f9ec7e4eab87f8c8ec97644c3d791450f131dec813f AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --profile dist --locked -p ownpg \
    && install -m 0755 target/dist/ownpg /ownpg

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:c31ff9abcb1910f3ab25c7957bdaf0bfe12a01eb546e8df2282f1c8f682b606c
LABEL org.opencontainers.image.title="OwnPG" \
      org.opencontainers.image.description="OwnPG MCP server for PostgreSQL in remote mode" \
      org.opencontainers.image.source="https://github.com/devops-infinity/ownpg-releases" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=build /ownpg /usr/local/bin/ownpg
ENV OWNPG_BIND=0.0.0.0:8765 \
    OWNPG_NO_INPUT=true \
    XDG_CONFIG_HOME=/var/lib/ownpg/config \
    XDG_DATA_HOME=/var/lib/ownpg/data \
    XDG_CACHE_HOME=/var/lib/ownpg/cache
USER 65532:65532
WORKDIR /var/lib/ownpg
EXPOSE 8765
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
  CMD ["/usr/local/bin/ownpg", "doctor", "--format", "json", "--quiet"]
ENTRYPOINT ["/usr/local/bin/ownpg"]
CMD ["serve", "--http"]
