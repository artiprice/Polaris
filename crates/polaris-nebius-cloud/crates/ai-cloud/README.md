# Nebius AI Cloud policy core

This crate currently implements local policy only: exact integer budget estimates and
reservations, VM lifetime and ingress validation, and exact resource ownership
filtering. It does not make provider calls, provision VMs, schedule shutdowns, or
produce gateway credentials/endpoints.

## Verified provider integration route

Keyless research of the official Nebius AI Cloud documentation on 2026-10-26 found:

- The [REST API overview](https://docs.nebius.com/rest-api) says its OpenAPI spec
  describes an HTTP/JSON API and that REST is generated from the same Protocol Buffer
  service definitions as gRPC.
- The [REST quickstart](https://docs.nebius.com/rest-api/quickstart) demonstrates
  access-token authentication and a REST request. The
  [provider-owned API repository](https://github.com/nebius/api) publishes the gRPC
  `.proto` definitions and describes the REST/gRPC relationship and bearer auth.
- Nebius documentation lists SDKs for Go, Python, and JavaScript. The documented
  `compute v1alpha1 instance` CLI command family is marked deprecated.

**Integration decision:** use the official REST API/OpenAPI contract for a future
Rust transport adapter, generating/binding request and response types from the
published contract instead of hand-writing provider paths or payloads. This decision
identifies the documented protocol surface; it does not verify a Rust SDK, generate a
client, establish IAM credential storage, or prove lifecycle endpoint semantics for a
specific project/region. Inspect the current OpenAPI reference and authentication
model in the adapter phase before adding dependencies or provider calls.

No paid API calls or cloud resources were used for this research.
