# Maelstrom ![](https://github.com/maelstrom-rs/maelstrom/workflows/Build/badge.svg)

A high-performance [Matrix](https://matrix.org) Home-Server designed to have a plugable storage engine, scalable, and light on resources.

## Project Status

This is a brand new project under **daily** active development. It is not currently in usable form yet.

## Project Goals

1. Performance, both in terms of scale and minimal resources.
2. From scratch design, no legacy architecture decisions.
3. Support for embedded or clustered deployment (configurable storage engine, e.g. Postgres, Sled, TiKV).
4. First-class e2e encryption and p2p support (as Matrix.org works towards a direction).
5. Designed for not only chat, but decentralized IoT use cases as well.

## Why

This project started due to a strong interest/support of Web 3.0 (decentralized web applications). Additionally,
having a performant embeddable home server can enable a stronger usecase for decentralized IoT applications in addition to chat.

## Technologies Used

- [Actix-web](https://actix.rs) A high performance webserver written in Rust
- [sqlx](https://github.com/launchbadge/sqlx) A rust version of the popular sqlx db library

## Similar Projects

The following are some other Rust based Home Server projects worth looking at:

- [Ruma](https://github.com/ruma) The server isn't maintained, but he client libraries appear so.
- [Matrixserver](https://git.koesters.xyz/timo/matrixserver) A new Rust based Home Server under development.

## License

Licensed under either of [Apache License](LICENSE-APACHE), Version
2.0 or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Maelstrom by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
