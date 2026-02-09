---
title: Initial linkedin-ads CLI + release pipeline
status: active
priority: 0
issue-type: task
created-at: "\"2026-02-09T20:46:52.440816+07:00\""
---

Scaffold Rust CLI like meta-ads-cli: discovery commands, Rest.li client (headers, query tunneling, x-restli-id), asset upload helpers, release GH action (macos-15 aarch64 + linux x86_64), install script, README (install+env+token steps). AC: cargo build ok; workflow builds tar.gz assets; install.sh works; README covers env+token.
