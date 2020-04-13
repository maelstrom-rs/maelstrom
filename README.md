# Maelstrom ![](https://github.com/maelstrom-rs/maelstrom/workflows/Build/badge.svg)

<img src="./.github/logo-banner.svg">

A high-performance [Matrix](https://matrix.org) Home-Server written in [Rust](rust-lang.org) designed to have a plugable storage engine, scalable, and light on resources.

General discussion for development is at [#maelstrom-server:matrix.org](https://matrix.to/#/maelstrom-server:matrix.org)

## Project Status

This is a brand new project under **daily** active development. It is not currently in usable form yet.

### Completed Features

You can review the [Closed `matrix-spec` Issues](https://github.com/maelstrom-rs/maelstrom/issues?q=is%3Aissue+is%3Aclosed+sort%3Acreated-asc+label%3Amatrix-spec+) in the issue tracker for a list of completed features.

## Project Goals

1. Performance, both in terms of scale and minimal resources.
2. From scratch design, no legacy architecture decisions.
3. Support for embedded (Raspi, Jetson Nano, etc.) or clustered deployment with configurable storage engine (e.g. Postgres, Sqlite, Sled, etc.).
4. First-class e2e encryption and p2p support (as Matrix.org works towards a direction).
5. Designed for not only chat, but decentralized IoT use cases as well.
6. SOCKS5 Proxy support to enable .onion homeservers ([Relevant Synapse Issue](https://github.com/matrix-org/synapse/issues/7088))

## Why

This project started due to a strong interest/support of Web 3.0 (decentralized web applications). Additionally,
having a performant embeddable home server can enable a stronger usecase for decentralized IoT applications in addition to chat.

## Building & Running

### Using Rust

```bash
# install rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# clone repo and cd
git clone https://github.com/maelstrom-rs/maelstrom.git && cd maelstrom

# copy .env-example and set with your specific settings
cp .env-example .env

# build & run
cargo run --release
```

### Generating the AUTH_KEY

```bash
openssl ecparam -genkey -name secp256k1 | openssl pkcs8 -topk8 -nocrypt -out ec_private.pem
```

Make sure you set AUTH_KEY_FILE to `path/to/ec_private.pem`

## Technologies Used

- [Actix-web](https://actix.rs) A high performance webserver written in Rust
- [sqlx](https://github.com/launchbadge/sqlx) A rust version of the popular sqlx db library
- [jwt](https://jwt.io)
- [Ruma](https://github.com/ruma)

## Similar Projects

The following are some other Rust based Home Server projects worth looking at:

- [Ruma](https://github.com/ruma) The server isn't maintained, but he client libraries appear so.
- [Condui](https://git.koesters.xyz/timo/conduit) A new Rust based Home Server under development.

## License

Licensed under either of [Apache License](LICENSE-APACHE), Version
2.0 or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Maelstrom by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
