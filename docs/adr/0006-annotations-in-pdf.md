# ADR 0006: Annotations saved inside the PDF

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D6 (plan §3)

## Context

Users move files between apps and devices; annotations kept outside the file get lost.

## Decision

Save annotations inside the PDF as standard annotations, with incremental save. Use a sidecar only for non-PDF formats and app-only data.

## Why

Interoperability: other readers see the user's annotations.

## Consequences

Saving must be atomic and incremental (K.15). App-only data, such as anchors, goes in private keys or the sidecar.
