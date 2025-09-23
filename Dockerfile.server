FROM rust:1.88.0

WORKDIR /app

ARG APP_PROFILE

ENV APP_PROFILE=${APP_PROFILE}

# copy binary from build stage
COPY target/release/server .

# copy base configuration files
COPY base.toml .

# copy profile-specific configuration file
COPY base.${APP_PROFILE}.toml .

RUN chmod +x server

CMD ["./server"]
