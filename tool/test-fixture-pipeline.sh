#!/usr/bin/env bash
set -euo pipefail

GESENIUS_VALIDATE_TEI_EXTERNAL=1 cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
xmllint --noout schema/tei-lex0.rng
sqlite3 :memory: < schema/sqlite-v1.sql
