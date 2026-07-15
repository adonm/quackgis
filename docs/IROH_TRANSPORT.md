# Iroh transport

The active I0 implementation starts in `crates/quackgis-edge`. It is one shared
protocol boundary for the bootstrap, tiny client, and complete worker rather than
three independently evolving wire formats.

## Implemented foundation

- `quackgis/control/1` and `quackgis/edge/1` are fixed ALPN identifiers.
- A bootstrap-signed access lease binds one credential public key, LOGIN role,
  worker endpoint/address, assignment generation, epochs, permitted protocols,
  and a lifetime of at most five minutes.
- Lease refresh proves the registered credential key and binds the request to the
  caller's current iroh transport endpoint and the selected bootstrap.
- Worker authentication adds a fresh challenge. The credential proof covers the
  signed lease, worker challenge, client transport endpoint, and compression
  offers, preventing a copied lease from becoming a bearer credential.
- Control messages are bounded at 16 KiB and reject unknown fields. Roles,
  protocol lists, time ranges, assignments, signatures, proofs, and compression
  negotiation fail closed.
- `none` is the only implemented compression codec and is mandatory. A candidate
  codec will be added only after the I0 direct/relay measurements select one.
- Omitted relay configuration selects iroh's public production preset. An
  explicitly empty, malformed, or duplicate custom list is rejected.

Run this evidence with `just iroh-protocol-test`.

This is protocol and cryptographic evidence, not yet an application transport
claim. The executable bootstrap, worker stream bridge, tiny local client, direct
and relayed profiles, and compression measurements remain open and are tracked in
[ROADMAP_STATUS.md](./ROADMAP_STATUS.md).

## Security boundary

The credential key is distinct from the ephemeral iroh endpoint key. Bootstrap
signs assignment but never proxies application bytes. A worker must verify the
bootstrap signature, lease lifetime/assignment, current transport endpoint, and
fresh credential proof before accepting a typed pgwire, cancellation, or future
HTTP stream. Neither relay access nor possession of a copied signed lease grants
a database role.
