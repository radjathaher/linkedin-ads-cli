---
title: Initial linkedin-ads CLI + release pipeline
status: closed
priority: 0
issue-type: task
created-at: "\"\\\"2026-02-09T20:46:52.440816+07:00\\\"\""
closed-at: "2026-02-09T21:17:48.504711+07:00"
close-reason: "Implemented Rust CLI + command tree, Rest.li client (headers+tunneling+x-restli-id), assets image/video upload helpers, GH Actions release (macos-15 aarch64 + linux x86_64), install script + README. Validated: cargo build/release ok; GH release v0.1.0 w/ tar.gz assets."
---

Scaffold Rust CLI like meta-ads-cli: discovery commands, Rest.li client (headers, query tunneling, x-restli-id), asset upload helpers, release GH action (macos-15 aarch64 + linux x86_64), install script, README (install+env+token steps). AC: cargo build ok; workflow builds tar.gz assets; install.sh works; README covers env+token.
